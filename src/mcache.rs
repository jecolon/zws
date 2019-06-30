use std::collections::HashMap;
use std::fs;
use std::hash::BuildHasherDefault;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Condvar, Mutex, RwLock};
use std::{io, str, thread, time};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use seahash::SeaHasher;
use solicit::http::Response;

/// Entry is a cache entry that can fetch its Response lazily.
pub type Entry = Arc<(Mutex<bool>, Condvar, RwLock<Option<Response>>)>;

/// BuildHasher lets us use SeaHasher with HashMap.
type BuildHasher = BuildHasherDefault<SeaHasher>;

pub struct Cache {
    pub webroot: PathBuf,
    store: RwLock<HashMap<String, Entry, BuildHasher>>,
}

impl Cache {
    pub fn new(webroot: PathBuf) -> Arc<Cache> {
        debug!(
            "new: starting file response cache for webroot: {:?}",
            webroot
        );
        let cache_1 = Arc::new(Cache {
            webroot: webroot,
            store: RwLock::new(HashMap::<String, Entry, BuildHasher>::default()),
        });
        let cache_2 = Arc::clone(&cache_1);
        thread::spawn(move || watch(cache_2));
        cache_1
    }

    pub fn get(self: Arc<Self>, key: &str) -> (Entry, bool) {
        if let Some(value) = self.store.read().unwrap().get(key) {
            debug!("get: cache hit for {}", key);
            return (Arc::clone(&value), true);
        }

        debug!("get: cache miss for {}", key);
        let value = Arc::new((Mutex::new(false), Condvar::new(), RwLock::new(None)));
        let clone = Arc::clone(&value);
        self.store.write().unwrap().insert(key.to_string(), clone);
        let clone = Arc::clone(&value);
        let k = key.to_string();
        let c = Arc::clone(&self);

        thread::spawn(move || {
            let &(ref mtx, ref cnd, ref rwl) = &*clone;
            let mut rwl_guard = rwl.write().unwrap();
            let mut guard = mtx.lock().unwrap();
            let (resp, err) = file_response(&c.webroot, &k);
            if err {
                c.store.write().unwrap().remove(&k);
            }
            *rwl_guard = Some(resp);
            *guard = true;
            cnd.notify_all();
        });

        thread::sleep(time::Duration::from_millis(0));
        (Arc::clone(&value), false)
    }

    pub fn del(&self, key: &str) {
        debug!("del: removing cache entry with key: {}", key);
        self.store.write().unwrap().remove(key);
    }

    pub fn put(&self, key: &str, value: Entry) {
        debug!("put: inserting cache entry with key: {}", key);
        self.store.write().unwrap().insert(key.to_string(), value);
    }
}

/// file_response produces a response for the given filename.
pub fn file_response(webroot: &PathBuf, filename: &str) -> (Response, bool) {
    let path = Path::new(&filename);
    if path.is_dir() {
        let webroot_len = webroot.to_string_lossy().len() + 1;
        let redirect = format!("{}/index.html", &filename[webroot_len..]).into_bytes();
        debug!(
            "file_response: redirecting dir request without trailing slash to {}",
            str::from_utf8(&redirect).unwrap()
        );
        return (
            Response {
                headers: vec![
                    (b":status".to_vec(), b"307".to_vec()),
                    (b"location".to_vec(), redirect),
                ],
                body: b"Moved Termporarily\n".to_vec(),
                stream_id: 0,
            },
            true,
        );
    }

    let buf = match fs::read(path) {
        Ok(buf) => buf,
        Err(e) => {
            eprintln!("error reading file {}: {}", filename, e);
            if io::ErrorKind::NotFound == e.kind() {
                return (
                    Response {
                        headers: vec![(b":status".to_vec(), b"404".to_vec())],
                        body: b"Not Found\n".to_vec(),
                        stream_id: 0,
                    },
                    true,
                );
            }
            return (
                Response {
                    headers: vec![(b":status".to_vec(), b"500".to_vec())],
                    body: b"Unable to read file\n".to_vec(),
                    stream_id: 0,
                },
                true,
            );
        }
    };

    let ctype = get_ctype(filename);

    (
        Response {
            headers: vec![
                (b":status".to_vec(), b"200".to_vec()),
                (b"content-type".to_vec(), ctype.as_bytes().to_vec()),
            ],
            body: buf,
            stream_id: 0,
        },
        false,
    )
}

/// get_ctype produces a MIME content type string based on filename extension.
pub fn get_ctype(filename: &str) -> &str {
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

/// watch is a file system even processor that maintains the cache up-to-date.
fn watch(cache: Arc<Cache>) -> notify::Result<()> {
    debug!("watch: watching FS at {:?}", &cache.webroot);
    // Create a channel to receive the events.
    let (tx, rx) = mpsc::channel();

    // Automatically select the best implementation for your platform.
    // You can also access each implementation directly e.g. INotifyWatcher.
    let mut watcher: RecommendedWatcher = Watcher::new(tx, time::Duration::from_secs(2))?;

    // Add a path to be watched. All files and directories at that path and
    // below will be monitored for changes.
    watcher.watch(cache.webroot.to_str().unwrap(), RecursiveMode::Recursive)?;

    // This is a simple loop, but you may want to use more complex logic here,
    // for example to handle I/O.
    loop {
        match rx.recv() {
            Ok(event) => match event {
                notify::DebouncedEvent::Write(path) | notify::DebouncedEvent::Remove(path) => {
                    debug!("watch: FS event write or remove for {:?}", &path);
                    cache.del(&path.to_string_lossy());
                }
                notify::DebouncedEvent::Rename(path, _) => {
                    debug!("watch: FS event rename for {:?}", &path);
                    cache.del(&path.to_string_lossy());
                }
                _ => continue,
            },
            Err(e) => println!("watch error: {:?}", e),
        }
    }
}
