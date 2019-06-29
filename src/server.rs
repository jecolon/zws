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
use solicit::http::HttpScheme;

use crate::error::Result;
use crate::handlers::{Handler, NotFound};
use crate::request::{Action, ServerRequest};
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
    pub fn new(cert: &String, key: &String, socket: &String) -> Result<Arc<Server>> {
        env_logger::from_env(Env::default().default_filter_or("info")).init();

        let srv = Server {
            acceptor: Server::new_acceptor(&cert, &key)?,
            listener: TcpListener::bind(&socket)?,
            router: HashMap::<Action, Box<Handler>, BuildHasher>::default(),
            not_found: Box::new(NotFound {}),
        };

        println!("zws HTTP server listening on {}. CTRL+C to stop.", &socket);
        info!("Using certificate: {}, and key: {}.", &cert, &key);

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
    pub fn add_handler(self: Arc<Self>, action: Action, handler: Box<Handler>) -> Arc<Self> {
        let mut srv = match Arc::try_unwrap(self) {
            Ok(srv) => srv,
            Err(_) => panic!("unalble to move out of Arc"),
        };
        srv.router.insert(action, handler);
        Arc::new(srv)
    }

    /// handler returns a handler for a given Action, or file_handler if none found.
    fn handler(&self, action: &Action) -> &Box<Handler> {
        if let Some(h) = self.router.get(&action) {
            return h;
        }

        let mut path = match action {
            Action::GET(path) => PathBuf::from(path),
            _ => unimplemented!(),
        };

        while path.pop() {
            let action = Action::GET(PathBuf::from(&path));
            if let Some(h) = self.router.get(&action) {
                return h;
            }
        }

        &self.not_found
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
                debug!("handle_stream: received request: {:?}", req.action);
                responses.push(srv.handler(&req.action).handle(req));
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
