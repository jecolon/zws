use zws::handlers::StaticFile;
use zws::{Handler, Response, Server, ServerRequest};

fn main() -> zws::Result<()> {
    Server::new("tls/dev/cert.pem", "tls/dev/key.pem", "127.0.0.1:8443")?
        .add_handler("GET /hello", StringHandler::new("Hello"))?
        .add_handler("GET /hello/world", StringHandler::new("World"))?
        .add_handler("GET /", StaticFile::new("webroot", true)?)?
        .run()
}

struct StringHandler {
    body: Vec<u8>,
}

impl StringHandler {
    fn new(s: &str) -> Box<StringHandler> {
        Box::new(StringHandler {
            body: s.as_bytes().to_vec(),
        })
    }
}

impl Handler for StringHandler {
    fn handle(&self, req: ServerRequest) -> Response {
        Response {
            stream_id: req.stream_id,
            headers: vec![(b":status".to_vec(), b"200".to_vec())],
            body: self.body.clone(),
        }
    }
}
