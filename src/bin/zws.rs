use std::path::PathBuf;

use solicit::http::Response;

use zws;

fn main() -> zws::Result<()> {
    let srv = zws::Server::new(
        &"tls/dev/cert.pem".to_string(),
        &"tls/dev/key.pem".to_string(),
        &"127.0.0.1:8443".to_string(),
    )?;
    srv.add_handler(
        zws::Action::GET(PathBuf::from("/hello")),
        Box::new(HelloHandler {}),
    )
    .add_handler(
        zws::Action::GET(PathBuf::from("/")),
        Box::new(zws::handlers::StaticFile::new("webroot", true)?),
    )
    .run()
}

struct HelloHandler;

impl zws::Handler for HelloHandler {
    fn handle(&self, req: zws::ServerRequest) -> Response {
        Response {
            stream_id: req.stream_id,
            headers: vec![(b":status".to_vec(), b"200".to_vec())],
            body: b"Hello world!".to_vec(),
        }
    }
}
