extern crate openssl;

use std::cell::RefCell;
use std::io;
use std::net::{TcpListener, TcpStream};
use std::sync::Arc;
use std::thread;

use openssl::ssl::{AlpnError, Error, SslAcceptor, SslFiletype, SslMethod, SslStream};

use solicit::http::connection::{EndStream, HttpConnection, SendStatus};
use solicit::http::server::ServerConnection;
use solicit::http::session::{DefaultSessionState, SessionState, Stream};
use solicit::http::transport::TransportStream;
use solicit::http::{Header, HttpScheme, Response, StreamId};

// new_acceptor creates a new TLS acceptor with the given certificate and key.
pub fn new_acceptor(cert: &str, key: &str) -> Result<Arc<SslAcceptor>, Error> {
    let mut acceptor =
        SslAcceptor::mozilla_intermediate(SslMethod::tls()).expect("error creating SSL Acceptor");
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

// handle_incoming takes an incoming TLS connection and sends its stream to be handled.
pub fn run() {
    let acceptor = match new_acceptor("tls/dev/cert.pem", "tls/dev/key.pem") {
        Ok(acceptor) => acceptor,
        Err(e) => {
            eprintln!("error creating TLS acceptor: {}", e);
            return;
        }
    };
    let listener = match TcpListener::bind("127.0.0.1:8443") {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("error binding to TCP socket: {}", e);
            return;
        }
    };

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let acceptor = Arc::clone(&acceptor);
                thread::spawn(|| handle_stream(stream, acceptor));
            }
            Err(e) => {
                eprintln!("error in TCP accept: {}", e);
            }
        }
    }
}

/// The struct represents a fully received request.
pub struct ServerRequest<'a> {
    pub stream_id: StreamId,
    pub headers: &'a [Header],
    pub body: &'a [u8],
}

struct Wrapper(Arc<RefCell<SslStream<TcpStream>>>);

// io::Write
impl io::Write for Wrapper {
    fn write(&mut self, buf: &[u8]) -> Result<usize, io::Error> {
        self.0.borrow_mut().write(buf)
    }
    fn flush(&mut self) -> Result<(), io::Error> {
        self.0.borrow_mut().flush()
    }
}

// io::Read
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

fn handle_request(req: ServerRequest) -> Response {
    Response {
        headers: vec![(b":status".to_vec(), b"200".to_vec())],
        body: b"Hello World!".to_vec(),
        stream_id: req.stream_id,
    }
}

fn handle_stream(stream: TcpStream, acceptor: Arc<SslAcceptor>) {
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
                responses.push(handle_request(req));
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
