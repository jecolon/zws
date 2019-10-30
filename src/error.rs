use std::{error, fmt, io, result};

use openssl::error::ErrorStack as SslErrorStack;

pub type Result<T> = result::Result<T, ServerError>;

#[derive(Debug)]
pub enum ServerError {
    ParseAction(String),
    BadRequest,
    Io(io::Error),
    Ssl(SslErrorStack),
}

impl fmt::Display for ServerError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ServerError::ParseAction(msg) => write!(f, "Error parsing Action: {}", msg),
            ServerError::BadRequest => write!(f, "Bad request"),
            ServerError::Io(ref err) => write!(f, "Io error: {}", err),
            ServerError::Ssl(ref err) => write!(f, "SSL error: {}", err),
        }
    }
}

impl error::Error for ServerError {
    fn description(&self) -> &str {
        match self {
            ServerError::ParseAction(_) => "Error parsing request action",
            ServerError::BadRequest => "Bad request",
            ServerError::Io(ref err) => err.description(),
            ServerError::Ssl(ref err) => err.description(),
        }
    }

    fn cause(&self) -> Option<&dyn error::Error> {
        match *self {
            ServerError::ParseAction(_) => None,
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
