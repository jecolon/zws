use zws::handlers::StaticFile;
use zws::{Handler, Request, Response, Server};

fn main() -> zws::Result<()> {
    Server::new("tls/dev/cert.pem", "tls/dev/key.pem", "127.0.0.1:8443")?
        .add_handler("GET /hello", StringHandler::new("Hello"))?
        .add_handler("GET /hello/world", StringHandler::new("World"))?
        .add_handler("GET /", StaticFile::new("webroot", true)?)?
        .run()
}

#[derive(Clone)]
struct StringHandler {
    body: String,
}

impl StringHandler {
    fn new(s: &str) -> Box<StringHandler> {
        Box::new(StringHandler {
            body: s.to_string(),
        })
    }
}

impl Handler for StringHandler {
    fn handle(&self, _req: Request, mut resp: Response) -> Response {
        resp.header(":status", "200");
        resp.body(self.body.clone());
        resp
    }
}
