use std::sync::{mpsc, Arc, Mutex};
use std::thread::{self, JoinHandle};

trait FnBox {
    fn call_box(self: Box<Self>);
}

impl<F: FnOnce()> FnBox for F {
    fn call_box(self: Box<F>) {
        (*self)()
    }
}

type Job = Box<dyn FnBox + Send + Sync + 'static>;
type SharedReceiver = Arc<Mutex<mpsc::Receiver<Message>>>;

enum Message {
    NewJob(Job),
    Terminate,
}

struct Worker {
    id: usize,
    handle: Option<JoinHandle<()>>,
}

impl Worker {
    fn new(id: usize, receiver: SharedReceiver) -> Worker {
        let handle = thread::spawn(move || loop {
            let msg = receiver.lock().unwrap().recv().unwrap();
            match msg {
                Message::NewJob(job) => {
                    debug!("worker {} executing job", id);
                    job.call_box();
                }
                Message::Terminate => {
                    debug!("worker {} terminating", id);
                    break;
                }
            }
        });

        Worker {
            id,
            handle: Some(handle),
        }
    }
}

pub struct Pool {
    sender: mpsc::Sender<Message>,
    workers: Vec<Worker>,
}

impl Pool {
    pub fn new(size: usize) -> Pool {
        let (sender, receiver) = mpsc::channel();
        let receiver = Arc::new(Mutex::new(receiver));
        let mut workers = Vec::with_capacity(size);

        for id in 0..size {
            workers.push(Worker::new(id, Arc::clone(&receiver)));
        }

        debug!("Created thread pool with {} worker threads", size);
        Pool { sender, workers }
    }

    pub fn execute<F>(&self, f: F)
    where
        F: FnOnce() + Send + Sync + 'static,
    {
        self.sender.send(Message::NewJob(Box::new(f))).unwrap();
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        info!("Server shutting down...");
        for _ in &self.workers {
            self.sender.send(Message::Terminate).unwrap();
        }
        for worker in &mut self.workers {
            if let Some(handle) = worker.handle.take() {
                debug!("Waiting for worker {} to stop...", worker.id);
                handle.join().unwrap();
            }
        }
    }
}
