use zws::handlers::StaticFile;
use zws::{Action, Handler, Response, Server, ServerRequest};

fn main() -> zws::Result<()> {
    Server::new("tls/dev/cert.pem", "tls/dev/key.pem", "127.0.0.1:8443")?
        .add_handler(Action::GET("/hello".to_string()), Box::new(HelloHandler {}))
        .add_handler(
            Action::GET("/".to_string()),
            Box::new(StaticFile::new("webroot", true)?),
        )
        .run()
}

struct HelloHandler;

impl Handler for HelloHandler {
    fn handle(&self, req: ServerRequest) -> Response {
        Response {
            stream_id: req.stream_id,
            headers: vec![(b":status".to_vec(), b"200".to_vec())],
            body: b"Hello world!".to_vec(),
        }
    }
}
