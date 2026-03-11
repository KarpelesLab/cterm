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

/// GitHub repository for downloading ctermd releases
#[cfg(unix)]
const GITHUB_REPO: &str = "KarpelesLab/cterm";

/// Generate a shell script that finds ctermd on the remote host, installs it
/// if needed, starts the daemon, and prints the socket path on stdout.
///
/// The script:
/// 1. Checks if `ctermd` is in PATH or at `~/.local/bin/ctermd`
/// 2. If not found, detects the platform and downloads the latest release
/// 3. Starts the daemon (daemonizes, returns immediately)
/// 4. Prints the socket path
#[cfg(unix)]
fn remote_setup_script() -> String {
    format!(
        r#"set -e
CTERMD=""
if command -v ctermd >/dev/null 2>&1; then
  CTERMD=$(command -v ctermd)
elif [ -x "$HOME/.local/bin/ctermd" ]; then
  CTERMD="$HOME/.local/bin/ctermd"
fi
if [ -z "$CTERMD" ]; then
  ARCH=$(uname -m)
  case "$(uname -s)" in
    Linux) case "$ARCH" in
      x86_64) ASSET=ctermd-linux-x86_64;;
      aarch64) ASSET=ctermd-linux-arm64;;
      *) echo "Unsupported architecture: $ARCH" >&2; exit 1;; esac;;
    Darwin) ASSET=ctermd-macos-universal;;
    *) echo "Unsupported OS: $(uname -s)" >&2; exit 1;;
  esac
  mkdir -p "$HOME/.local/bin"
  URL="https://github.com/{repo}/releases/latest/download/$ASSET.tar.gz"
  echo "Installing ctermd from $URL" >&2
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL "$URL" | tar xzf - --strip-components=1 -C "$HOME/.local/bin" "$ASSET/ctermd"
  elif command -v wget >/dev/null 2>&1; then
    wget -qO- "$URL" | tar xzf - --strip-components=1 -C "$HOME/.local/bin" "$ASSET/ctermd"
  else
    echo "curl or wget required to install ctermd" >&2; exit 1
  fi
  chmod +x "$HOME/.local/bin/ctermd"
  CTERMD="$HOME/.local/bin/ctermd"
  echo "Installed ctermd to $CTERMD" >&2
fi
"$CTERMD" >/dev/null 2>&1 || true
"$CTERMD" --print-socket-path"#,
        repo = GITHUB_REPO
    )
}

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
    /// Connect to the local ctermd, auto-starting if needed.
    ///
    /// On Unix, connects via Unix socket. On Windows, connects via named pipe.
    pub async fn connect_local() -> Result<Self> {
        let socket_path = socket::default_socket_path();
        Self::connect_unix(&socket_path, true).await
    }

    /// Connect to ctermd via a specific socket/pipe path.
    ///
    /// On Unix, `socket_path` is a Unix socket path.
    /// On Windows, `socket_path` is a named pipe path (e.g., `\\.\pipe\ctermd-user`).
    /// If `auto_start` is true, spawn ctermd if not already running.
    pub async fn connect_unix(socket_path: &Path, auto_start: bool) -> Result<Self> {
        // Try connecting first
        match Self::try_connect(socket_path).await {
            Ok(conn) => Ok(conn),
            Err(_) if auto_start => {
                // Try to start the daemon
                Self::start_daemon(socket_path)?;
                // Retry connection with backoff
                for i in 0..20 {
                    tokio::time::sleep(std::time::Duration::from_millis(100 * (i + 1))).await;
                    if let Ok(conn) = Self::try_connect(socket_path).await {
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

    /// Connect to a remote ctermd via SSH socket forwarding.
    ///
    /// This:
    /// 1. Finds ctermd on the remote host, auto-installing from GitHub releases if needed
    /// 2. Starts the daemon if not already running
    /// 3. Sets up SSH local forwarding (`-L`) to tunnel the remote socket locally
    /// 4. Connects the gRPC client to the local forwarded socket
    ///
    /// Auto-install detects the remote platform via `uname` and downloads
    /// the appropriate ctermd binary from the latest GitHub release.
    ///
    /// Because ctermd runs as a daemon on the remote with its own Unix socket,
    /// sessions survive SSH disconnects and can be reattached.
    ///
    /// The `host` parameter can be `user@hostname` or just `hostname`.
    #[cfg(unix)]
    pub async fn connect_ssh(host: &str) -> Result<Self> {
        use tokio::process::Command as TokioCommand;

        log::info!("Connecting to {} via SSH", host);

        // 1. Single SSH command: find/install ctermd, start daemon, print socket path
        let script = remote_setup_script();
        let output = TokioCommand::new("ssh")
            .args([host, &script])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await
            .map_err(|e| ClientError::Connection(format!("Failed to run ssh: {}", e)))?;

        // Log any stderr (install progress, warnings, etc.)
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.is_empty() {
            for line in stderr.trim().lines() {
                log::info!("Remote [{}]: {}", host, line);
            }
        }

        if !output.status.success() {
            return Err(ClientError::Connection(format!(
                "Failed to set up ctermd on {}: {}",
                host,
                stderr.trim()
            )));
        }

        let remote_socket = String::from_utf8_lossy(&output.stdout)
            .lines()
            .last()
            .unwrap_or("")
            .trim()
            .to_string();

        if remote_socket.is_empty() {
            return Err(ClientError::Connection(
                "Empty socket path from remote ctermd".to_string(),
            ));
        }
        log::info!("Remote socket path: {}", remote_socket);

        // Give daemon a moment to fully start and create the socket
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;

        // 2. Create a local temp socket for forwarding
        let local_socket = Self::ssh_forward_socket_path(host);

        // Clean up stale local socket from previous connection
        if local_socket.exists() {
            let _ = std::fs::remove_file(&local_socket);
        }

        // Ensure parent directory exists
        if let Some(parent) = local_socket.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        // 3. Start SSH tunnel: forward remote Unix socket to local Unix socket
        let forward_spec = format!("{}:{}", local_socket.display(), remote_socket);

        log::info!("Starting SSH tunnel: -L {}", forward_spec);

        let tunnel = TokioCommand::new("ssh")
            .args([
                "-N", // No remote command
                "-o",
                "ExitOnForwardFailure=yes",
                "-o",
                "StreamLocalBindUnlink=yes",
                "-L",
                &forward_spec,
                host,
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| ClientError::Connection(format!("Failed to start SSH tunnel: {}", e)))?;

        // Wait for the local socket to appear (tunnel is establishing)
        for i in 0..30 {
            if local_socket.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100 * (i / 5 + 1))).await;
        }

        if !local_socket.exists() {
            return Err(ClientError::Connection(format!(
                "SSH tunnel failed to create local socket at {}",
                local_socket.display()
            )));
        }

        // 4. Connect to the forwarded socket
        let conn = Self::try_connect_unix(&local_socket).await?;

        // Keep the tunnel process alive in the background
        tokio::spawn(async move {
            let local_socket_cleanup = local_socket;
            match tunnel.wait_with_output().await {
                Ok(output) => {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    if !stderr.is_empty() {
                        log::warn!("SSH tunnel stderr: {}", stderr.trim());
                    }
                    log::info!("SSH tunnel exited: {}", output.status);
                }
                Err(e) => log::error!("SSH tunnel error: {}", e),
            }
            // Clean up local socket
            let _ = std::fs::remove_file(&local_socket_cleanup);
        });

        Ok(conn)
    }

    /// Get the local socket path used for SSH forwarding to a given host
    #[cfg(unix)]
    fn ssh_forward_socket_path(host: &str) -> PathBuf {
        // Sanitize hostname for use in path
        let safe_host: String = host
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '.' {
                    c
                } else {
                    '_'
                }
            })
            .collect();

        let mut path = socket::default_socket_path();
        path.set_file_name(format!("ctermd-ssh-{}.sock", safe_host));
        path
    }

    /// Try to connect to the daemon at the given path (platform-dispatched).
    async fn try_connect(socket_path: &Path) -> Result<Self> {
        #[cfg(unix)]
        {
            Self::try_connect_unix(socket_path).await
        }
        #[cfg(windows)]
        {
            Self::try_connect_named_pipe(socket_path).await
        }
    }

    /// Try to connect to an existing Unix socket
    #[cfg(unix)]
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

    /// Try to connect to an existing named pipe (Windows)
    #[cfg(windows)]
    async fn try_connect_named_pipe(pipe_path: &Path) -> Result<Self> {
        let pipe_name = pipe_path.to_string_lossy().to_string();
        let channel = tonic::transport::Endpoint::try_from("http://[::]:0")
            .map_err(|e| ClientError::Connection(e.to_string()))?
            .connect_with_connector(tower::service_fn(move |_: tonic::transport::Uri| {
                let name = pipe_name.clone();
                async move {
                    let client =
                        tokio::net::windows::named_pipe::ClientOptions::new().open(&name)?;
                    Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(client))
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
            .args(["--listen", &socket_path.to_string_lossy(), "--foreground"])
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
        #[cfg(unix)]
        let which_cmd = "which";
        #[cfg(windows)]
        let which_cmd = "where";
        if let Ok(output) = Command::new(which_cmd).arg("ctermd").output() {
            if output.status.success() {
                let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
                // `where` on Windows may return multiple lines; take the first
                let path = path.lines().next().unwrap_or("").trim();
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

    /// Request daemon relaunch (exec-in-place, preserving PTY FDs).
    ///
    /// If `binary_path` is empty, the daemon re-execs the current binary.
    /// The connection will be dropped when the daemon execs — callers
    /// should reconnect after a brief delay.
    pub async fn relaunch_daemon(&self, binary_path: &str) -> Result<RelaunchDaemonResponse> {
        let response = self
            .client
            .lock()
            .await
            .relaunch_daemon(RelaunchDaemonRequest {
                binary_path: binary_path.to_string(),
            })
            .await?;

        Ok(response.into_inner())
    }
}
