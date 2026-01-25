//! gRPC server setup for Unix socket and TCP

use crate::proto::terminal_service_server::TerminalServiceServer;
use crate::service::TerminalServiceImpl;
use crate::session::SessionManager;
use std::path::Path;
use std::sync::Arc;
use tonic::transport::Server;

/// Server configuration
pub struct ServerConfig {
    /// Use TCP instead of Unix socket
    pub use_tcp: bool,
    /// TCP bind address (default: 127.0.0.1)
    pub bind_addr: String,
    /// TCP port (default: 50051)
    pub port: u16,
    /// Unix socket path (default: /tmp/ctermd.sock)
    pub socket_path: String,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            use_tcp: false,
            bind_addr: "127.0.0.1".to_string(),
            port: 50051,
            socket_path: "/tmp/ctermd.sock".to_string(),
        }
    }
}

/// Run the gRPC server with the given configuration
pub async fn run_server(config: ServerConfig) -> anyhow::Result<()> {
    let session_manager = Arc::new(SessionManager::new());
    let service = TerminalServiceImpl::new(session_manager);

    if config.use_tcp {
        run_tcp_server(config, service).await
    } else {
        #[cfg(unix)]
        {
            run_unix_socket_server(config, service).await
        }
        #[cfg(not(unix))]
        {
            log::warn!("Unix sockets not supported on this platform, falling back to TCP");
            run_tcp_server(config, service).await
        }
    }
}

/// Run the server on a TCP socket
async fn run_tcp_server(config: ServerConfig, service: TerminalServiceImpl) -> anyhow::Result<()> {
    let addr = format!("{}:{}", config.bind_addr, config.port).parse()?;

    log::info!("Starting ctermd on TCP {}", addr);

    Server::builder()
        .add_service(TerminalServiceServer::new(service))
        .serve(addr)
        .await?;

    Ok(())
}

/// Run the server on a Unix socket
#[cfg(unix)]
async fn run_unix_socket_server(
    config: ServerConfig,
    service: TerminalServiceImpl,
) -> anyhow::Result<()> {
    use tokio::net::UnixListener;
    use tokio_stream::wrappers::UnixListenerStream;

    let socket_path = Path::new(&config.socket_path);

    // Remove existing socket file if present
    if socket_path.exists() {
        std::fs::remove_file(socket_path)?;
    }

    let listener = UnixListener::bind(socket_path)?;

    log::info!("Starting ctermd on Unix socket {}", config.socket_path);

    let incoming = UnixListenerStream::new(listener);

    Server::builder()
        .add_service(TerminalServiceServer::new(service))
        .serve_with_incoming(incoming)
        .await?;

    // Clean up socket file on exit
    let _ = std::fs::remove_file(socket_path);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_server_config_default() {
        let config = ServerConfig::default();
        assert!(!config.use_tcp);
        assert_eq!(config.bind_addr, "127.0.0.1");
        assert_eq!(config.port, 50051);
        assert_eq!(config.socket_path, "/tmp/ctermd.sock");
    }
}
