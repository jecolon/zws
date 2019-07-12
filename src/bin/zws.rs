use zws::handlers::StaticFile;
use zws::{Handler, Request, Response, Server};

fn main() -> zws::Result<()> {
    Server::new("tls/dev/cert.pem", "tls/dev/key.pem", "127.0.0.1:8443")?
        .add_handler("GET /hello", StringHandler::new("Hello"))?
        .add_handler("GET /", StaticFile::new("webroot", true)?)?
        .add_handler_func("GET /user/:fname/:lname/:age", |req, mut resp| {
            resp.header(":status", "200");
            if req.has_params() {
                let fname = req.param("fname");
                let lname = req.param("lname");
                let age = req.param("age");
                resp.body(format!(
                    "Hello {} {}. You are {} years old!",
                    fname, lname, age
                ));
            } else {
                resp.body("Hello stranger!");
            }
            resp
        })?
        .run()
}

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
