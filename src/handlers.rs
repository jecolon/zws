use std::path::PathBuf;
use std::sync::Arc;
use std::{io, str};

use solicit::http::Response;

use crate::error::Result;
use crate::mcache::{self, Cache, Entry};
use crate::request::ServerRequest;

// Handler is a type that produces a Response for a given ServerRequest.
pub trait Handler: Send + Sync {
    fn handle(&self, req: ServerRequest) -> Response;
}

pub struct StaticFile {
    cache: Option<Arc<Cache>>,
    webroot: PathBuf,
}

impl StaticFile {
    pub fn new(webroot: &str, caching: bool) -> Result<StaticFile> {
        let mut sf = StaticFile {
            cache: None,
            webroot: PathBuf::from(webroot).canonicalize()?,
        };

        if caching {
            sf.cache = Some(Cache::new(sf.webroot.clone()));
        }

        Ok(sf)
    }

    /// handle_cache_entry performs a cache get and unwraps the Response.
    fn handle_cache_entry(&self, (entry, found): (Entry, bool)) -> Response {
        if found {
            // Cache hit
            let &(_, _, ref rwl) = &*entry;
            return rwl.read().unwrap().clone().unwrap();
        }

        // Cache miss
        let &(ref mtx, ref cnd, ref rwl) = &*entry;
        let mut guard = mtx.lock().unwrap();
        while !*guard {
            guard = cnd.wait(guard).unwrap();
        }
        rwl.read().unwrap().clone().unwrap()
    }
}

impl Handler for StaticFile {
    fn handle(&self, req: ServerRequest) -> Response {
        let (mut path, original) = match req.header(":path") {
            Some(path) => (PathBuf::from(&self.webroot).join(&path[1..]), path),
            None => {
                return Response {
                    stream_id: req.stream_id,
                    headers: vec![(b":status".to_vec(), b"400".to_vec())],
                    body: b"Bad Request\n".to_vec(),
                }
            }
        };
        debug!("FileHandler: path is {:?}", &path);
        let is_dir = path.is_dir();
        if !original.ends_with("/") && is_dir {
            let path_with_slash = format!("{}{}", original, "/");
            debug!(
                "FileHandler: redirecting dir path without trailing slash to {}",
                path_with_slash
            );
            return Response {
                stream_id: req.stream_id,
                headers: vec![
                    (b":status".to_vec(), b"307".to_vec()),
                    (b"location".to_vec(), path_with_slash.into_bytes()),
                ],
                body: b"Moved Temporarily\n".to_vec(),
            };
        }

        if is_dir {
            path.push("index.html");
        }

        let path = match path.canonicalize() {
            Ok(path) => path,
            Err(e) => {
                debug!("FileHandler: error canonicalizing path {:?}: {}", path, e);
                match e.kind() {
                    io::ErrorKind::NotFound => {
                        return Response {
                            stream_id: req.stream_id,
                            headers: vec![(b":status".to_vec(), b"404".to_vec())],
                            body: b"Not Found\n".to_vec(),
                        }
                    }
                    _ => {
                        return Response {
                            stream_id: req.stream_id,
                            headers: vec![(b":status".to_vec(), b"400".to_vec())],
                            body: b"Bad Request\n".to_vec(),
                        }
                    }
                }
            }
        };

        let filename = path.to_string_lossy();
        debug!("FileHandler: filename is {}", &filename);

        let mut response = match &self.cache {
            Some(cache) => {
                let cache = Arc::clone(&cache);
                self.handle_cache_entry(cache.get(&filename))
            }
            None => mcache::file_response(&filename).0,
        };

        response.stream_id = req.stream_id;
        response
    }
}

/// NotFound is a handler that always returns a 404 Not Found Response.
pub struct NotFound;

impl Handler for NotFound {
    fn handle(&self, req: ServerRequest) -> Response {
        Response {
            stream_id: req.stream_id,
            headers: vec![(b":status".to_vec(), b"404".to_vec())],
            body: b"Not Found\n".to_vec(),
        }
    }
}
