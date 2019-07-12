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
            pseudo_headers: [(":status".to_string(), "200".to_string())]
                .iter()
                .cloned()
                .collect(),
            headers: [("server".to_string(), "zws".to_string())]
                .iter()
                .cloned()
                .collect(),
            body: Vec::new(),
        }
    }

    pub fn stream_id(&mut self, id: http::StreamId) {
        self.stream_id = id;
    }

    pub fn add_header(&mut self, key: &str, value: &str) {
        if key.starts_with(':') {
            self.pseudo_headers
                .insert(key.to_string(), value.to_string());
        } else {
            self.headers.insert(key.to_string(), value.to_string());
        }
    }

    pub fn set_body<T: Into<Vec<u8>>>(&mut self, b: T) {
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
