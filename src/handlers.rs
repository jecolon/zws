use std::str;
use std::sync::Arc;

use crate::error::Result;
use crate::mcache::{self, Cache, Entry};
use crate::request::Request;
use crate::response::Response;

// Handler is a type that produces a Response for a given Request. The handle
// method consumes the handler.
pub trait Handler: Send + Sync {
    fn handle(&self, req: Request, resp: Response) -> Response;
}

#[derive(Clone)]
pub struct StaticFile<'a> {
    cache: Option<Arc<Cache>>,
    webroot: &'a str,
}

impl<'a> StaticFile<'a> {
    pub fn new(webroot: &str, caching: bool) -> Result<Box<StaticFile>> {
        let mut sf = StaticFile {
            cache: None,
            webroot: webroot,
        };

        if caching {
            sf.cache = Some(Cache::new(sf.webroot.clone()));
        }

        Ok(Box::new(sf))
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

impl<'a> Handler for StaticFile<'a> {
    fn handle(&self, req: Request, _resp: Response) -> Response {
        debug!("FileHandler: path is {}", &req.path);
        let filename = format!("{}{}", self.webroot, &req.path);
        debug!("FileHandler: filename is {}", &filename);

        let mut response = match &self.cache {
            Some(cache) => {
                let cache = Arc::clone(&cache);
                self.handle_cache_entry(cache.get(&filename))
            }
            None => mcache::file_response(self.webroot, &filename).0,
        };

        response.stream_id(req.stream_id);
        response
    }
}

/// NotFound is a handler that always returns a 404 Not Found Response.
#[derive(Clone)]
pub struct NotFound;

impl Handler for NotFound {
    fn handle(&self, _req: Request, mut resp: Response) -> Response {
        resp.header(":status", "404");
        resp.body("Not Found\n");
        resp
    }
}
