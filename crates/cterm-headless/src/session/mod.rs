//! Session management for ctermd

mod id;
mod manager;
mod state;

pub use id::generate_session_id;
pub use manager::SessionManager;
pub use state::{OutputData, SessionState};
