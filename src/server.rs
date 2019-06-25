use std::io;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::str;
use std::sync::{Arc, Mutex};
use std::thread;

use env_logger;
use openssl::ssl::{AlpnError, ShutdownResult, SslAcceptor, SslFiletype, SslMethod, SslStream};
use solicit::http::connection::{EndStream, HttpConnection, SendStatus};
use solicit::http::server::ServerConnection;
use solicit::http::session::{DefaultSessionState, SessionState, Stream};
use solicit::http::transport::TransportStream;
use solicit::http::{Header, HttpScheme, Response, StreamId};

use crate::error::Result;
use crate::mcache::{self, Cache, Entry};

/// Server is a simple HTT/2 server
pub struct Server {
    acceptor: Arc<SslAcceptor>,
    listener: TcpListener,
    cache: Option<Arc<Cache>>,
    webroot: PathBuf,
}

impl Server {
    /// new returns an initialized instance of Server
    pub fn new(
        webroot: &str,
        cert: &str,
        key: &str,
        socket: &str,
        caching: bool,
    ) -> Result<Arc<Server>> {
        env_logger::init();

        let mut srv = Server {
            acceptor: Server::new_acceptor(cert, key)?,
            listener: TcpListener::bind(socket)?,
            cache: None,
            webroot: PathBuf::from(webroot).canonicalize()?,
        };

        if caching {
            srv.cache = Some(Cache::new(srv.webroot.clone()));
        }

        Ok(Arc::new(srv))
    }

    // run does setup and takes an incoming TLS connection and sends its stream to be handled.
    pub fn run(self: Arc<Self>) -> Result<()> {
        println!("zws HTTP server listening on 127.0.0.1:8443. CTRL+C to stop.");
        if self.cache.is_some() {
            info!("Response caching enabled.");
        }

        for stream in self.listener.incoming() {
            match stream {
                Ok(stream) => {
                    let srv = Arc::clone(&self);
                    thread::spawn(move || handle_stream(stream, srv));
                }
                Err(e) => {
                    eprintln!("error in TCP accept: {}", e);
                }
            }
        }
        Ok(())
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
}

/// handle_stream processess an HTTP/2 TCP/TLS streaml
fn handle_stream(stream: TcpStream, srv: Arc<Server>) {
    let acceptor = Arc::clone(&srv.acceptor);
    let stream = match acceptor.accept(stream) {
        Ok(stream) => stream,
        Err(e) => {
            eprintln!("error in TLS accept: {}", e);
            return;
        }
    };
    let mut stream = Wrapper(Arc::new(Mutex::new(stream)));

    let mut preface = [0; 24];
    if let Err(e) = TransportStream::read_exact(&mut stream, &mut preface) {
        eprintln!("error reading from client connection: {}", e);
        return;
    }
    if &preface != b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n" {
        eprintln!("error in HTTP2 preface: {:?}", &preface);
        return;
    }

    let conn = HttpConnection::<Wrapper, Wrapper>::with_stream(stream, HttpScheme::Https);
    let mut conn: ServerConnection<Wrapper, Wrapper> =
        ServerConnection::with_connection(conn, DefaultSessionState::new());
    if let Err(e) = conn.init() {
        eprintln!("error binding to TCP socket: {}", e);
        return;
    };

    while let Ok(_) = conn.handle_next_frame() {
        let mut responses = Vec::new();
        for stream in conn.state.iter() {
            if stream.is_closed_remote() {
                let h = match stream.headers.as_ref() {
                    Some(h) => h,
                    None => {
                        eprintln!("error, no HTTP/2 stream headers");
                        return;
                    }
                };
                let req = ServerRequest {
                    stream_id: stream.stream_id,
                    headers: h,
                    body: &stream.body,
                };
                let srv = Arc::clone(&srv);
                responses.push(handle_request(req, srv));
            }
        }

        for response in responses {
            if let Err(e) = conn.start_response(response.headers, response.stream_id, EndStream::No)
            {
                eprintln!("error starting response: {}", e);
                return;
            }
            let stream = match conn.state.get_stream_mut(response.stream_id) {
                Some(stream) => stream,
                None => {
                    eprintln!("error getting mutable stream");
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
                    eprintln!("error sending next data: {}", e);
                    break;
                }
            }
        }
        let _ = conn.state.get_closed();
    }
}

/// handle_request processes an HTTP/2 request. It always returns a Response.
fn handle_request(req: ServerRequest, srv: Arc<Server>) -> Response {
    let mut filename = srv.webroot.clone();
    filename.push("index.html");

    for (name, value) in req.headers {
        let name = str::from_utf8(&name).unwrap();
        let mut value = str::from_utf8(&value).unwrap();
        if name == ":path" {
            // Site root
            if value == "" || value == "/" {
                break;
            }
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

    // TODO: implement optional caching.
    let mut response: Response;
    let filename = &filename.to_string_lossy();

    if let Some(cache) = &srv.cache {
        let cache = Arc::clone(&cache);
        response = handle_cache_entry(cache.get(filename));
    } else {
        let (r, _) = mcache::file_response(filename);
        response = r
    }

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

/// ServerRequest represents a fully received request.
struct ServerRequest<'a> {
    stream_id: StreamId,
    headers: &'a [Header],
    body: &'a [u8],
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
