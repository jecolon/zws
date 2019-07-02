use std::collections::HashMap;
use std::hash::BuildHasherDefault;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
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
use crate::handlers::{Handler, NotFound};
use crate::request::{Action, Request};
use crate::response::Response;
use crate::tls::Wrapper;

/// BuildHasher lets us use SeaHasher with HashMap.
type BuildHasher = BuildHasherDefault<SeaHasher>;

/// Server is a simple HTT/2 server
pub struct Server {
    acceptor: Arc<SslAcceptor>,
    listener: TcpListener,
    router: HashMap<Action, Box<Handler>, BuildHasher>,
    not_found: Box<Handler>,
}

impl Server {
    /// new returns an initialized instance of Server
    pub fn new(cert: &str, key: &str, socket: &str) -> Result<Server> {
        env_logger::from_env(Env::default().default_filter_or("info")).init();

        println!("zws HTTP server listening on {}. CTRL+C to stop.", socket);
        info!("Using certificate: {}, and key: {}.", cert, key);

        Ok(Server {
            acceptor: Server::new_acceptor(cert, key)?,
            listener: TcpListener::bind(socket)?,
            router: HashMap::<Action, Box<Handler>, BuildHasher>::default(),
            not_found: Box::new(NotFound {}),
        })
    }

    /// add_handler registers a handler for a given Action.
    pub fn add_handler(mut self, action: &str, handler: Box<Handler>) -> Result<Self> {
        self.router.insert(action.parse()?, handler);
        Ok(self)
    }

    // run does setup and takes an incoming TLS connection and sends its stream to be handled.
    pub fn run(self) -> Result<()> {
        let srv = Arc::new(self);
        for stream in srv.listener.incoming() {
            match stream {
                Ok(stream) => {
                    let clone = Arc::clone(&srv);
                    thread::spawn(move || clone.handle_stream(stream));
                }
                Err(e) => {
                    warn!("error in TCP accept: {}", e);
                }
            }
        }
        Ok(())
    }

    /// handler returns a handler for a given Action, or file_handler if none found.
    fn handler(&self, action: &Action) -> &Box<Handler> {
        if let Some(h) = self.router.get(&action) {
            return h.clone();
        }

        let mut path = match action {
            Action::GET(path) => PathBuf::from(path),
        };

        while path.pop() {
            let action = Action::GET(path.to_string_lossy().to_string());
            if let Some(h) = self.router.get(&action) {
                return h.clone();
            }
        }

        &self.not_found
    }

    /// new_acceptor creates a new TLS acceptor with the given certificate and key.
    fn new_acceptor(cert: &str, key: &str) -> Result<Arc<SslAcceptor>> {
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

        Ok(Arc::new(acceptor.build()))
    }

    /// handle_stream processess an HTTP/2 TCP/TLS streaml
    fn handle_stream(&self, stream: TcpStream) {
        let acceptor = Arc::clone(&self.acceptor);
        let stream = match acceptor.accept(stream) {
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
            error!("error binding to TCP socket: {}", e);
            return;
        };

        while let Ok(_) = conn.handle_next_frame() {
            let mut responses = Vec::new();
            for stream in conn.state.iter() {
                if stream.is_closed_remote() {
                    let req = match Request::new(&stream) {
                        Ok(req) => req,
                        Err(e) => {
                            warn!("error processing request: {}", e);
                            let mut resp = Response::new(stream.stream_id);
                            resp.header(":status", "400");
                            resp.body("Bad Request\n");
                            responses.push(resp);
                            continue;
                        }
                    };
                    debug!("handle_stream: received request: {:?}", req.action);
                    let resp = Response::new(stream.stream_id);
                    responses.push(self.handler(&req.action).handle(req, resp));
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
