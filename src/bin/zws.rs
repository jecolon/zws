use std::env;

use docopt::Docopt;

use zws;

fn main() -> zws::Result<()> {
    const USAGE: &'static str = "
Usage: zws [-c CERT] [-k KEY] [-n] [-s SOCKET] [-w DIR]

Options:
    -c CERT, --cert CERT  Path to PEM certificate file. [default: tls/dev/cert.pem]
    -k KEY, --key KEY  Path to PEM key file. [default: tls/dev/key.pem]
    -n, --nocache  Disable response cache.
    -s SOCKET, --socket SOCKET  TCP socket to listen on. [default: 127.0.0.1:8443]
    -w DIR, --webroot DIR  Path to root of file serving area. [default: webroot]
";

    let argv = env::args();
    let args = Docopt::new(USAGE)
        .and_then(|d| d.argv(argv.into_iter()).parse())
        .unwrap_or_else(|e| e.exit());

    zws::Server::new(
        args.get_str("--webroot"),
        args.get_str("--cert"),
        args.get_str("--key"),
        args.get_str("--socket"),
        !args.get_bool("--nocache"),
    )?
    .run()
}
