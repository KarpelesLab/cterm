//! Mosh protocol implementation for cterm.
//!
//! Implements the mosh (mobile shell) protocol: AES-128-OCB3 encryption,
//! hand-rolled protobuf, fragment reassembly, and the SSP state machine.
//! Mosh hoststring data is raw terminal output that feeds directly into
//! `Terminal::process()`.

pub mod crypto;
pub mod fragment;
pub mod proto;
pub mod server_session;
pub mod session;
pub mod ssh_launch;
pub mod ssp;
pub mod ssp_server;
pub mod transport;

pub use session::MoshSession;

/// Connection type for mosh: direct or via relay.
#[derive(Debug, Clone, Default)]
pub enum MoshConnectionType {
    /// Direct SSH + UDP to the target host
    #[default]
    Direct,
    /// Via relay server (3-hop SSH tunnel)
    Relay {
        /// Relay host (e.g., "unixshells.com")
        relay_host: String,
        /// Jump host (e.g., "relay.unixshells.com")
        jump_host: String,
        /// Relay account username
        relay_username: String,
        /// Relay device name
        relay_device: String,
        /// Latch session name (e.g., "default")
        session_name: String,
    },
}

/// Configuration for a mosh connection.
#[derive(Debug, Clone)]
pub struct MoshConfig {
    /// SSH destination (user@hostname or hostname)
    pub host: String,
    /// Connection type: direct or relay
    pub connection_type: MoshConnectionType,
    /// SSH ProxyJump for direct connections (e.g., "bastion.example.com")
    pub proxy_jump: Option<String>,
    /// Terminal columns
    pub cols: u16,
    /// Terminal rows
    pub rows: u16,
    /// Locale to set on remote (e.g., "en_US.UTF-8")
    pub locale: Option<String>,
    /// TERM value to set on remote (e.g., "xterm-256color")
    pub term: Option<String>,
    /// Extra SSH arguments (e.g., ["-p", "2222"])
    pub ssh_args: Vec<String>,
}

/// Commands sent to the mosh session event loop.
#[derive(Debug)]
pub enum MoshCommand {
    /// Send keystrokes to the remote
    Write(Vec<u8>),
    /// Notify remote of terminal resize
    Resize(u16, u16),
    /// Shut down the session
    Shutdown,
}

/// Events received from the mosh session.
#[derive(Debug)]
pub enum MoshEvent {
    /// Terminal output data (hoststring) for `Terminal::process()`
    Output(Vec<u8>),
    /// Session has closed
    Closed(Option<MoshError>),
}

/// Mosh protocol errors.
#[derive(Debug, thiserror::Error)]
pub enum MoshError {
    #[error("invalid base64 key")]
    InvalidKey,
    #[error("encryption failed")]
    EncryptionFailed,
    #[error("decryption failed (bad key or tampered data)")]
    DecryptionFailed,
    #[error("datagram too short")]
    DatagramTooShort,
    #[error("wrong direction bit")]
    WrongDirection,
    #[error("fragment too short")]
    FragmentTooShort,
    #[error("decompression failed")]
    DecompressionFailed,
    #[error("compression failed")]
    CompressionFailed,
    #[error("protobuf decode error")]
    ProtobufError,
    #[error("SSH failed: {0}")]
    SshFailed(String),
    #[error("invalid MOSH CONNECT output")]
    InvalidMoshConnect,
    #[error("invalid address: {0}")]
    InvalidAddress(String),
    #[error("UDP bind failed: {0}")]
    UdpBindFailed(String),
    #[error("UDP connect failed: {0}")]
    UdpConnectFailed(String),
    #[error("UDP recv failed: {0}")]
    UdpRecvFailed(String),
}
