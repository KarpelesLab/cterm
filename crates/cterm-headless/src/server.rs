//! gRPC server setup for Unix socket and TCP

use crate::proto::terminal_service_server::TerminalServiceServer;
use crate::service::TerminalServiceImpl;
use crate::session::SessionManager;
#[cfg(unix)]
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncWrite};
use tonic::transport::Server;

/// Server configuration
pub struct ServerConfig {
    /// Use TCP instead of Unix socket
    pub use_tcp: bool,
    /// TCP bind address (default: 127.0.0.1)
    pub bind_addr: String,
    /// TCP port (default: 50051)
    pub port: u16,
    /// Unix socket path
    pub socket_path: String,
    /// Default scrollback lines for new sessions
    pub scrollback_lines: usize,
    /// Run in foreground (don't daemonize)
    pub foreground: bool,
    /// Run in stdio mode (gRPC over stdin/stdout)
    pub stdio: bool,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            use_tcp: false,
            bind_addr: "127.0.0.1".to_string(),
            port: 50051,
            socket_path: crate::cli::default_socket_path()
                .to_string_lossy()
                .to_string(),
            scrollback_lines: 10000,
            foreground: false,
            stdio: false,
        }
    }
}

/// Run the gRPC server with the given configuration
pub async fn run_server(config: ServerConfig) -> anyhow::Result<()> {
    // Write PID file (not in stdio mode)
    let pid_path = crate::cli::pid_file_path();
    if !config.stdio {
        let pid = std::process::id();
        if let Err(e) = std::fs::write(&pid_path, pid.to_string()) {
            log::warn!("Failed to write PID file {}: {}", pid_path.display(), e);
        }
    }

    let is_stdio = config.stdio;
    let session_manager = Arc::new(SessionManager::with_scrollback(config.scrollback_lines));
    let service = TerminalServiceImpl::new(session_manager);

    let result = if config.stdio {
        run_stdio_server(service).await
    } else if config.use_tcp {
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
    };

    // Clean up PID file on exit
    if !is_stdio {
        let _ = std::fs::remove_file(&pid_path);
    }

    result
}

/// Run the server in stdio mode (gRPC over stdin/stdout).
///
/// Used for SSH transport: `ssh user@host ctermd --stdio`
/// The gRPC protocol runs directly over the SSH channel's stdin/stdout.
async fn run_stdio_server(service: TerminalServiceImpl) -> anyhow::Result<()> {
    log::info!("Starting ctermd in stdio mode");

    // Create a combined stdin/stdout stream
    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let pipe = StdioPipe { stdin, stdout };

    // Create a one-shot stream that yields our single connection
    let incoming = futures::stream::once(async { Ok::<_, std::io::Error>(pipe) });

    Server::builder()
        .add_service(TerminalServiceServer::new(service))
        .serve_with_incoming(incoming)
        .await?;

    Ok(())
}

/// Combined stdin/stdout as a single AsyncRead + AsyncWrite stream.
///
/// Reads from stdin, writes to stdout. Used for stdio mode where gRPC
/// runs over a process pipe (e.g. SSH).
struct StdioPipe {
    stdin: tokio::io::Stdin,
    stdout: tokio::io::Stdout,
}

impl AsyncRead for StdioPipe {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.stdin).poll_read(cx, buf)
    }
}

impl AsyncWrite for StdioPipe {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.stdout).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.stdout).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.stdout).poll_shutdown(cx)
    }
}

impl tonic::transport::server::Connected for StdioPipe {
    type ConnectInfo = ();
    fn connect_info(&self) -> Self::ConnectInfo {}
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

    // Remove stale socket if present
    if socket_path.exists() {
        if is_socket_stale(socket_path) {
            log::info!("Removing stale socket: {}", socket_path.display());
            std::fs::remove_file(socket_path)?;
        } else {
            return Err(anyhow::anyhow!(
                "Socket {} already exists and daemon appears to be running",
                socket_path.display()
            ));
        }
    }

    // Ensure parent directory exists
    if let Some(parent) = socket_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let listener = UnixListener::bind(socket_path)?;

    // Set socket permissions to user-only
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(socket_path, std::fs::Permissions::from_mode(0o700)).ok();
    }

    log::info!("Starting ctermd on Unix socket {}", config.socket_path);

    // Set up signal handler for graceful shutdown (SIGINT + SIGTERM)
    let shutdown = async {
        let ctrl_c = tokio::signal::ctrl_c();
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to register SIGTERM handler");
        tokio::select! {
            _ = ctrl_c => log::info!("Received SIGINT"),
            _ = sigterm.recv() => log::info!("Received SIGTERM"),
        }
        log::info!("Shutting down...");
    };

    let incoming = UnixListenerStream::new(listener);

    Server::builder()
        .add_service(TerminalServiceServer::new(service))
        .serve_with_incoming_shutdown(incoming, shutdown)
        .await?;

    // Clean up socket file on exit
    log::info!("Cleaning up socket: {}", socket_path.display());
    let _ = std::fs::remove_file(socket_path);

    Ok(())
}

/// Check if a socket file is stale (no process using it)
#[cfg(unix)]
fn is_socket_stale(socket_path: &Path) -> bool {
    // Check PID file
    let mut pid_path = socket_path.to_path_buf();
    pid_path.set_extension("pid");

    if let Ok(contents) = std::fs::read_to_string(&pid_path) {
        if let Ok(pid) = contents.trim().parse::<i32>() {
            // Check if process is still running
            let result = unsafe { libc::kill(pid, 0) };
            if result == 0 {
                // Process exists — socket is not stale
                return false;
            }
            // Process doesn't exist — clean up PID file too
            let _ = std::fs::remove_file(&pid_path);
        }
    }

    // No PID file or process is gone — try to connect to confirm
    std::os::unix::net::UnixStream::connect(socket_path).is_err()
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
        assert!(config.socket_path.contains("ctermd"));
        assert_eq!(config.scrollback_lines, 10000);
        assert!(!config.foreground);
    }
}
