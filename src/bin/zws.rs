use zws::handlers::StaticFile;
use zws::{Handler, Response, Server, ServerRequest};

fn main() -> zws::Result<()> {
    Server::new("tls/dev/cert.pem", "tls/dev/key.pem", "127.0.0.1:8443")?
        .add_handler("GET /hello", HelloHandler::new())?
        .add_handler("GET /hello/world", HelloHandler::new())?
        .add_handler("GET /", StaticFile::new("webroot", true)?)?
        .run()
}

struct HelloHandler;

impl HelloHandler {
    fn new() -> Box<HelloHandler> {
        Box::new(HelloHandler {})
    }
}

impl Handler for HelloHandler {
    fn handle(&self, req: ServerRequest) -> Response {
        Response {
            stream_id: req.stream_id,
            headers: vec![(b":status".to_vec(), b"200".to_vec())],
            body: b"Hello world!".to_vec(),
        }
    }
}
