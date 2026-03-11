//! DaemonConnection - manages connection to a ctermd instance

use crate::error::{ClientError, Result};
use crate::session::SessionHandle;
use crate::socket;
use cterm_proto::proto::terminal_service_client::TerminalServiceClient;
use cterm_proto::proto::*;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use tokio::sync::Mutex;
use tonic::transport::Channel;

/// Information about the connected daemon
#[derive(Debug, Clone)]
pub struct DaemonInfo {
    pub daemon_id: String,
    pub daemon_version: String,
    pub hostname: String,
    pub is_local: bool,
}

/// Options for creating a new terminal session
#[derive(Default)]
pub struct CreateSessionOpts {
    pub cols: u32,
    pub rows: u32,
    pub shell: Option<String>,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub env: Vec<(String, String)>,
    pub term: Option<String>,
}

/// Connection to a ctermd instance
#[derive(Clone)]
pub struct DaemonConnection {
    client: Arc<Mutex<TerminalServiceClient<Channel>>>,
    info: Arc<DaemonInfo>,
}

impl DaemonConnection {
    /// Connect to the local ctermd via Unix socket, auto-starting if needed.
    pub async fn connect_local() -> Result<Self> {
        let socket_path = socket::default_socket_path();
        Self::connect_unix(&socket_path, true).await
    }

    /// Connect to ctermd via a specific Unix socket path.
    /// If `auto_start` is true, spawn ctermd if not already running.
    pub async fn connect_unix(socket_path: &Path, auto_start: bool) -> Result<Self> {
        // Try connecting first
        match Self::try_connect_unix(socket_path).await {
            Ok(conn) => Ok(conn),
            Err(_) if auto_start => {
                // Try to start the daemon
                Self::start_daemon(socket_path)?;
                // Retry connection with backoff
                for i in 0..20 {
                    tokio::time::sleep(std::time::Duration::from_millis(100 * (i + 1))).await;
                    if let Ok(conn) = Self::try_connect_unix(socket_path).await {
                        return Ok(conn);
                    }
                }
                Err(ClientError::DaemonNotRunning(
                    "Failed to connect after starting daemon".to_string(),
                ))
            }
            Err(e) => Err(e),
        }
    }

    /// Connect to ctermd via TCP (for testing or remote fallback).
    pub async fn connect_tcp(addr: &str) -> Result<Self> {
        let channel = Channel::from_shared(addr.to_string())
            .map_err(|e| ClientError::Connection(e.to_string()))?
            .connect()
            .await?;

        Self::handshake(channel).await
    }

    /// Connect to a remote ctermd via SSH.
    ///
    /// Spawns `ssh user@host ctermd --stdio` and uses stdin/stdout as the gRPC transport.
    /// The `host` parameter can be `user@hostname` or just `hostname`.
    pub async fn connect_ssh(host: &str) -> Result<Self> {
        use tokio::process::Command as TokioCommand;

        log::info!("Connecting to {} via SSH", host);

        // Check if ctermd is available on the remote host
        let check = TokioCommand::new("ssh")
            .args([host, "which", "ctermd"])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .status()
            .await
            .map_err(|e| ClientError::Connection(format!("Failed to run ssh: {}", e)))?;

        if !check.success() {
            return Err(ClientError::Connection(format!(
                "ctermd not found on remote host {}. Install it first.",
                host
            )));
        }

        // Spawn: ssh user@host ctermd --stdio
        let mut child = TokioCommand::new("ssh")
            .args([host, "ctermd", "--stdio"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| ClientError::Connection(format!("Failed to spawn ssh: {}", e)))?;

        let child_stdin = child
            .stdin
            .take()
            .ok_or_else(|| ClientError::Connection("Failed to capture ssh stdin".to_string()))?;
        let child_stdout = child
            .stdout
            .take()
            .ok_or_else(|| ClientError::Connection("Failed to capture ssh stdout".to_string()))?;

        // Wrap stdin/stdout as a combined AsyncRead+AsyncWrite
        let pipe = SshPipe {
            reader: child_stdout,
            writer: child_stdin,
        };

        // Connect tonic channel over the SSH pipe using a one-shot connector
        let pipe_cell = std::sync::Arc::new(tokio::sync::Mutex::new(Some(pipe)));
        let channel = tonic::transport::Endpoint::try_from("http://[::]:0")
            .map_err(|e| ClientError::Connection(e.to_string()))?
            .connect_with_connector(tower::service_fn(move |_: tonic::transport::Uri| {
                let pipe_cell = pipe_cell.clone();
                async move {
                    let pipe =
                        pipe_cell.lock().await.take().ok_or_else(|| {
                            std::io::Error::other("SSH connection already consumed")
                        })?;
                    Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(pipe))
                }
            }))
            .await?;

        // Spawn a task to wait for the child process and log when it exits
        tokio::spawn(async move {
            match child.wait().await {
                Ok(status) => log::info!("SSH process exited: {}", status),
                Err(e) => log::error!("Failed to wait for SSH process: {}", e),
            }
        });

        let mut conn = Self::handshake(channel).await?;
        // Override is_local since this is a remote connection
        if let Some(info) = Arc::get_mut(&mut conn.info) {
            info.is_local = false;
        }
        Ok(conn)
    }

    /// Try to connect to an existing Unix socket
    async fn try_connect_unix(socket_path: &Path) -> Result<Self> {
        if !socket_path.exists() {
            return Err(ClientError::Connection(format!(
                "Socket not found: {}",
                socket_path.display()
            )));
        }

        let socket_path = socket_path.to_owned();
        let channel = tonic::transport::Endpoint::try_from("http://[::]:0")
            .map_err(|e| ClientError::Connection(e.to_string()))?
            .connect_with_connector(tower::service_fn(move |_: tonic::transport::Uri| {
                let path = socket_path.clone();
                async move {
                    let stream = tokio::net::UnixStream::connect(path).await?;
                    Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(stream))
                }
            }))
            .await?;

        Self::handshake(channel).await
    }

    /// Perform the initial handshake with the daemon
    async fn handshake(channel: Channel) -> Result<Self> {
        let mut client = TerminalServiceClient::new(channel);

        let response = client
            .handshake(HandshakeRequest {
                client_id: uuid::Uuid::new_v4().to_string(),
                client_version: env!("CARGO_PKG_VERSION").to_string(),
                protocol_version: 1,
            })
            .await?;

        let resp = response.into_inner();
        let info = DaemonInfo {
            daemon_id: resp.daemon_id,
            daemon_version: resp.daemon_version,
            hostname: resp.hostname,
            is_local: resp.is_local,
        };

        log::info!(
            "Connected to ctermd {} on {} (local={})",
            info.daemon_version,
            info.hostname,
            info.is_local
        );

        Ok(Self {
            client: Arc::new(Mutex::new(client)),
            info: Arc::new(info),
        })
    }

    /// Start a local ctermd daemon process
    fn start_daemon(socket_path: &Path) -> Result<()> {
        let ctermd = Self::find_ctermd()?;

        log::info!("Starting ctermd: {}", ctermd.display());

        Command::new(&ctermd)
            .args(["--listen", &socket_path.to_string_lossy()])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| {
                ClientError::DaemonNotRunning(format!(
                    "Failed to spawn {}: {}",
                    ctermd.display(),
                    e
                ))
            })?;

        Ok(())
    }

    /// Find the ctermd binary
    fn find_ctermd() -> Result<PathBuf> {
        // First: next to the current executable
        if let Ok(exe) = std::env::current_exe() {
            let dir = exe.parent().unwrap_or(Path::new("."));
            let candidate = dir.join("ctermd");
            if candidate.exists() {
                return Ok(candidate);
            }
            #[cfg(windows)]
            {
                let candidate = dir.join("ctermd.exe");
                if candidate.exists() {
                    return Ok(candidate);
                }
            }
        }

        // Second: in PATH
        if let Ok(output) = Command::new("which").arg("ctermd").output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !path.is_empty() {
                    return Ok(PathBuf::from(path));
                }
            }
        }

        Err(ClientError::DaemonNotRunning(
            "ctermd binary not found".to_string(),
        ))
    }

    /// Get information about the connected daemon
    pub fn info(&self) -> &DaemonInfo {
        &self.info
    }

    /// Create a new terminal session
    pub async fn create_session(&self, opts: CreateSessionOpts) -> Result<SessionHandle> {
        let response = self
            .client
            .lock()
            .await
            .create_session(CreateSessionRequest {
                cols: opts.cols,
                rows: opts.rows,
                shell: opts.shell,
                args: opts.args,
                cwd: opts.cwd,
                env: opts.env.into_iter().collect(),
                term: opts.term,
            })
            .await?;

        let resp = response.into_inner();
        Ok(SessionHandle::new(
            resp.session_id,
            self.client.clone(),
            self.info.clone(),
        ))
    }

    /// List all sessions on this daemon
    pub async fn list_sessions(&self) -> Result<Vec<SessionInfo>> {
        let response = self
            .client
            .lock()
            .await
            .list_sessions(ListSessionsRequest {})
            .await?;

        Ok(response.into_inner().sessions)
    }

    /// Attach to an existing session by ID
    pub async fn attach_session(
        &self,
        session_id: &str,
        cols: u32,
        rows: u32,
    ) -> Result<(SessionHandle, Option<GetScreenResponse>)> {
        let response = self
            .client
            .lock()
            .await
            .attach_session(AttachSessionRequest {
                session_id: session_id.to_string(),
                cols,
                rows,
                want_screen_snapshot: true,
            })
            .await?;

        let resp = response.into_inner();
        let handle = SessionHandle::new(
            session_id.to_string(),
            self.client.clone(),
            self.info.clone(),
        );

        Ok((handle, resp.initial_screen))
    }

    /// Get daemon info
    pub async fn get_daemon_info(&self) -> Result<GetDaemonInfoResponse> {
        let response = self
            .client
            .lock()
            .await
            .get_daemon_info(GetDaemonInfoRequest {})
            .await?;

        Ok(response.into_inner())
    }

    /// Request daemon shutdown
    pub async fn shutdown(&self, force: bool) -> Result<ShutdownResponse> {
        let response = self
            .client
            .lock()
            .await
            .shutdown(ShutdownRequest { force })
            .await?;

        Ok(response.into_inner())
    }
}

/// Combined child stdout/stdin as a single AsyncRead + AsyncWrite stream.
///
/// Reads from child stdout, writes to child stdin. Used for SSH transport
/// where gRPC runs over a process pipe.
struct SshPipe {
    reader: tokio::process::ChildStdout,
    writer: tokio::process::ChildStdin,
}

impl tokio::io::AsyncRead for SshPipe {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.reader).poll_read(cx, buf)
    }
}

impl tokio::io::AsyncWrite for SshPipe {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        std::pin::Pin::new(&mut self.writer).poll_write(cx, buf)
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.writer).poll_flush(cx)
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::pin::Pin::new(&mut self.writer).poll_shutdown(cx)
    }
}
