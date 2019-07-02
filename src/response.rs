use std::collections::HashMap;
use std::convert::Into;

use solicit::http;

#[derive(Clone)]
pub struct Response {
    stream_id: http::StreamId,
    pseudo_headers: HashMap<String, String>,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

impl Response {
    pub fn new(id: http::StreamId) -> Response {
        Response {
            stream_id: id,
            pseudo_headers: HashMap::new(),
            headers: HashMap::new(),
            body: Vec::new(),
        }
    }

    pub fn stream_id(&mut self, id: http::StreamId) {
        self.stream_id = id;
    }

    pub fn header(&mut self, key: &str, value: &str) {
        if key.starts_with(':') {
            self.pseudo_headers
                .insert(key.to_string(), value.to_string());
        } else {
            self.headers.insert(key.to_string(), value.to_string());
        }
    }

    pub fn body<T: Into<Vec<u8>>>(&mut self, b: T) {
        self.body = b.into();
    }
}

impl Into<http::Response> for Response {
    fn into(self) -> http::Response {
        let mut resp = http::Response {
            stream_id: self.stream_id,
            headers: Vec::new(),
            body: self.body,
        };

        self.pseudo_headers.into_iter().for_each(|(k, v)| {
            resp.headers.push((k.into_bytes(), v.into_bytes()));
        });

        self.headers.into_iter().for_each(|(k, v)| {
            resp.headers.push((k.into_bytes(), v.into_bytes()));
        });

        resp
    }
}
