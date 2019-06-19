use seahash::SeaHasher;
use std::collections::HashMap;
use std::fs::File;
use std::hash::BuildHasherDefault;
use std::io;
use std::io::prelude::*;
use std::io::BufReader;
use std::sync::{Arc, RwLock};

use solicit::http::session::Stream;
use solicit::http::Response;

/// BuildHasher lets us use SeaHasher with HashMap.
type BuildHasher = BuildHasherDefault<SeaHasher>;

// Memcache is a concurrency safe cache for Responses.
pub struct MemCache {
    store: RwLock<HashMap<String, Response, BuildHasher>>,
}

impl MemCache {
    /// new returns a new initialized MemCache instance.
    pub fn new() -> Arc<MemCache> {
        Arc::new(MemCache {
            store: RwLock::new(HashMap::<String, Response, BuildHasher>::default()),
        })
    }

    /// get returns an HTTP/2 response for filename. It always returns a Response.
    pub fn get(&self, filename: &String) -> Response {
        // Short circuit return if found
        if let Some(resp) = self.store.read().unwrap().get(filename) {
            return resp.clone();
        }

        let file = match File::open(filename) {
            Ok(file) => file,
            Err(e) => {
                eprintln!("error opening file {}: {}", filename, e);
                if io::ErrorKind::NotFound == e.kind() {
                    return Response {
                        headers: vec![(b":status".to_vec(), b"404".to_vec())],
                        body: b"Not Found\n".to_vec(),
                        stream_id: 0,
                    };
                }
                return Response {
                    headers: vec![(b":status".to_vec(), b"500".to_vec())],
                    body: b"Unable to get file\n".to_vec(),
                    stream_id: 0,
                };
            }
        };

        let meta = match file.metadata() {
            Ok(meta) => meta,
            Err(e) => {
                eprintln!("error reading file {} metadata: {}", filename, e);
                return Response {
                    headers: vec![(b":status".to_vec(), b"500".to_vec())],
                    body: b"Unable to get file metadata\n".to_vec(),
                    stream_id: 0,
                };
            }
        };

        let mut buf_reader = BufReader::new(file);
        let mut buf = Vec::with_capacity(meta.len() as usize);
        if let Err(e) = buf_reader.read_to_end(&mut buf) {
            eprintln!("error reading file {}: {}", filename, e);
            return Response {
                headers: vec![(b":status".to_vec(), b"500".to_vec())],
                body: b"Unable to read file\n".to_vec(),
                stream_id: 0,
            };
        }

        let ctype = get_ctype(filename);

        let resp = Response {
            headers: vec![
                (b":status".to_vec(), b"200".to_vec()),
                (b"content-type".to_vec(), ctype.as_bytes().to_vec()),
            ],
            body: buf,
            stream_id: 0,
        };

        self.store
            .write()
            .unwrap()
            .insert(filename.to_string(), resp.clone());

        resp
    }
}

/// get_ctype produces a MIME content type string based on filename extension.
fn get_ctype(filename: &str) -> &str {
    let mut ctype = "application/octet-stream";

    if let Some(dot) = filename.rfind('.') {
        ctype = match &filename[dot..] {
            ".html" | ".htm" => "text/html; charset=utf-8",
            ".css" => "text/css",
            ".js" => "text/javascript",
            ".png" => "image/png",
            ".jpg" | ".jpeg" => "image/jpeg",
            ".gif" => "image/gif",
            ".svg" => "image/svg+xml",
            ".webp" => "image/webp",
            ".txt" => "text/plain; charset=utf-8",
            ".json" => "application/json",
            _ => "binary/octet-stream",
        }
    }

    &ctype
}
