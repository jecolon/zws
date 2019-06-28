use std::collections::HashMap;
use std::hash::BuildHasherDefault;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::str;
use std::sync::{Arc, Mutex};
use std::thread;

use env_logger::Env;
use openssl::ssl::{AlpnError, SslAcceptor, SslFiletype, SslMethod};
use seahash::SeaHasher;
use serde::Deserialize;
use solicit::http::connection::{EndStream, HttpConnection, SendStatus};
use solicit::http::server::ServerConnection;
use solicit::http::session::{DefaultSessionState, SessionState, Stream};
use solicit::http::transport::TransportStream;
use solicit::http::HttpScheme;

use crate::error::Result;
use crate::handlers::{file_handler, Handler};
use crate::mcache::Cache;
use crate::request::{Action, ServerRequest};
use crate::tls::Wrapper;

/// ServerBuilder builds a Server providing sensible defaults.
#[derive(Deserialize)]
pub struct ServerBuilder {
    pub flag_nocache: bool,
    pub flag_cert: String,
    pub flag_key: String,
    pub flag_socket: String,
    pub flag_webroot: String,
    #[serde(skip)]
    handlers: HashMap<Action, Handler, BuildHasher>,
}

impl ServerBuilder {
    pub fn new() -> ServerBuilder {
        ServerBuilder {
            flag_nocache: false,
            flag_cert: "tls/dev/cert.pem".to_string(),
            flag_key: "tls/dev/key.pem".to_string(),
            flag_socket: "127.0.0.1:8443".to_string(),
            flag_webroot: "webroot".to_string(),
            handlers: HashMap::<Action, Handler, BuildHasher>::default(),
        }
    }

    pub fn without_cache(&mut self) -> &mut Self {
        self.flag_nocache = true;
        self
    }

    pub fn cert(&mut self, cert: &str) -> &mut Self {
        self.flag_cert = cert.to_string();
        self
    }

    pub fn key(&mut self, key: &str) -> &mut Self {
        self.flag_key = key.to_string();
        self
    }

    pub fn socket(&mut self, socket: &str) -> &mut Self {
        self.flag_socket = socket.to_string();
        self
    }

    pub fn webroot(&mut self, webroot: &str) -> &mut Self {
        self.flag_webroot = webroot.to_string();
        self
    }

    pub fn handler(&mut self, action: Action, handler: Handler) -> &mut Self {
        self.handlers.insert(action, handler);
        self
    }

    pub fn build(&self) -> Result<Arc<Server>> {
        let srv = Server::new(
            self.flag_nocache,
            &self.flag_cert,
            &self.flag_key,
            &self.flag_socket,
            &self.flag_webroot,
        )?;

        let mut srv = match Arc::try_unwrap(srv) {
            Ok(srv) => srv,
            Err(_) => {
                panic!("unable to build Server");
            }
        };

        srv.router.clone_from(&self.handlers);

        Ok(Arc::new(srv))
    }
}

/// BuildHasher lets us use SeaHasher with HashMap.
type BuildHasher = BuildHasherDefault<SeaHasher>;

/// Server is a simple HTT/2 server
pub struct Server {
    acceptor: Arc<SslAcceptor>,
    listener: TcpListener,
    pub cache: Option<Arc<Cache>>,
    pub webroot: PathBuf,
    router: HashMap<Action, Handler, BuildHasher>,
}

impl Server {
    /// new returns an initialized instance of Server
    pub fn new(
        nocache: bool,
        cert: &String,
        key: &String,
        socket: &String,
        webroot: &String,
    ) -> Result<Arc<Server>> {
        env_logger::from_env(Env::default().default_filter_or("info")).init();

        let mut srv = Server {
            acceptor: Server::new_acceptor(&cert, &key)?,
            listener: TcpListener::bind(&socket)?,
            cache: None,
            webroot: PathBuf::from(&webroot).canonicalize()?,
            router: HashMap::<Action, Handler, BuildHasher>::default(),
        };

        println!("zws HTTP server listening on {}. CTRL+C to stop.", &socket);
        info!("Serving files in {}", &webroot);
        info!("Using certificate: {}, and key: {}.", &cert, &key);
        if !nocache {
            srv.cache = Some(Cache::new(srv.webroot.clone()));
            info!("Response caching enabled.");
        }

        Ok(Arc::new(srv))
    }

    // run does setup and takes an incoming TLS connection and sends its stream to be handled.
    pub fn run(self: Arc<Self>) -> Result<()> {
        for stream in self.listener.incoming() {
            match stream {
                Ok(stream) => {
                    let srv = Arc::clone(&self);
                    thread::spawn(move || handle_stream(stream, srv));
                }
                Err(e) => {
                    warn!("error in TCP accept: {}", e);
                }
            }
        }
        Ok(())
    }

    /// add_handler registers a handler for a given Action.
    pub fn add_handler(&mut self, action: Action, handler: Handler) {
        self.router.insert(action, handler);
    }

    /// handler returns a handler for a given Action, or file_handler if none found.
    fn handler(&self, action: &Action) -> Handler {
        if let Some(h) = self.router.get(&action) {
            return *h;
        }
        return file_handler;
    }

    /// new_acceptor creates a new TLS acceptor with the given certificate and key.
    fn new_acceptor(cert: &String, key: &String) -> Result<Arc<SslAcceptor>> {
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
}

/// handle_stream processess an HTTP/2 TCP/TLS streaml
fn handle_stream(stream: TcpStream, srv: Arc<Server>) {
    let acceptor = Arc::clone(&srv.acceptor);
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
                let req = match ServerRequest::new(&stream) {
                    Ok(req) => req,
                    Err(e) => {
                        warn!("error processing request: {}", e);
                        return;
                    }
                };
                debug!("received request: {:?}", req.action);
                let srv = Arc::clone(&srv);
                responses.push(srv.handler(&req.action)(req, srv));
            }
        }

        for response in responses {
            if let Err(e) = conn.start_response(response.headers, response.stream_id, EndStream::No)
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
