use std::io;
use std::net::TcpStream;
use std::sync::{Arc, Mutex};

use openssl::ssl::{ShutdownResult, SslStream};
use solicit::http::transport::TransportStream;

/// Wrapper is a newtype to implement solicit's TransportStream for an SslStream<TcpStream>.
pub struct Wrapper(pub Arc<Mutex<SslStream<TcpStream>>>);

// io::Write for Wrapper
impl io::Write for Wrapper {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.lock().unwrap().flush()
    }
}

// io::Read for Wrapper
impl io::Read for Wrapper {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.lock().unwrap().read(buf)
    }
}

// solicit::http::transport::TransportStream
impl TransportStream for Wrapper {
    fn try_split(&self) -> io::Result<Wrapper> {
        Ok(Wrapper(self.0.clone()))
    }

    fn close(&mut self) -> io::Result<()> {
        loop {
            match self.0.lock().unwrap().shutdown() {
                Ok(ShutdownResult::Received) => return Ok(()),
                Err(e) => return Err(io::Error::new(io::ErrorKind::Other, e)),
                _ => continue,
            }
        }
    }
}
