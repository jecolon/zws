use zws::handlers::StaticFile;
use zws::{Handler, Request, Response, Server};

fn main() -> zws::Result<()> {
    Server::new("tls/dev/cert.pem", "tls/dev/key.pem", "127.0.0.1:8443")?
        .add_handler("GET /hello", StringHandler::new("Hello"))?
        .add_handler("GET /", StaticFile::with_cache("webroot")?)?
        .add_handler_func("GET /user/:fname/:lname/:age", greeter_func)?
        .run()
}

fn greeter_func(req: Request, mut resp: Response) -> Response {
    if req.has_params() {
        let fname = req.param("fname");
        let lname = req.param("lname");
        let age = req.param("age");
        resp.set_body(format!(
            "Hello {} {}. You are {} years old!",
            fname, lname, age
        ));
    } else {
        resp.set_body("Hello stranger!");
    }
    resp
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
        resp.set_body(self.body.clone());
        resp
    }
}
