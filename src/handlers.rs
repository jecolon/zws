use std::str;
use std::sync::Arc;

use crate::mcache::{self, Entry};
use crate::request::ServerRequest;
use crate::server::Server;

use solicit::http::Response;

// Handler is a function that produces a Response for a given ServerRequest.
pub type Handler = fn(ServerRequest, Arc<Server>) -> Response;

/// file_handler processes a request for a file. It always returns a Response.
pub fn file_handler(req: ServerRequest, srv: Arc<Server>) -> Response {
    let mut filename = srv.webroot.clone();
    filename.push("index.html");

    for (name, value) in req.headers {
        //let name = str::from_utf8(&name).unwrap();
        if name == b":path" {
            // Site root
            if value == b"" || value == b"/" {
                break;
            }

            let mut value = match str::from_utf8(value) {
                Ok(value) => value,
                Err(e) => {
                    warn!("error decoding :path header as UTF-8: {}", e);
                    break;
                }
            };

            // Strip leading /
            if value.starts_with("/") {
                value = &value[1..];
            }
            // Remove index.html
            filename.pop();
            // Add requested path to absolute webroot path
            filename.push(value);
            // Stop processing headers.
            break;
        }
    }

    let filename = &filename.to_string_lossy();

    let mut response = match &srv.cache {
        Some(cache) => {
            let cache = Arc::clone(&cache);
            handle_cache_entry(cache.get(filename))
        }
        None => mcache::file_response(filename).0,
    };

    response.stream_id = req.stream_id;
    response
}

/// handle_cache_entry performs a cache get and unwraps the Response.
fn handle_cache_entry((entry, found): (Entry, bool)) -> Response {
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
