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
use solicit::http::{Header, HttpError, HttpResult, HttpScheme, Response, StreamId};

// new_acceptor creates a new TLS acceptor with the given certificate and key.
pub fn new_acceptor(cert: &str, key: &str) -> Result<SslAcceptor, Error> {
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

    Ok(acceptor.build())
}

// handle_incoming takes an incoming TLS connection and sends its stream to be handled.
pub fn run() {
    let acceptor = new_acceptor("tls/dev/cert.pem", "tls/dev/key.pem").unwrap();
    let listener = TcpListener::bind("127.0.0.1:8443").unwrap();

    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let stream = match acceptor.accept(stream) {
                    Ok(stream) => stream,
                    Err(e) => {
                        eprintln!("error in TLS accept: {}", e);
                        continue;
                    }
                };
                thread::spawn(|| handle_stream(stream));
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

fn handle_stream(stream: SslStream<TcpStream>) {
    let mut stream = Wrapper(Arc::new(RefCell::new(stream)));

    let mut preface = [0; 24];
    TransportStream::read_exact(&mut stream, &mut preface).unwrap();
    if &preface != b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n" {
        return;
    }

    let conn = HttpConnection::<Wrapper, Wrapper>::with_stream(stream, HttpScheme::Https);
    let mut conn: ServerConnection<Wrapper, Wrapper> =
        ServerConnection::with_connection(conn, DefaultSessionState::new());
    conn.init().unwrap();

    while let Ok(_) = conn.handle_next_frame() {
        let mut responses = Vec::new();
        for stream in conn.state.iter() {
            if stream.is_closed_remote() {
                let req = ServerRequest {
                    stream_id: stream.stream_id,
                    headers: stream.headers.as_ref().unwrap(),
                    body: &stream.body,
                };
                responses.push(handle_request(req));
            }
        }

        for response in responses {
            conn.start_response(response.headers, response.stream_id, EndStream::No)
                .unwrap();
            let stream = conn.state.get_stream_mut(response.stream_id).unwrap();
            stream.set_full_data(response.body);
        }

        while let SendStatus::Sent = conn.send_next_data().unwrap() {}
        let _ = conn.state.get_closed();
    }
}

pub struct SimpleServer<TS, H>
where
    TS: TransportStream,
    H: FnMut(ServerRequest) -> Response,
{
    conn: ServerConnection<TS, TS>,
    handler: H,
}

impl<TS, H> SimpleServer<TS, H>
where
    TS: TransportStream,
    H: FnMut(ServerRequest) -> Response,
{
    /// Creates a new `SimpleServer` that will use the given `TransportStream` to communicate to
    /// the client. Assumes that the stream is fully uninitialized -- no preface sent or read yet.
    pub fn new(mut stream: TS, handler: H) -> HttpResult<SimpleServer<TS, H>> {
        // First assert that the preface is received
        let mut preface = [0; 24];
        TransportStream::read_exact(&mut stream, &mut preface)?;
        if &preface != b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n" {
            return Err(HttpError::UnableToConnect);
        }

        let conn = HttpConnection::<TS, TS>::with_stream(stream, HttpScheme::Https);
        let mut server = SimpleServer {
            conn: ServerConnection::with_connection(conn, DefaultSessionState::new()),
            handler: handler,
        };

        // Initialize the connection -- send own settings and process the peer's
        server.conn.init()?;

        // Set up done
        Ok(server)
    }

    /// Handles the next incoming frame, blocking to receive it if nothing is available on the
    /// underlying stream.
    ///
    /// Handling the frame can trigger the handler callback. Any responses returned by the handler
    /// are immediately flushed out to the client (blocking the call until it's done).
    pub fn handle_next(&mut self) -> HttpResult<()> {
        self.conn.handle_next_frame()?;
        let responses = self.handle_requests()?;
        self.prepare_responses(responses)?;
        self.flush_streams()?;
        self.reap_streams()?;

        Ok(())
    }

    /// Invokes the request handler for each fully received request. Collects all the responses
    /// into the returned `Vec`.
    fn handle_requests(&mut self) -> HttpResult<Vec<Response>> {
        let handler = &mut self.handler;
        Ok(self
            .conn
            .state
            .iter()
            .filter(|s| s.is_closed_remote())
            .map(|stream| {
                let req = ServerRequest {
                    stream_id: stream.stream_id,
                    headers: stream.headers.as_ref().unwrap(),
                    body: &stream.body,
                };
                handler(req)
            })
            .collect())
    }

    /// Prepares the streams for each of the given responses. Headers for each response are
    /// immediately sent and the data staged into the streams' outgoing buffer.
    fn prepare_responses(&mut self, responses: Vec<Response>) -> HttpResult<()> {
        for response in responses.into_iter() {
            self.conn
                .start_response(response.headers, response.stream_id, EndStream::No)?;
            let stream = self.conn.state.get_stream_mut(response.stream_id).unwrap();
            stream.set_full_data(response.body);
        }

        Ok(())
    }

    /// Flushes the outgoing buffers of all streams.
    #[inline]
    fn flush_streams(&mut self) -> HttpResult<()> {
        while let SendStatus::Sent = self.conn.send_next_data()? {}

        Ok(())
    }

    /// Removes closed streams from the connection state.
    #[inline]
    fn reap_streams(&mut self) -> HttpResult<()> {
        // Moves the streams out of the state and then drops them
        let _ = self.conn.state.get_closed();
        Ok(())
    }
}
