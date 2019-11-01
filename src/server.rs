use std::collections::HashMap;
use std::hash::BuildHasherDefault;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use env_logger::Env;
use openssl::ssl::{AlpnError, SslAcceptor, SslFiletype, SslMethod};
use seahash::SeaHasher;
use solicit::http::connection::{EndStream, HttpConnection, SendStatus};
use solicit::http::server::ServerConnection;
use solicit::http::session::{DefaultSessionState, SessionState, Stream};
use solicit::http::transport::TransportStream;
use solicit::http::{self, HttpScheme};

use crate::error::Result;
use crate::handlers::{Handler, HandlerFunc, NotFound};
use crate::request::{Action, Request};
use crate::response::Response;
use crate::tls::Wrapper;
use crate::workers;

/// BuildHasher lets us use SeaHasher with HashMap.
type BuildHasher = BuildHasherDefault<SeaHasher>;

/// Builder is the Server builder.
pub struct Builder {
    cert: String,
    key: String,
    router: HashMap<Action, Box<dyn Handler>, BuildHasher>,
    socket: String,
    threads: usize,
}

impl Builder {
    /// new returns an initialized Server Builder.
    pub fn new() -> Builder {
        Builder {
            cert: "tls/dev/cert.pem".to_string(),
            key: "tls/dev/key.pem".to_string(),
            router: HashMap::<Action, Box<dyn Handler>, BuildHasher>::default(),
            socket: "127.0.0.1:8443".to_string(),
            threads: 0,
        }
    }

    /// tls sets the certificate and key files.
    pub fn tls(mut self, cert: &str, key: &str) -> Self {
        self.cert = cert.to_string();
        self.key = key.to_string();
        self
    }

    /// socket sets the TcP socket to listen on.
    pub fn socket(mut self, socket: &str) -> Self {
        self.socket = socket.to_string();
        self
    }

    /// threads sets the number of worker pool threads.
    pub fn threads(mut self, threads: usize) -> Self {
        self.threads = threads;
        self
    }

    /// handler registers a handler for a given Action.
    pub fn handler<H: Handler>(mut self, action: &str, handler: H) -> Result<Self> {
        self.router.insert(action.parse()?, Box::new(handler));
        Ok(self)
    }

    /// handler_func registers a closure as a handler for a given Action.
    pub fn handler_func<F>(mut self, action: &str, func: F) -> Result<Self>
    where
        F: FnOnce(Request, Response) -> Response,
        F: Clone + Send + Sync + 'static,
    {
        self.router
            .insert(action.parse()?, Box::new(HandlerFunc::new(func)));
        Ok(self)
    }

    pub fn build(self) -> Result<Server> {
        let mut server = Server::new(&self.cert, &self.key, &self.socket, self.threads)?;
        for (key, value) in self.router {
            server.router.insert(key, value);
        }
        Ok(server)
    }
}

enum Event {
    Incoming(TcpStream),
    Shutdown,
}

/// Server is a simple HTT/2 server
pub struct Server {
    acceptor: SslAcceptor,
    listener: TcpListener,
    router: HashMap<Action, Box<dyn Handler>, BuildHasher>,
    not_found: Box<dyn Handler>,
    threads: usize,
}

impl Server {
    /// builder prepares a Server Builder.
    pub fn builder() -> Builder {
        Builder::new()
    }

    /// new returns an initialized instance of Server
    pub fn new(cert: &str, key: &str, socket: &str, threads: usize) -> Result<Server> {
        env_logger::from_env(Env::default().default_filter_or("info")).init();

        println!("zws HTTP server listening on {}. CTRL+C to stop.", socket);
        info!("Using certificate: {}, and key: {}.", cert, key);
        info!(
            "Using {} threads for worker pool request handling.",
            threads
        );

        Ok(Server {
            acceptor: Server::new_acceptor(cert, key)?,
            listener: TcpListener::bind(socket)?,
            router: HashMap::<Action, Box<dyn Handler>, BuildHasher>::default(),
            not_found: Box::new(NotFound {}),
            threads,
        })
    }

    /// add_handler registers a handler for a given Action.
    pub fn add_handler<H: Handler>(mut self, action: &str, handler: H) -> Result<Self> {
        if let Some(_) = self.router.insert(action.parse()?, Box::new(handler)) {
            warn!("add_handler: overwriting handler for action: {}", action);
        }
        Ok(self)
    }

    /// add_handler_func registers a closure as a handler for a given Action.
    pub fn add_handler_func<F>(mut self, action: &str, func: F) -> Result<Self>
    where
        F: FnOnce(Request, Response) -> Response,
        F: Clone + Send + Sync + 'static,
    {
        if let Some(_) = self
            .router
            .insert(action.parse()?, Box::new(HandlerFunc::new(func)))
        {
            warn!(
                "add_handler_func: overwriting handler func for action: {}",
                action
            );
        }
        Ok(self)
    }

    // run does setup and takes an incoming TLS connection and sends its stream to be handled.
    pub fn run(self) -> Result<()> {
        // Graceful shutdown via CTRL+C
        let (event_tx, event_rx) = mpsc::channel();
        let event_tx_clone_ctrlc = mpsc::Sender::clone(&event_tx);

        ctrlc::set_handler(move || {
            info!("CTRL+C received! Shutting down...");
            event_tx_clone_ctrlc.send(Event::Shutdown).unwrap();
        })
        .unwrap();

        let srv = Arc::new(self);
        let srv_clone_main = Arc::clone(&srv);
        let event_tx_clone_main = mpsc::Sender::clone(&event_tx);

        thread::spawn(move || {
            for stream in srv_clone_main.listener.incoming() {
                match stream {
                    Ok(stream) => {
                        event_tx_clone_main.send(Event::Incoming(stream)).unwrap();
                    }
                    Err(e) => {
                        warn!("error in TCP accept: {}", e);
                    }
                }
            }
        });

        let pool = workers::Pool::new(srv.threads);
        for event in event_rx {
            match event {
                Event::Incoming(stream) => {
                    let srv_clone_pool = Arc::clone(&srv);
                    pool.execute(move || srv_clone_pool.handle_stream(stream));
                }
                Event::Shutdown => break,
            }
        }

        Ok(())
    }

    /// handler returns a handler for a given Action, or file_handler if none found.
    fn handler(&self, action: &mut Action) -> &Box<dyn Handler> {
        if let Some(h) = self.router.get(&action) {
            return h.clone();
        }

        let mut path = PathBuf::from(&action.path);

        while path.pop() {
            action.path = path.to_string_lossy().to_string();
            if let Some(h) = self.router.get(&action) {
                return h.clone();
            }
        }

        &self.not_found
    }

    /// new_acceptor creates a new TLS acceptor with the given certificate and key.
    fn new_acceptor(cert: &str, key: &str) -> Result<SslAcceptor> {
        let mut acceptor = SslAcceptor::mozilla_intermediate(SslMethod::tls())?;
        acceptor.set_private_key_file(key, SslFiletype::PEM)?;
        acceptor.set_certificate_chain_file(cert)?;
        acceptor.check_private_key()?;
        acceptor.set_alpn_protos(b"\x08http/1.1\x02h2")?;
        acceptor.set_alpn_select_callback(|_, protos| {
            const H2: &[u8] = b"\x02h2";
            if protos.windows(3).any(|window| window == H2) {
                Ok(b"h2")
            } else {
                Err(AlpnError::NOACK)
            }
        });

        Ok(acceptor.build())
    }

    /// handle_stream processess an HTTP/2 TCP/TLS streaml
    fn handle_stream(&self, stream: TcpStream) {
        let stream = match self.acceptor.accept(stream) {
            Ok(stream) => stream,
            Err(e) => {
                warn!("error in TLS accept: {}", e);
                return;
            }
        };
        let mut stream = Wrapper(Arc::new(Mutex::new(stream)));

        let mut preface = [0; 24];
        if let Err(e) = TransportStream::read_exact(&mut stream, &mut preface) {
            warn!("error reading from client connection: {}", e);
            return;
        }
        if &preface != b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n" {
            warn!("error in HTTP2 preface: {:?}", &preface);
            return;
        }

        let conn = HttpConnection::<Wrapper, Wrapper>::with_stream(stream, HttpScheme::Https);
        let mut conn: ServerConnection<Wrapper, Wrapper> =
            ServerConnection::with_connection(conn, DefaultSessionState::new());
        if let Err(e) = conn.init() {
            error!("error initializing ServerConnection: {}", e);
            return;
        };

        while let Ok(_) = conn.handle_next_frame() {
            let mut responses = Vec::new();
            for stream in conn.state.iter() {
                if stream.is_closed_remote() {
                    let actions = self.router.keys().cloned().collect();
                    let mut req = match Request::new(&stream, &actions) {
                        Ok(req) => req,
                        Err(e) => {
                            warn!("error processing request: {}", e);
                            let mut resp = Response::new(stream.stream_id);
                            resp.add_header(":status", "400");
                            resp.set_body("Bad Request\n");
                            responses.push(resp);
                            continue;
                        }
                    };
                    debug!("handle_stream: received request: {}", req);
                    let resp = Response::new(stream.stream_id);
                    responses.push(self.handler(&mut req.action).handle(req, resp));
                }
            }

            for resp in responses {
                let response: http::Response = resp.into();
                if let Err(e) =
                    conn.start_response(response.headers, response.stream_id, EndStream::No)
                {
                    warn!("error starting response: {}", e);
                    return;
                }
                let stream = match conn.state.get_stream_mut(response.stream_id) {
                    Some(stream) => stream,
                    None => {
                        warn!("error getting mutable stream");
                        return;
                    }
                };
                stream.set_full_data(response.body);
            }

            loop {
                match conn.send_next_data() {
                    Ok(status) => {
                        if status != SendStatus::Sent {
                            break;
                        }
                    }
                    Err(e) => {
                        warn!("error sending next data: {}", e);
                        break;
                    }
                }
            }
            let _ = conn.state.get_closed();
        }
    }
}
