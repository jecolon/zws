use zws;

fn main() -> zws::Result<()> {
    zws::Server::new(
        "webroot",
        "tls/dev/cert.pem",
        "tls/dev/key.pem",
        "127.0.0.1:8443",
    )?
    .run()
}
