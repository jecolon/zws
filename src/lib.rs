#[macro_use]
extern crate log;

pub mod error;
pub mod handlers;
pub mod request;
pub mod response;
pub mod server;
pub mod tls;

pub use error::Result;
pub use handlers::{Handler, StaticFile};
pub use request::{Action, Request};
pub use response::Response;
pub use server::Server;

#[cfg(test)]
mod tests {

    #[test]
    fn it_workds() {
        assert_eq!(4, 2 + 2);
    }
}
