//! Claude over the Claude CLI's `stream-json` protocol and its control requests.

pub(crate) mod connection;
pub mod login;
pub mod permissions;
pub mod session;

pub use connection::CLIENT_APP;
