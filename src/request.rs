use std::str::{self, FromStr};

use solicit::http::session::DefaultStream;
use solicit::http::{Header, StreamId};

use crate::error::{Result, ServerError};

/// Action is an HTTP method and path combination.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Hash)]
pub enum Action {
    GET(String),
}

impl FromStr for Action {
    type Err = ServerError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let parts: Vec<&str> = s.trim().split(' ').map(|p| p.trim()).collect();

        if parts.len() > 2 {
            return Err(ServerError::ParseAction(format!(
                "Request actions have 2 parts, got {}",
                parts.len()
            )));
        }

        match &parts[0].to_uppercase()[..] {
            "GET" => Ok(Action::GET(parts[1].to_string())),
            _ => Err(ServerError::ParseAction(format!(
                "Request action verb unknown: {}",
                parts[0]
            ))),
        }
    }
}

/// ServerRequest represents a fully received request.
pub struct ServerRequest<'a> {
    pub action: Action,
    pub stream_id: StreamId,
    pub headers: &'a [Header],
    pub body: &'a [u8],
}

impl<'a> ServerRequest<'a> {
    pub fn new(stream: &DefaultStream) -> Result<ServerRequest> {
        let headers = match stream.headers.as_ref() {
            Some(h) => h,
            None => {
                warn!("error, no HTTP/2 stream headers");
                return Err(ServerError::BadRequest);
            }
        };

        let mut req = ServerRequest {
            action: Action::GET(String::new()),
            stream_id: stream.stream_id,
            headers: headers,
            body: &stream.body,
        };

        req.action = match req.header(":method") {
            Some(method) => match method {
                "GET" => {
                    let path = match req.header(":path") {
                        Some(path) => path,
                        None => {
                            warn!("error, request without :path header");
                            return Err(ServerError::BadRequest);
                        }
                    };
                    Action::GET(String::from(path))
                }
                _ => {
                    warn!("error, unsupported request method");
                    return Err(ServerError::BadRequest);
                }
            },
            None => {
                warn!("error, request without :method header");
                return Err(ServerError::BadRequest);
            }
        };

        Ok(req)
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        for (key, value) in self.headers {
            if key == &name.as_bytes() {
                return match str::from_utf8(value) {
                    Ok(sv) => Some(sv),
                    Err(e) => {
                        warn!("error decoding header {} as UTF-8: {}", name, e);
                        None
                    }
                };
            }
        }
        None
    }
}
