use std::thread;

use crossbeam::{channel, Receiver, Sender};

trait FnBox {
    fn call_box(self: Box<Self>);
}

impl<F: FnOnce()> FnBox for F {
    fn call_box(self: Box<F>) {
        (*self)()
    }
}

type Job = Box<dyn FnBox + Send + Sync + 'static>;

struct Worker {
    id: usize,
    handle: Option<thread::JoinHandle<()>>,
}

impl Worker {
    fn new(id: usize, receiver: Receiver<Job>) -> Worker {
        let handle = thread::spawn(move || {
            for job in receiver.iter() {
                debug!("worker {} executing job", id);
                job.call_box();
            }
        });

        Worker {
            id,
            handle: Some(handle),
        }
    }
}

pub struct Pool {
    sender: Option<Sender<Job>>,
    workers: Vec<Worker>,
}

impl Pool {
    pub fn new(size: usize) -> Pool {
        let (sender, receiver) = channel::unbounded();
        let mut workers = Vec::with_capacity(size);

        for id in 0..size {
            workers.push(Worker::new(id, receiver.clone()));
        }

        debug!("Created thread pool with {} worker threads", size);
        Pool {
            sender: Some(sender),
            workers: workers,
        }
    }

    pub fn execute<F>(&self, f: F)
    where
        F: FnOnce() + Send + Sync + 'static,
    {
        self.sender.as_ref().unwrap().send(Box::new(f)).unwrap();
    }
}

impl Drop for Pool {
    fn drop(&mut self) {
        debug!("Pool shutting down...");
        drop(self.sender.take().unwrap());
        for worker in &mut self.workers {
            if let Some(handle) = worker.handle.take() {
                debug!("Waiting for worker {} to stop...", worker.id);
                handle.join().unwrap();
            }
        }
    }
}
