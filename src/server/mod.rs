use std::cell::RefCell;
use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::io::prelude::*;
use std::io::BufReader;
use std::net::{TcpListener, TcpStream};
use std::str;
use std::sync::{Arc, RwLock};
use std::thread;

use openssl::ssl::{AlpnError, Error, SslAcceptor, SslFiletype, SslMethod, SslStream};

use solicit::http::connection::{EndStream, HttpConnection, SendStatus};
use solicit::http::server::ServerConnection;
use solicit::http::session::{DefaultSessionState, SessionState, Stream};
use solicit::http::transport::TransportStream;
use solicit::http::{Header, HttpScheme, Response, StreamId};

/// Server is a simple HTT/2 server
pub struct Server {
    acceptor: Arc<SslAcceptor>,
    listener: TcpListener,
    cache: Arc<MemCache>,
}

impl Server {
    /// new returns an initialized instance of Server
    pub fn new(cert: &str, key: &str, socket: &str) -> Result<Server, Box<std::error::Error>> {
        Ok(Server {
            acceptor: Server::new_acceptor(cert, key)?,
            listener: TcpListener::bind(socket)?,
            cache: MemCache::new(),
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
struct Wrapper(Arc<RefCell<SslStream<TcpStream>>>);

// io::Write for Wrapper
impl io::Write for Wrapper {
    fn write(&mut self, buf: &[u8]) -> Result<usize, io::Error> {
        self.0.borrow_mut().write(buf)
    }
    fn flush(&mut self) -> Result<(), io::Error> {
        self.0.borrow_mut().flush()
    }
}

// io::Read for Wrapper
impl io::Read for Wrapper {
    fn read(&mut self, buf: &mut [u8]) -> Result<usize, io::Error> {
        self.0.borrow_mut().read(buf)
    }
}

// solicit::http::transport::TransportStream
impl TransportStream for Wrapper {
    fn try_split(&self) -> Result<Wrapper, io::Error> {
        Ok(Wrapper(self.0.clone()))
    }

    fn close(&mut self) -> Result<(), io::Error> {
        match self.0.borrow_mut().shutdown() {
            Ok(_) => Ok(()),
            Err(e) => Err(io::Error::new(io::ErrorKind::Other, e)),
        }
    }
}

/// get_ctype produces a MIME content type string based on filename extension.
pub fn get_ctype(filename: &str) -> &str {
    let mut ctype = "application/octet-stream";

    if let Some(dot) = filename.rfind('.') {
        ctype = match &filename[dot..] {
            ".html" | ".htm" => "text/html; charset=utf-8",
            ".css" => "text/css",
            ".js" => "text/javascript",
            ".png" => "image/png",
            ".jpg" | ".jpeg" => "image/jpeg",
            ".gif" => "image/gif",
            ".svg" => "image/svg+xml",
            ".webp" => "image/webp",
            ".txt" => "text/plain; charset=utf-8",
            ".json" => "application/json",
            _ => "binary/octet-stream",
        }
    }

    &ctype
}

/// handle_request processes an HTTP/2 request. It always returns a Response.
fn handle_request(req: ServerRequest, cache: Arc<MemCache>) -> Response {
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
    let filename = &filename;

    let mut resp = cache.get(filename);
    resp.stream_id = req.stream_id;
    resp
}

/// handle_stream processess an HTTP/2 TCP/TLS streaml
fn handle_stream(stream: TcpStream, acceptor: Arc<SslAcceptor>, cache: Arc<MemCache>) {
    let stream = match acceptor.accept(stream) {
        Ok(stream) => stream,
        Err(e) => {
            eprintln!("error in TLS accept: {}", e);
            return;
        }
    };
    let mut stream = Wrapper(Arc::new(RefCell::new(stream)));

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

// Memcache is a concurrency safe cache for Responses.
struct MemCache {
    store: RwLock<HashMap<String, Response>>,
}

impl MemCache {
    /// new returns a new initialized MemCache instance.
    pub fn new() -> Arc<MemCache> {
        Arc::new(MemCache {
            store: RwLock::new(HashMap::new()),
        })
    }

    /// get returns an HTTP/2 response for filename. It always returns a Response.
    fn get(&self, filename: &String) -> Response {
        // Short circuit return if found
        if let Some(resp) = self.store.read().unwrap().get(filename) {
            return resp.clone();
        }

        let file = match File::open(filename) {
            Ok(file) => file,
            Err(e) => {
                eprintln!("error opening file {}: {}", filename, e);
                if io::ErrorKind::NotFound == e.kind() {
                    return Response {
                        headers: vec![(b":status".to_vec(), b"404".to_vec())],
                        body: b"Not Found\n".to_vec(),
                        stream_id: 0,
                    };
                }
                return Response {
                    headers: vec![(b":status".to_vec(), b"500".to_vec())],
                    body: b"Unable to get file\n".to_vec(),
                    stream_id: 0,
                };
            }
        };

        let meta = match file.metadata() {
            Ok(meta) => meta,
            Err(e) => {
                eprintln!("error reading file {} metadata: {}", filename, e);
                return Response {
                    headers: vec![(b":status".to_vec(), b"500".to_vec())],
                    body: b"Unable to get file metadata\n".to_vec(),
                    stream_id: 0,
                };
            }
        };

        let mut buf_reader = BufReader::new(file);
        let mut buf = Vec::with_capacity(meta.len() as usize);
        if let Err(e) = buf_reader.read_to_end(&mut buf) {
            eprintln!("error reading file {}: {}", filename, e);
            return Response {
                headers: vec![(b":status".to_vec(), b"500".to_vec())],
                body: b"Unable to read file\n".to_vec(),
                stream_id: 0,
            };
        }

        let ctype = get_ctype(filename);

        let resp = Response {
            headers: vec![
                (b":status".to_vec(), b"200".to_vec()),
                (b"content-type".to_vec(), ctype.as_bytes().to_vec()),
            ],
            body: buf,
            stream_id: 0,
        };

        self.store
            .write()
            .unwrap()
            .insert(filename.to_string(), resp.clone());

        resp
    }
}
