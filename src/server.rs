use std::collections::HashMap;
use std::io;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::str;
use std::sync::{Arc, Mutex};
use std::thread;

use env_logger::Env;
use openssl::ssl::{AlpnError, ShutdownResult, SslAcceptor, SslFiletype, SslMethod, SslStream};
use serde::Deserialize;
use solicit::http::connection::{EndStream, HttpConnection, SendStatus};
use solicit::http::server::ServerConnection;
use solicit::http::session::{DefaultSessionState, DefaultStream, SessionState, Stream};
use solicit::http::transport::TransportStream;
use solicit::http::{Header, HttpScheme, Response, StreamId};

use crate::error::{Result, ServerError};
use crate::mcache::{self, Cache, Entry};

/// ServerBuilder builds a Server providing sensible defaults.
#[derive(Deserialize)]
pub struct ServerBuilder {
    pub flag_nocache: bool,
    pub flag_cert: String,
    pub flag_key: String,
    pub flag_socket: String,
    pub flag_webroot: String,
}

impl ServerBuilder {
    pub fn new() -> ServerBuilder {
        ServerBuilder {
            flag_nocache: false,
            flag_cert: "tls/dev/cert.pem".to_string(),
            flag_key: "tls/dev/key.pem".to_string(),
            flag_socket: "127.0.0.1:8443".to_string(),
            flag_webroot: "webroot".to_string(),
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

    pub fn build(&self) -> Result<Arc<Server>> {
        Server::new(
            self.flag_nocache,
            &self.flag_cert,
            &self.flag_key,
            &self.flag_socket,
            &self.flag_webroot,
        )
    }
}

// Handler is a function that produces a Response for a given ServerRequest.
type Handler = fn(ServerRequest, Arc<Server>) -> Response;

/// Server is a simple HTT/2 server
pub struct Server {
    acceptor: Arc<SslAcceptor>,
    listener: TcpListener,
    cache: Option<Arc<Cache>>,
    webroot: PathBuf,
    router: HashMap<Action, Handler>,
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
            router: HashMap::new(),
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

/// file_handler processes a request for a file. It always returns a Response.
fn file_handler(req: ServerRequest, srv: Arc<Server>) -> Response {
    let mut filename = srv.webroot.clone();
    filename.push("index.html");

    for (name, value) in req.headers {
        //let name = str::from_utf8(&name).unwrap();
        if name == b":path" {
            // Site root
            if value == b"" || value == b"/" {
                break;
            }

            let mut value = match str::from_utf8(value) {
                Ok(value) => value,
                Err(e) => {
                    warn!("error decoding :path header as UTF-8: {}", e);
                    break;
                }
            };

            // Strip leading /
            if value.starts_with("/") {
                value = &value[1..];
            }
            // Remove index.html
            filename.pop();
            // Add requested path to absolute webroot path
            filename.push(value);
            // Stop processing headers.
            break;
        }
    }

    let filename = &filename.to_string_lossy();

    let mut response = match &srv.cache {
        Some(cache) => {
            let cache = Arc::clone(&cache);
            handle_cache_entry(cache.get(filename))
        }
        None => mcache::file_response(filename).0,
    };

    response.stream_id = req.stream_id;
    response
}

/// handle_cache_entry performs a cache get and unwraps the Response.
fn handle_cache_entry((entry, found): (Entry, bool)) -> Response {
    if found {
        // Cache hit
        let &(_, _, ref rwl) = &*entry;
        return rwl.read().unwrap().clone().unwrap();
    }

    // Cache miss
    let &(ref mtx, ref cnd, ref rwl) = &*entry;
    let mut guard = mtx.lock().unwrap();
    while !*guard {
        guard = cnd.wait(guard).unwrap();
    }
    rwl.read().unwrap().clone().unwrap()
}

/// Action is an HTTP method and path combination.
#[derive(Debug, PartialEq, Eq, PartialOrd, Hash)]
enum Action {
    GET(String),
}

/// ServerRequest represents a fully received request.
struct ServerRequest<'a> {
    action: Action,
    stream_id: StreamId,
    headers: &'a [Header],
    body: &'a [u8],
}

impl<'a> ServerRequest<'a> {
    fn new(stream: &DefaultStream) -> Result<ServerRequest> {
        let headers = match stream.headers.as_ref() {
            Some(h) => h,
            None => {
                warn!("error, no HTTP/2 stream headers");
                return Err(ServerError::BadRequest);
            }
        };

        let mut req = ServerRequest {
            action: Action::GET(String::new()),
            stream_id: stream.stream_id,
            headers: headers,
            body: &stream.body,
        };

        req.action = match req.header(":method") {
            Some(method) => match method {
                "GET" => {
                    let path = match req.header(":path") {
                        Some(path) => path,
                        None => {
                            warn!("error, request without :path header");
                            return Err(ServerError::BadRequest);
                        }
                    };
                    Action::GET(String::from(path))
                }
                _ => {
                    warn!("error, unsupported request method");
                    return Err(ServerError::BadRequest);
                }
            },
            None => {
                warn!("error, request without :method header");
                return Err(ServerError::BadRequest);
            }
        };

        Ok(req)
    }

    fn header(&self, name: &str) -> Option<&str> {
        for (key, value) in self.headers {
            if key == &name.as_bytes() {
                return match str::from_utf8(value) {
                    Ok(sv) => Some(sv),
                    Err(e) => {
                        warn!("error decoding header {} as UTF-8: {}", name, e);
                        None
                    }
                };
            }
        }
        None
    }
}

/// Wrapper is a newtype to implement solicit's TransportStream for an SslStream<TcpStream>.
struct Wrapper(Arc<Mutex<SslStream<TcpStream>>>);

// io::Write for Wrapper
impl io::Write for Wrapper {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.lock().unwrap().flush()
    }
}

// io::Read for Wrapper
impl io::Read for Wrapper {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.lock().unwrap().read(buf)
    }
}

// solicit::http::transport::TransportStream
impl TransportStream for Wrapper {
    fn try_split(&self) -> io::Result<Wrapper> {
        Ok(Wrapper(self.0.clone()))
    }

    fn close(&mut self) -> io::Result<()> {
        loop {
            match self.0.lock().unwrap().shutdown() {
                Ok(ShutdownResult::Received) => return Ok(()),
                Err(e) => return Err(io::Error::new(io::ErrorKind::Other, e)),
                _ => continue,
            }
        }
    }
}
