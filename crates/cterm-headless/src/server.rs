//! gRPC server setup for Unix socket and TCP

use crate::proto::terminal_service_server::TerminalServiceServer;
use crate::service::TerminalServiceImpl;
use crate::session::SessionManager;
#[cfg(unix)]
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Notify;
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
        }
    }
}

/// Run the gRPC server with the given configuration
pub async fn run_server(
    config: ServerConfig,
    relaunch_state_path: Option<String>,
) -> anyhow::Result<()> {
    // Write PID file
    let pid_path = crate::cli::pid_file_path();
    let pid = std::process::id();
    if let Err(e) = std::fs::write(&pid_path, pid.to_string()) {
        log::warn!("Failed to write PID file {}: {}", pid_path.display(), e);
    }

    let session_manager = Arc::new(SessionManager::with_scrollback(config.scrollback_lines));

    // Restore sessions from relaunch state if provided
    #[cfg(unix)]
    if let Some(ref state_path) = relaunch_state_path {
        match crate::relaunch::read_relaunch_state(state_path) {
            Ok((state, state_dir)) => {
                log::info!(
                    "Restoring {} sessions from relaunch state",
                    state.sessions.len()
                );
                for s in &state.sessions {
                    match unsafe {
                        session_manager.restore_session(
                            s.session_id.clone(),
                            s.master_fd,
                            s.child_pid,
                            s.cols,
                            s.rows,
                            s.custom_title.clone(),
                            s.tab_color.clone(),
                            s.template_name.clone(),
                            s.scrollback_lines,
                        )
                    } {
                        Ok(session) => {
                            // Apply screen snapshot from binary file
                            if let Some(screen_data) =
                                crate::relaunch::read_screen_snapshot(&state_dir, &s.session_id)
                            {
                                session.with_terminal_mut(|term| {
                                    cterm_proto::convert::screen::apply_screen_snapshot(
                                        term,
                                        &screen_data,
                                    );
                                });
                                log::info!("Applied screen snapshot for session {}", s.session_id);
                            }
                        }
                        Err(e) => {
                            log::error!(
                                "Failed to restore session {} (fd={}, pid={}): {}",
                                s.session_id,
                                s.master_fd,
                                s.child_pid,
                                e
                            );
                        }
                    }
                }
                // Clean up the state directory
                crate::relaunch::cleanup_state_dir(&state_dir);
                log::info!(
                    "Restored {}/{} sessions",
                    session_manager.session_count(),
                    state.sessions.len()
                );
            }
            Err(e) => {
                log::error!("Failed to read relaunch state: {}", e);
            }
        }
    }

    #[cfg(not(unix))]
    if relaunch_state_path.is_some() {
        log::warn!("Relaunch state is only supported on Unix, ignoring");
    }

    let shutdown_notify = Arc::new(Notify::new());
    let mut service =
        TerminalServiceImpl::new(session_manager.clone(), Arc::clone(&shutdown_notify));
    service.set_server_config(config.socket_path.clone(), config.scrollback_lines);

    // Spawn periodic dead session cleanup task
    {
        let sm = session_manager.clone();
        let shutdown = Arc::clone(&shutdown_notify);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                interval.tick().await;
                let cleaned = sm.cleanup_dead_sessions();
                // If dead sessions were cleaned and none remain, check for auto-shutdown.
                // The stream drop callback handles the connected-client check;
                // this handles the case where sessions exited but no streams were active.
                if cleaned > 0 && sm.session_count() == 0 && sm.had_sessions() {
                    log::info!("All sessions exited, shutting down daemon");
                    shutdown.notify_one();
                    break;
                }
            }
        });
    }

    let result = if config.use_tcp {
        run_tcp_server(config, service, shutdown_notify).await
    } else {
        #[cfg(unix)]
        {
            run_unix_socket_server(config, service, shutdown_notify).await
        }
        #[cfg(not(unix))]
        {
            log::warn!("Unix sockets not supported on this platform, falling back to TCP");
            run_tcp_server(config, service, shutdown_notify).await
        }
    };

    // Clean up PID file on exit
    let _ = std::fs::remove_file(&pid_path);

    result
}

/// Run the server on a TCP socket
async fn run_tcp_server(
    config: ServerConfig,
    service: TerminalServiceImpl,
    shutdown_notify: Arc<Notify>,
) -> anyhow::Result<()> {
    let addr = format!("{}:{}", config.bind_addr, config.port).parse()?;

    log::info!("Starting ctermd on TCP {}", addr);

    let shutdown = async move {
        let ctrl_c = tokio::signal::ctrl_c();

        #[cfg(unix)]
        {
            let mut sigterm =
                tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                    .expect("failed to register SIGTERM handler");
            tokio::select! {
                _ = ctrl_c => log::info!("Received SIGINT"),
                _ = sigterm.recv() => log::info!("Received SIGTERM"),
                _ = shutdown_notify.notified() => log::info!("Shutdown requested via RPC"),
            }
        }
        #[cfg(not(unix))]
        {
            tokio::select! {
                _ = ctrl_c => log::info!("Received SIGINT"),
                _ = shutdown_notify.notified() => log::info!("Shutdown requested via RPC"),
            }
        }
        log::info!("Shutting down...");
    };

    Server::builder()
        .add_service(
            TerminalServiceServer::new(service).max_encoding_message_size(64 * 1024 * 1024),
        )
        .serve_with_shutdown(addr, shutdown)
        .await?;

    Ok(())
}

/// Run the server on a Unix socket
#[cfg(unix)]
async fn run_unix_socket_server(
    config: ServerConfig,
    service: TerminalServiceImpl,
    shutdown_notify: Arc<Notify>,
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

    // Set up signal handler for graceful shutdown (SIGINT + SIGTERM + RPC shutdown)
    let shutdown = async move {
        let ctrl_c = tokio::signal::ctrl_c();
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to register SIGTERM handler");
        tokio::select! {
            _ = ctrl_c => log::info!("Received SIGINT"),
            _ = sigterm.recv() => log::info!("Received SIGTERM"),
            _ = shutdown_notify.notified() => log::info!("Shutdown requested via RPC"),
        }
        log::info!("Shutting down...");
    };

    let incoming = UnixListenerStream::new(listener);

    Server::builder()
        .add_service(
            TerminalServiceServer::new(service).max_encoding_message_size(64 * 1024 * 1024),
        )
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
