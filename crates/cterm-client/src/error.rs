//! Client error types

use thiserror::Error;

#[derive(Error, Debug)]
pub enum ClientError {
    #[error("Connection failed: {0}")]
    Connection(String),

    #[error("Daemon not running and auto-start failed: {0}")]
    DaemonNotRunning(String),

    #[error("Session not found: {0}")]
    SessionNotFound(String),

    #[error("gRPC error: {0}")]
    Grpc(Box<tonic::Status>),

    #[error("Transport error: {0}")]
    Transport(#[from] tonic::transport::Error),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Version mismatch: daemon={daemon}, client={client}")]
    VersionMismatch { daemon: String, client: String },
}

impl From<tonic::Status> for ClientError {
    fn from(status: tonic::Status) -> Self {
        ClientError::Grpc(Box::new(status))
    }
}

pub type Result<T> = std::result::Result<T, ClientError>;
