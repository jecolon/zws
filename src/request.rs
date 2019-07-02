use std::fmt;
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

/// Request represents a fully received request.
pub struct Request<'a> {
    pub action: Action,
    pub method: String,
    pub path: String,
    pub query: String,
    pub stream_id: StreamId,
    pub headers: &'a [Header],
    pub body: &'a [u8],
}

impl<'a> Request<'a> {
    pub fn new(stream: &DefaultStream) -> Result<Request> {
        let headers = match stream.headers.as_ref() {
            Some(h) => h,
            None => {
                warn!("error, no HTTP/2 stream headers");
                return Err(ServerError::BadRequest);
            }
        };

        let mut req = Request {
            action: Action::GET(String::new()),
            method: "".to_string(),
            path: "".to_string(),
            query: "".to_string(),
            stream_id: stream.stream_id,
            headers: headers,
            body: &stream.body,
        };

        req.method = match req.header(":method") {
            Some(method) => method,
            None => {
                warn!("error, request without :method header");
                return Err(ServerError::BadRequest);
            }
        };

        match req.header(":path") {
            Some(path) => {
                let parts: Vec<&str> = path.split('?').collect();
                req.path = parts[0].to_string();
                if parts.len() > 1 {
                    req.query = parts[1].to_string();
                }
            }
            None => {
                warn!("error, request without :path header");
                return Err(ServerError::BadRequest);
            }
        }

        req.action = match req.method.as_str() {
            "GET" => Action::GET(req.path.clone()),
            _ => {
                warn!("error, unsupported request method: {}", req.method);
                return Err(ServerError::BadRequest);
            }
        };

        debug!("new: Request: {}", &req);
        Ok(req)
    }

    pub fn header(&self, name: &str) -> Option<String> {
        for (key, value) in self.headers {
            if key == &name.as_bytes() {
                return match str::from_utf8(value) {
                    Ok(sv) => Some(sv.to_string()),
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

impl<'a> fmt::Display for Request<'a> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "{{ action: {:?}, method: '{}', path: '{}', query: '{}', body_len: {} }}",
            &self.action,
            &self.method,
            &self.path,
            &self.query,
            &self.body.len()
        )
    }
}
