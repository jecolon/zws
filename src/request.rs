use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::str::{self, FromStr};

use solicit::http::session::DefaultStream;
use solicit::http::{Header, StreamId};

use crate::error::{Result, ServerError};

/// Method is an HTTP verb.
#[derive(Clone, Debug)]
pub enum Method {
    GET,
}

/// Action is an HTTP method and path combination.
#[derive(Clone, Debug)]
pub struct Action {
    pub method: Method,
    pub path: String,
    pub params: Option<HashMap<String, usize>>,
}

impl PartialEq for Action {
    fn eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}
impl Eq for Action {}

impl Hash for Action {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.path.hash(state);
    }
}

impl FromStr for Action {
    type Err = ServerError;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        let parts: Vec<&str> = s.trim().split(' ').map(|p| p.trim()).collect();

        if parts.len() != 2 {
            return Err(ServerError::ParseAction(format!(
                "Request actions have 2 parts, got {}",
                parts.len()
            )));
        }

        let mut path = parts[1].to_string();
        let mut params = None;

        if path.contains(':') {
            let path_parts: Vec<&str> = parts[1].split('/').collect();

            // Extract params
            let mut params_map = HashMap::new();
            for (index, part) in path_parts.iter().enumerate() {
                if part.contains(':') {
                    params_map.insert(part[1..].to_string(), index);
                }
            }
            params = Some(params_map);

            // Extract path to match
            path.clear();
            for p in path_parts.iter().skip(1) {
                if p.contains(':') {
                    break;
                }
                path.push('/');
                path.push_str(p);
            }
            if path.is_empty() {
                path.push('/');
            }
        }
        debug!("from_str: path is {}, params is {:?}", path, params);

        match &parts[0].to_uppercase()[..] {
            "GET" => Ok(Action {
                method: Method::GET,
                path: path,
                params: params,
            }),
            _ => Err(ServerError::ParseAction(format!(
                "Request action verb not implemented: {}",
                parts[0]
            ))),
        }
    }
}

/// Request represents a fully received request.
pub struct Request<'a> {
    pub action: Action,
    pub path: String,
    pub params: Option<HashMap<String, String>>,
    pub query: Option<String>,
    pub stream_id: StreamId,
    pub headers: &'a [Header],
    pub body: &'a [u8],
}

impl<'a> Request<'a> {
    pub fn new(stream: &'a DefaultStream, actions: &Vec<Action>) -> Result<Request<'a>> {
        let headers = match stream.headers.as_ref() {
            Some(h) => h,
            None => {
                warn!("error, no HTTP/2 stream headers");
                return Err(ServerError::BadRequest);
            }
        };

        let mut req = Request {
            action: Action {
                method: Method::GET,
                path: String::new(),
                params: None,
            },
            path: String::new(),
            params: None,
            query: None,
            stream_id: stream.stream_id,
            headers: headers,
            body: &stream.body,
        };

        let method = match req.header(":method") {
            Some(method) => match method.as_str() {
                "GET" => Method::GET,
                _ => {
                    warn!("error, unsupported request method: {}", method);
                    return Err(ServerError::BadRequest);
                }
            },
            None => {
                warn!("error, request without :method header");
                return Err(ServerError::BadRequest);
            }
        };

        let path = match req.header(":path") {
            Some(path) => {
                let parts: Vec<&str> = path.split('?').collect();
                req.path = parts[0].to_string();
                if parts.len() > 1 {
                    req.query = Some(parts[1].to_string());
                }
                parts[0].to_string()
            }
            None => {
                warn!("error, request without :path header");
                return Err(ServerError::BadRequest);
            }
        };

        req.action = Action {
            method,
            path,
            params: None,
        };

        let mut test_action = req.action.clone();
        let mut action_path = PathBuf::from(&test_action.path);
        let mut done = false;

        for action in actions {
            if action == &test_action {
                req.action = action.clone();
                done = true;
                break;
            }
        }

        if !done {
            'WHILE: while action_path.pop() {
                test_action = Action {
                    method: Method::GET,
                    path: action_path.to_string_lossy().to_string(),
                    params: None,
                };
                for action in actions {
                    if action == &test_action {
                        req.action = action.clone();
                        break 'WHILE;
                    }
                }
            }
        }

        if let Some(params) = &req.action.params {
            let mut req_params = HashMap::new();
            let path_parts: Vec<&str> = req.path.split('/').collect();
            for (name, index) in params {
                if index < &path_parts.len() {
                    req_params.insert(name.to_string(), path_parts[*index].to_string());
                }
            }
            if req_params.len() > 0 {
                req.params = Some(req_params);
            }
        }

        debug!("new: Request: {}", &req);
        Ok(req)
    }

    pub fn has_params(&self) -> bool {
        self.params != None
    }

    pub fn param(&self, name: &str) -> &str {
        if let Some(params) = &self.params {
            if let Some(value) = params.get(name) {
                return &value;
            }
        }
        ""
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
            "{{ action: {:?}, path: '{}', params: {:?}, query: '{:?}, 'body_len: {} }}",
            &self.action,
            &self.path,
            &self.params,
            &self.query,
            &self.body.len()
        )
    }
}
