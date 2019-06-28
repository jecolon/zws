#[macro_use]
extern crate log;

pub mod error;
pub mod handlers;
pub mod mcache;
pub mod request;
pub mod server;
pub mod tls;

pub use error::Result;
pub use handlers::Handler;
pub use request::{Action, ServerRequest};
pub use server::{Server, ServerBuilder};

#[cfg(test)]
mod tests {

    #[test]
    fn it_workds() {
        assert_eq!(4, 2 + 2);
    }
}
