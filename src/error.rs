use std::{error, fmt, io, result};

use openssl::error::ErrorStack as SslErrorStack;

pub type Result<T> = result::Result<T, ServerError>;

#[derive(Debug)]
pub enum ServerError {
    BadRequest,
    Io(io::Error),
    Ssl(SslErrorStack),
}

impl fmt::Display for ServerError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            ServerError::BadRequest => write!(f, "Bad request"),
            ServerError::Io(ref err) => write!(f, "Io error: {}", err),
            ServerError::Ssl(ref err) => write!(f, "SSL error: {}", err),
        }
    }
}

impl error::Error for ServerError {
    fn description(&self) -> &str {
        match *self {
            ServerError::BadRequest => "Bad request",
            ServerError::Io(ref err) => err.description(),
            ServerError::Ssl(ref err) => err.description(),
        }
    }

    fn cause(&self) -> Option<&error::Error> {
        match *self {
            ServerError::BadRequest => None,
            ServerError::Io(ref err) => Some(err),
            ServerError::Ssl(ref err) => Some(err),
        }
    }
}

impl From<io::Error> for ServerError {
    fn from(err: io::Error) -> ServerError {
        ServerError::Io(err)
    }
}

impl From<SslErrorStack> for ServerError {
    fn from(err: SslErrorStack) -> ServerError {
        ServerError::Ssl(err)
    }
}
