use zws::handlers::StaticFile;
use zws::{Handler, Request, Response, Server};

fn main() -> zws::Result<()> {
    Server::new("tls/dev/cert.pem", "tls/dev/key.pem", "127.0.0.1:8443")?
        .add_handler("GET /hello", StringHandler::new("Hello"))?
        .add_handler("GET /", StaticFile::new("webroot", true)?)?
        .add_handler_func("GET /user/:fname/:lname/:age", |req, mut resp| {
            resp.header(":status", "200");
            if let Some(params) = req.params {
                let fname = params.get("fname").unwrap();
                let lname = params.get("lname").unwrap();
                let age = params.get("age").unwrap();
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
