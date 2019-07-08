use std::collections::HashMap;
use std::path::Path;
use std::str;
use std::sync::{mpsc, Arc, RwLock};
use std::{fs, io, thread, time};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};

use crate::error::Result;
use crate::request::Request;
use crate::response::Response;

// Handler is a type that produces a Response for a given Request. The handle
// method consumes the handler.
pub trait Handler: Send + Sync {
    fn handle(&self, req: Request, resp: Response) -> Response;
}

type Cache = Arc<RwLock<HashMap<String, Response>>>;

#[derive(Clone)]
pub struct StaticFile {
    cache: Option<Cache>,
    webroot: String,
}

impl StaticFile {
    pub fn new(webroot: &str, caching: bool) -> Result<Box<StaticFile>> {
        let mut sf = StaticFile {
            cache: None,
            webroot: webroot.to_string(),
        };

        if caching {
            let cache = Arc::new(RwLock::new(HashMap::new()));
            let clone = Arc::clone(&cache);
            let wr = sf.webroot.clone();
            sf.cache = Some(cache);
            thread::spawn(|| watch_fs(clone, wr));
        }

        Ok(Box::new(sf))
    }
}

impl Handler for StaticFile {
    fn handle(&self, req: Request, _resp: Response) -> Response {
        debug!("FileHandler: path is {}", &req.path);
        let filename = format!("{}{}", self.webroot, &req.path);
        debug!("FileHandler: filename is {}", &filename);

        let mut response: Response;

        if let Some(cache) = &self.cache {
            let read_guard = cache.read().unwrap();
            if let Some(resp) = read_guard.get(&filename) {
                debug!("StaticFile: cache hit for {}", &filename);
                response = resp.clone();
            } else {
                debug!("StaticFile: cache miss for {}", &filename);
                drop(read_guard);
                let (resp, err) = file_response(&self.webroot, &filename);
                response = resp.clone();
                if !err {
                    cache.write().unwrap().insert(filename.clone(), resp);
                }
            }
        } else {
            response = file_response(&self.webroot, &filename).0;
        }

        response.stream_id(req.stream_id);
        response
    }
}

/// watch is a file system event processor that maintains the cache up-to-date.
fn watch_fs(cache: Cache, webroot: String) -> notify::Result<()> {
    debug!("watch: watching FS at {}", &webroot);
    // Create a channel to receive the events.
    let (tx, rx) = mpsc::channel();

    // Automatically select the best implementation for your platform.
    // You can also access each implementation directly e.g. INotifyWatcher.
    let mut watcher: RecommendedWatcher = Watcher::new(tx, time::Duration::from_secs(2))?;

    // Add a path to be watched. All files and directories at that path and
    // below will be monitored for changes.
    watcher.watch(&webroot, RecursiveMode::Recursive)?;

    // This is a simple loop, but you may want to use more complex logic here,
    // for example to handle I/O.
    let webroot_len = Path::new(&webroot)
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .len()
        - &webroot.len();
    loop {
        match rx.recv() {
            Ok(event) => match event {
                notify::DebouncedEvent::Write(path) | notify::DebouncedEvent::Remove(path) => {
                    let rel_path = &path.to_string_lossy()[webroot_len..];
                    debug!("watch: FS event write or remove for {}", rel_path);
                    cache.write().unwrap().remove(rel_path);
                }
                notify::DebouncedEvent::Rename(path, _) => {
                    let rel_path = &path.to_string_lossy()[webroot_len..];
                    debug!("watch: FS event rename for {}", rel_path);
                    cache.write().unwrap().remove(rel_path);
                }
                _ => continue,
            },
            Err(e) => println!("watch error: {:?}", e),
        }
    }
}

/// file_response produces a response for the given filename.
fn file_response(webroot: &str, filename: &str) -> (Response, bool) {
    let path = Path::new(&filename);
    if path.is_dir() {
        let webroot_len = webroot.len() + 1;
        let redirect = format!("{}/index.html", &filename[webroot_len..]);
        debug!(
            "file_response: redirecting dir request without trailing slash to {}",
            &redirect
        );
        let mut resp = Response::new(0);
        resp.header(":status", "307");
        resp.header("location", &redirect);
        resp.body("Moved Temporarily\n");
        return (resp, true);
    }

    let buf = match fs::read(path) {
        Ok(buf) => buf,
        Err(e) => {
            eprintln!("error reading file {}: {}", filename, e);
            if io::ErrorKind::NotFound == e.kind() {
                let mut resp = Response::new(0);
                resp.header(":status", "404");
                resp.body("Not Found\n");
                return (resp, true);
            }

            let mut resp = Response::new(0);
            resp.header(":status", "500");
            resp.body("Unable to read file\n");
            return (resp, true);
        }
    };

    let ctype = get_ctype(filename);

    let mut resp = Response::new(0);
    resp.header(":status", "200");
    resp.header("content-type", ctype);
    resp.body(buf);

    (resp, false)
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
