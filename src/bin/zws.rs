use std::env;

use docopt::Docopt;
use num_cpus;

use zws::handlers::StaticFile;
use zws::{Handler, Request, Response, Server};

fn main() -> zws::Result<()> {
    const USAGE: &'static str = "
Usage: zws [-h] [-c CERT] [-k KEY] [-s SOCKET] [-t THREADS] [-w DIR]

Options:
    -h, --help
        Show this usage screen.

    -c CERT, --cert CERT
        Path to PEM certificate file. [default: tls/dev/cert.pem]

    -k KEY, --key KEY
        Path to PEM key file. [default: tls/dev/key.pem]
        
    -s SOCKET, --socket SOCKET
        TCP socket to listen on. [default: 127.0.0.1:8443]

    -t THREADS, --threads THREADS
        Number of threads for worker pool request handling.
        0 = Total logical CPUs. [default: 0]

    -w DIR, --webroot DIR
        Path to root of file serving area. [default: webroot]
";

    let argv = env::args();
    let args = Docopt::new(USAGE)
        .and_then(|d| d.argv(argv.into_iter()).parse())
        .unwrap_or_else(|e| e.exit());

    let mut threads: usize = args.get_str("--threads").parse().unwrap_or(0);
    if threads == 0 {
        threads = num_cpus::get();
    }

    let webroot = args.get_str("--webroot");

    Server::builder()
        .tls(args.get_str("--cert"), args.get_str("--key"))
        .socket(args.get_str("--socket"))
        .threads(threads)
        .handler("GET /hello", StringHandler::new("Hello"))?
        .handler("GET /", StaticFile::with_cache(webroot)?)?
        .handler_func("GET /user/:fname/:lname/:age", greeter_func)?
        .build()?
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
    fn new(s: &str) -> StringHandler {
        StringHandler {
            body: s.to_string(),
        }
    }
}

impl Handler for StringHandler {
    fn handle(&self, _req: Request, mut resp: Response) -> Response {
        resp.set_body(self.body.clone());
        resp
    }
}
