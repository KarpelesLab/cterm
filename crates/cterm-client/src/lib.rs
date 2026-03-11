//! cterm-client: Client library for connecting to ctermd
//!
//! Provides `DaemonConnection` for connecting to a ctermd instance over Unix socket,
//! TCP, or SSH, and `SessionHandle` for interacting with individual terminal sessions.

mod connection;
mod error;
mod session;
mod socket;

pub use connection::{CreateSessionOpts, DaemonConnection};
pub use error::ClientError;
pub use session::SessionHandle;
pub use socket::{default_socket_path, pid_file_path};
