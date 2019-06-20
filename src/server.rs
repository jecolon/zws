use std::io;
use std::net::{TcpListener, TcpStream};
use std::str;
use std::sync::{Arc, Mutex};
use std::thread;

use openssl::ssl::{AlpnError, Error, SslAcceptor, SslFiletype, SslMethod, SslStream};
use solicit::http::connection::{EndStream, HttpConnection, SendStatus};
use solicit::http::server::ServerConnection;
use solicit::http::session::{DefaultSessionState, SessionState, Stream};
use solicit::http::transport::TransportStream;
use solicit::http::{Header, HttpScheme, Response, StreamId};

//use crate::cache;
use mcache::{Cache, Entry};

/// Server is a simple HTT/2 server
pub struct Server {
    acceptor: Arc<SslAcceptor>,
    listener: TcpListener,
    cache: Arc<Cache>,
}

impl Server {
    /// new returns an initialized instance of Server
    pub fn new(cert: &str, key: &str, socket: &str) -> Result<Server, Box<std::error::Error>> {
        Ok(Server {
            acceptor: Server::new_acceptor(cert, key)?,
            listener: TcpListener::bind(socket)?,
            cache: Cache::new(),
        })
    }

    /// new_acceptor creates a new TLS acceptor with the given certificate and key.
    fn new_acceptor(cert: &str, key: &str) -> Result<Arc<SslAcceptor>, Error> {
        let mut acceptor = SslAcceptor::mozilla_intermediate(SslMethod::tls())?;
        acceptor.set_private_key_file(key, SslFiletype::PEM)?;
        acceptor.set_certificate_chain_file(cert)?;
        acceptor.check_private_key()?;
        acceptor.set_alpn_select_callback(|_, protos| {
            const H2: &[u8] = b"\x02h2";
            if protos.windows(3).any(|window| window == H2) {
                Ok(b"h2")
            } else {
                Err(AlpnError::NOACK)
            }
        });
        acceptor.set_alpn_protos(b"\x08http/1.1\x02h2")?;

        Ok(Arc::new(acceptor.build()))
    }

    // run does setup and takes an incoming TLS connection and sends its stream to be handled.
    pub fn run(&self) {
        for stream in self.listener.incoming() {
            match stream {
                Ok(stream) => {
                    let acceptor = Arc::clone(&self.acceptor);
                    let cache = Arc::clone(&self.cache);
                    thread::spawn(move || handle_stream(stream, acceptor, cache));
                }
                Err(e) => {
                    eprintln!("error in TCP accept: {}", e);
                }
            }
        }
    }
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
    fn write(&mut self, buf: &[u8]) -> Result<usize, io::Error> {
        self.0.lock().unwrap().write(buf)
    }
    fn flush(&mut self) -> Result<(), io::Error> {
        self.0.lock().unwrap().flush()
    }
}

// io::Read for Wrapper
impl io::Read for Wrapper {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, io::Error> {
        self.0.lock().unwrap().read(buf)
    }
}

// solicit::http::transport::TransportStream
impl TransportStream for Wrapper {
    fn try_split(&self) -> Result<Wrapper, io::Error> {
        Ok(Wrapper(self.0.clone()))
    }

    fn close(&mut self) -> Result<(), io::Error> {
        match self.0.lock().unwrap().shutdown() {
            Ok(_) => Ok(()),
            Err(e) => Err(io::Error::new(io::ErrorKind::Other, e)),
        }
    }
}

/// handle_request processes an HTTP/2 request. It always returns a Response.
fn handle_request(req: ServerRequest, cache: Arc<Cache>) -> Response {
    let mut filename = String::from("index.html");
    for (name, value) in req.headers {
        let name = str::from_utf8(&name).unwrap();
        let value = str::from_utf8(&value).unwrap();
        if name == ":path" {
            filename = format!(".{}", value);
            if filename.ends_with("/") {
                filename = format!("{}{}", filename, "index.html");
            }
        }
    }

    let mut response = handle_cache_entry(cache.get(&filename[..]));
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

/// handle_stream processess an HTTP/2 TCP/TLS streaml
fn handle_stream(stream: TcpStream, acceptor: Arc<SslAcceptor>, cache: Arc<Cache>) {
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
                let cache = Arc::clone(&cache);
                responses.push(handle_request(req, cache));
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
