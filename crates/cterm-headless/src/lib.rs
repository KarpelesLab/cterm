//! cterm-headless: Headless terminal daemon with gRPC API
//!
//! This crate provides a headless terminal daemon (`ctermd`) that exposes
//! terminal functionality via gRPC. It supports multiple terminal sessions,
//! both Unix socket (default) and TCP transports.
//!
//! # Features
//!
//! - Multi-session terminal management
//! - gRPC API for session control, input/output, and screen state
//! - Unix socket (default) and TCP transport options
//! - Streaming output and event notifications
//!
//! # Usage
//!
//! ```bash
//! # Start with Unix socket (default)
//! ctermd
//!
//! # Start with TCP
//! ctermd --tcp --port 50051
//! ```

pub mod bridge;
pub mod cli;
pub mod convert;
pub mod error;
pub mod server;
pub mod service;
pub mod session;

/// Generated protobuf and gRPC code
pub mod proto {
    tonic::include_proto!("cterm.terminal");
}

pub use error::{HeadlessError, Result};
pub use server::{run_server, ServerConfig};
pub use service::TerminalServiceImpl;
pub use session::{SessionManager, SessionState};
