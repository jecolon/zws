#[macro_use]
extern crate log;

pub mod error;
pub mod mcache;
pub mod server;

pub use error::Result;
pub use server::{Action, Handler, Server, ServerBuilder, ServerRequest};

#[cfg(test)]
mod tests {

    #[test]
    fn it_workds() {
        assert_eq!(4, 2 + 2);
    }
}
