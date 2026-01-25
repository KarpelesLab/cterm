//! Error types for cterm-headless

use thiserror::Error;
use tonic::Status;

/// Errors that can occur in the headless terminal daemon
#[derive(Error, Debug)]
pub enum HeadlessError {
    #[error("Session not found: {0}")]
    SessionNotFound(String),

    #[error("Session already exists: {0}")]
    SessionAlreadyExists(String),

    #[error("PTY error: {0}")]
    Pty(#[from] cterm_core::PtyError),

    #[error("Invalid argument: {0}")]
    InvalidArgument(String),

    #[error("Internal error: {0}")]
    Internal(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

impl From<HeadlessError> for Status {
    fn from(err: HeadlessError) -> Self {
        match err {
            HeadlessError::SessionNotFound(id) => {
                Status::not_found(format!("Session not found: {}", id))
            }
            HeadlessError::SessionAlreadyExists(id) => {
                Status::already_exists(format!("Session already exists: {}", id))
            }
            HeadlessError::InvalidArgument(msg) => Status::invalid_argument(msg),
            HeadlessError::Pty(e) => Status::internal(format!("PTY error: {}", e)),
            HeadlessError::Internal(msg) => Status::internal(msg),
            HeadlessError::Io(e) => Status::internal(format!("IO error: {}", e)),
        }
    }
}

/// Result type for headless operations
pub type Result<T> = std::result::Result<T, HeadlessError>;
