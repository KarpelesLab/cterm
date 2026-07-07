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

/// Max time to establish the HTTP/2 transport to the daemon. A wedged daemon can
/// accept the socket connection (the listen backlog is kernel-side) yet never
/// complete the HTTP/2 settings exchange, so this guards the transport handshake.
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(3);

/// Max time to wait for the daemon to answer the Handshake RPC. The handler is
/// trivial (no I/O), so a healthy daemon replies in milliseconds; a larger budget
/// only tolerates a cold/loaded daemon. Without this, a deadlocked daemon makes the
/// client hang forever at startup.
const HANDSHAKE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Parse an SSH host string, extracting an optional port suffix.
///
/// Accepts `user@host:port`, `host:port`, `user@host`, or `host`.
/// Returns the SSH destination (without port) and the port if present.
#[cfg(unix)]
fn parse_ssh_host(input: &str) -> (String, Option<u16>) {
    // Check for user@host:port or host:port
    // The port is the part after the last colon, but only if it parses as u16
    if let Some(colon_pos) = input.rfind(':') {
        let maybe_port = &input[colon_pos + 1..];
        if let Ok(port) = maybe_port.parse::<u16>() {
            let dest = &input[..colon_pos];
            return (dest.to_string(), Some(port));
        }
    }
    (input.to_string(), None)
}

/// Generate a shell script that finds ctermd on the remote host, installs it
/// if needed, starts the daemon, and prints the socket path on stdout.
///
/// The script:
/// 1. Checks if `ctermd` is in PATH or at `~/.local/bin/ctermd`
/// 2. If found, checks if daemon is already running (socket exists) — if so, just prints path
/// 3. If binary not found, detects the platform and downloads the latest release
/// 4. Starts the daemon (daemonizes, returns immediately)
/// 5. Prints the socket path
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
if [ -n "$CTERMD" ]; then
  SOCK=$("$CTERMD" --print-socket-path 2>/dev/null || echo "")
  if [ -n "$SOCK" ] && [ -S "$SOCK" ]; then
    echo "$SOCK"
    exit 0
  fi
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
    /// Socket path used for this connection (allows reconnecting from a different runtime).
    /// Set for Unix socket and SSH-tunneled connections; None for TCP.
    pub socket_path: Option<PathBuf>,
}

/// Handle for a native (puressh) SSH tunnel. Allows another part of the app
/// (typically `RemoteManager::disconnect`) to terminate the tunnel.
///
/// Cloning is cheap (Arc); calling `kill()` is idempotent. The tunnel is also
/// torn down when the last handle is dropped.
#[cfg(unix)]
#[derive(Clone, Default)]
pub struct SshTunnelHandle {
    tunnel: Option<Arc<cterm_core::SshTunnel>>,
    /// Registry key under which the tunnel is registered, so `kill` can
    /// deregister it (stopping reconnects from reaching a dead connection).
    key: Option<PathBuf>,
}

#[cfg(unix)]
impl SshTunnelHandle {
    /// Stop the tunnel and remove it from the registry. No-op if already gone.
    pub fn kill(&self) {
        if let Some(key) = &self.key {
            unregister_ssh_tunnel(key);
        }
        if let Some(tunnel) = &self.tunnel {
            tunnel.close();
        }
    }
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
    /// When set, the daemon opens a native SSH session instead of a local shell.
    pub ssh: Option<SshParams>,
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
        // SSH-tunneled connections have no socket file: if this path is a
        // registered SSH tunnel key, dial a fresh channel over the shared SSH
        // connection instead of a Unix socket.
        #[cfg(unix)]
        if let Some(opener) = ssh_opener_for(socket_path) {
            return Self::connect_via_ssh(socket_path, opener).await;
        }

        // Try connecting first
        match Self::try_connect(socket_path).await {
            Ok(conn) => Ok(conn),
            // A wedged daemon (socket exists and accepts connections, but never answers
            // the handshake) must NOT trigger auto-start: spawning a second ctermd would
            // just fail to bind the same socket and exit, and we'd keep retrying against
            // the wedged one. Surface it immediately so the UI can report it.
            Err(e @ ClientError::DaemonUnresponsive(_)) => Err(e),
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

        Self::handshake(channel, None).await
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
    /// When `compress` is true, SSH compression (`-C`) is enabled on the tunnel,
    /// which significantly reduces bandwidth for terminal data (scrollback, screen snapshots).
    #[cfg(unix)]
    pub async fn connect_ssh(host: &str, compress: bool) -> Result<(Self, SshTunnelHandle)> {
        Self::connect_ssh_with_prompts(host, compress, cterm_core::SshPrompts::default()).await
    }

    /// Like [`Self::connect_ssh`], but with interactive prompt callbacks for
    /// host-key verification, password, and key passphrase. Because the tunnel
    /// runs in the UI process, these can show native dialogs directly (unlike
    /// daemon-side SSH tabs, which round-trip prompts over gRPC).
    #[cfg(unix)]
    pub async fn connect_ssh_with_prompts(
        host: &str,
        compress: bool,
        prompts: cterm_core::SshPrompts,
    ) -> Result<(Self, SshTunnelHandle)> {
        log::info!("Connecting to {} via SSH (native puressh)", host);

        // Parse optional port and split user@host. A `>`-separated jump chain
        // (`bastion:2222>10.0.0.5`) is passed through whole: cterm-core parses
        // per-segment `[user@]host[:port]` itself, and splitting on the first
        // `@` here would corrupt chains like `a>user@b`.
        let (username, hostname, port) = if host.contains('>') {
            (None, host.to_string(), None)
        } else {
            let (ssh_dest, port) = parse_ssh_host(host);
            match ssh_dest.split_once('@') {
                Some((u, h)) => (Some(u.to_string()), h.to_string(), port),
                None => (None, ssh_dest, port),
            }
        };

        // The setup script finds/installs ctermd, starts the daemon, and prints
        // the remote socket path; we then talk gRPC directly over the SSH
        // connection — no local socket file is ever created.
        let script = remote_setup_script();
        // Synthetic key, carried through the app as this connection's
        // `socket_path`. Readers reconnect through it (see `connect_unix`);
        // nothing is written to this path.
        let tunnel_key = Self::ssh_tunnel_key(host);

        let ssh_config = cterm_core::SshConfig {
            host: hostname,
            port: port.unwrap_or(22),
            username,
            compress,
            host_key_prompt: prompts.host_key,
            password_prompt: prompts.password,
            passphrase_prompt: prompts.passphrase,
            ..Default::default()
        };

        // puressh is blocking and spawns its own pump thread; run the
        // connect+setup on a blocking task so we don't stall the reactor.
        let host_owned = host.to_string();
        let tunnel = tokio::task::spawn_blocking(move || {
            cterm_core::SshTunnel::connect(ssh_config, &script)
        })
        .await
        .map_err(|e| ClientError::Connection(format!("SSH tunnel task panicked: {e}")))?
        .map_err(|e| ClientError::Connection(format!("SSH tunnel to {host_owned}: {e}")))?;

        // Keep the SSH connection alive in a registry so reconnects (output
        // streams run in their own runtimes) can open fresh channels over it,
        // and so dropping the returned handle doesn't tear it down.
        let tunnel = Arc::new(tunnel);
        register_ssh_tunnel(tunnel_key.clone(), Arc::clone(&tunnel));

        // Dial the gRPC connection directly over the SSH channel.
        let conn = Self::connect_via_ssh(&tunnel_key, tunnel.opener()).await?;
        let tunnel_handle = SshTunnelHandle {
            tunnel: Some(tunnel),
            key: Some(tunnel_key),
        };
        Ok((conn, tunnel_handle))
    }

    /// Stable per-host key identifying an SSH tunnel in the registry. It reuses
    /// the daemon socket directory for a recognizable value but is never created
    /// as a file — it only flows through the app as a connection's `socket_path`.
    #[cfg(unix)]
    fn ssh_tunnel_key(host: &str) -> PathBuf {
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
        path.set_file_name(format!("ctermd-ssh-{}", safe_host));
        path
    }

    /// Build a gRPC connection that runs over a fresh `direct-streamlocal`
    /// channel on the shared SSH connection. `key` is recorded as the
    /// connection's `socket_path` so reconnects route back through the registry.
    #[cfg(unix)]
    async fn connect_via_ssh(key: &Path, opener: cterm_core::SshChannelOpener) -> Result<Self> {
        let endpoint = tonic::transport::Endpoint::try_from("http://[::]:0")
            .map_err(|e| ClientError::Connection(e.to_string()))?;
        let connect =
            endpoint.connect_with_connector(tower::service_fn(move |_: tonic::transport::Uri| {
                let opener = opener.clone();
                async move {
                    // Opening the channel is a quick round-trip to the serve
                    // loop (a separate thread); fine to do inline here.
                    let (reader, writer) = opener.open()?;
                    Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(ssh_channel_io(
                        reader, writer,
                    )))
                }
            }));
        let channel = tokio::time::timeout(CONNECT_TIMEOUT, connect)
            .await
            .map_err(|_| {
                ClientError::DaemonUnresponsive(format!(
                    "connect timed out after {}s",
                    CONNECT_TIMEOUT.as_secs()
                ))
            })??;
        Self::handshake(channel, Some(key.to_owned())).await
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

        let owned_path = socket_path.to_owned();
        let endpoint = tonic::transport::Endpoint::try_from("http://[::]:0")
            .map_err(|e| ClientError::Connection(e.to_string()))?;
        let connect =
            endpoint.connect_with_connector(tower::service_fn(move |_: tonic::transport::Uri| {
                let path = owned_path.clone();
                async move {
                    let stream = tokio::net::UnixStream::connect(path).await?;
                    Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(stream))
                }
            }));
        let channel = tokio::time::timeout(CONNECT_TIMEOUT, connect)
            .await
            .map_err(|_| {
                ClientError::DaemonUnresponsive(format!(
                    "connect timed out after {}s",
                    CONNECT_TIMEOUT.as_secs()
                ))
            })??;

        Self::handshake(channel, Some(socket_path.to_owned())).await
    }

    /// Try to connect to an existing named pipe (Windows)
    #[cfg(windows)]
    async fn try_connect_named_pipe(pipe_path: &Path) -> Result<Self> {
        let pipe_name = pipe_path.to_string_lossy().to_string();
        let endpoint = tonic::transport::Endpoint::try_from("http://[::]:0")
            .map_err(|e| ClientError::Connection(e.to_string()))?;
        let connect =
            endpoint.connect_with_connector(tower::service_fn(move |_: tonic::transport::Uri| {
                let name = pipe_name.clone();
                async move {
                    let client =
                        tokio::net::windows::named_pipe::ClientOptions::new().open(&name)?;
                    Ok::<_, std::io::Error>(hyper_util::rt::TokioIo::new(client))
                }
            }));
        let channel = tokio::time::timeout(CONNECT_TIMEOUT, connect)
            .await
            .map_err(|_| {
                ClientError::DaemonUnresponsive(format!(
                    "connect timed out after {}s",
                    CONNECT_TIMEOUT.as_secs()
                ))
            })??;
        Self::handshake(channel, Some(pipe_path.to_owned())).await
    }

    /// Perform the initial handshake with the daemon
    async fn handshake(channel: Channel, socket_path: Option<PathBuf>) -> Result<Self> {
        let mut client =
            TerminalServiceClient::new(channel).max_decoding_message_size(64 * 1024 * 1024);

        let response = tokio::time::timeout(
            HANDSHAKE_TIMEOUT,
            client.handshake(HandshakeRequest {
                client_id: uuid::Uuid::new_v4().to_string(),
                client_version: env!("CARGO_PKG_VERSION").to_string(),
                protocol_version: 1,
            }),
        )
        .await
        .map_err(|_| {
            ClientError::DaemonUnresponsive(format!(
                "handshake timed out after {}s",
                HANDSHAKE_TIMEOUT.as_secs()
            ))
        })??;

        let resp = response.into_inner();
        let info = DaemonInfo {
            daemon_id: resp.daemon_id,
            daemon_version: resp.daemon_version,
            hostname: resp.hostname,
            is_local: resp.is_local,
            socket_path,
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
                ssh: opts.ssh,
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
        // Clone the client out of the mutex so the RPC isn't held under the lock
        // for its whole duration — otherwise concurrent RPCs (e.g. attaching many
        // sessions at once during reconnect) would serialize behind each other.
        let mut client = self.client.lock().await.clone();
        let response = client.list_sessions(ListSessionsRequest {}).await?;

        Ok(response.into_inner().sessions)
    }

    /// Get info about a specific session
    pub async fn get_session(&self, session_id: &str) -> Result<SessionInfo> {
        let response = self
            .client
            .lock()
            .await
            .get_session(GetSessionRequest {
                session_id: session_id.to_string(),
            })
            .await?;

        response
            .into_inner()
            .session
            .ok_or_else(|| crate::error::ClientError::SessionNotFound(session_id.to_string()))
    }

    /// Attach to an existing session by ID
    pub async fn attach_session(
        &self,
        session_id: &str,
        cols: u32,
        rows: u32,
    ) -> Result<(SessionHandle, Option<GetScreenResponse>)> {
        // Clone the client so the RPC (which streams a full screen snapshot,
        // scrollback included) doesn't hold the connection mutex for its whole
        // duration. This lets `reconnect_all_sessions_on` attach every session
        // concurrently over the multiplexed HTTP/2 channel instead of serially.
        let mut client = self.client.lock().await.clone();
        let response = client
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

    /// Attach to a session without fetching a screen snapshot.
    ///
    /// Output-stream readers open their own connection purely to receive the
    /// PTY stream; the screen state was already fetched and applied by the
    /// reconnect that created the tab. Skipping the snapshot here avoids
    /// re-transferring the full scrollback (up to 10k lines) a second time per
    /// session. Passing `cols`/`rows` of 0 leaves the daemon-side size
    /// unchanged — the UI sends a real resize once the view is laid out, so we
    /// avoid a spurious reflow to a placeholder size.
    pub async fn attach_session_no_snapshot(
        &self,
        session_id: &str,
        cols: u32,
        rows: u32,
    ) -> Result<SessionHandle> {
        let mut client = self.client.lock().await.clone();
        client
            .attach_session(AttachSessionRequest {
                session_id: session_id.to_string(),
                cols,
                rows,
                want_screen_snapshot: false,
            })
            .await?;

        Ok(SessionHandle::new(
            session_id.to_string(),
            self.client.clone(),
            self.info.clone(),
        ))
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

// ===========================================================================
// SSH transport: gRPC over an SSH channel with no local socket file.
//
// Each SSH connection's `SshChannelOpener` is held in a registry keyed by the
// synthetic path that flows through the app as the connection's `socket_path`.
// Reconnects (output-stream readers run in their own runtimes) look the opener
// up and dial a fresh `direct-streamlocal` channel over the shared SSH
// connection, which is bridged to async with in-process channels (no sockets).
// ===========================================================================

#[cfg(unix)]
static SSH_TUNNELS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<PathBuf, Arc<cterm_core::SshTunnel>>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

#[cfg(unix)]
fn register_ssh_tunnel(key: PathBuf, tunnel: Arc<cterm_core::SshTunnel>) {
    SSH_TUNNELS.lock().unwrap().insert(key, tunnel);
}

#[cfg(unix)]
fn ssh_opener_for(key: &Path) -> Option<cterm_core::SshChannelOpener> {
    SSH_TUNNELS.lock().unwrap().get(key).map(|t| t.opener())
}

#[cfg(unix)]
fn unregister_ssh_tunnel(key: &Path) {
    if let Some(tunnel) = SSH_TUNNELS.lock().unwrap().remove(key) {
        tunnel.close();
    }
}

/// An async-I/O bridge over a blocking SSH channel. The read direction
/// (daemon -> client) is bounded so a slow consumer applies backpressure to
/// the remote via the SSH channel's own flow control; the write direction
/// (client -> daemon: input, resize, control) is small and unbounded.
#[cfg(unix)]
struct SshChannelIo {
    rx: tokio::sync::mpsc::Receiver<Vec<u8>>,
    read_buf: Vec<u8>,
    read_pos: usize,
    tx: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
}

#[cfg(unix)]
fn ssh_channel_io(
    reader: cterm_core::SshChannelReader,
    writer: cterm_core::SshChannelWriter,
) -> SshChannelIo {
    use std::io::{Read, Write};

    let (read_tx, read_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(16);
    let (write_tx, mut write_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();

    // Daemon output: blocking-read the channel, hand chunks to the async side.
    let mut reader = reader;
    std::thread::spawn(move || {
        let mut buf = [0u8; 32 * 1024];
        loop {
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if read_tx.blocking_send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
        // `read_tx` drops here -> the async receiver sees EOF.
    });

    // Client input: blocking-write each chunk the async side produces.
    let mut writer = writer;
    std::thread::spawn(move || {
        while let Some(chunk) = write_rx.blocking_recv() {
            if writer.write_all(&chunk).is_err() {
                break;
            }
        }
        // `writer` drops here -> SshChannelWriter sends EOF/Close to the channel.
    });

    SshChannelIo {
        rx: read_rx,
        read_buf: Vec::new(),
        read_pos: 0,
        tx: write_tx,
    }
}

#[cfg(unix)]
impl tokio::io::AsyncRead for SshChannelIo {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        use std::task::Poll;
        let this = self.get_mut();
        if this.read_pos >= this.read_buf.len() {
            match this.rx.poll_recv(cx) {
                Poll::Ready(Some(chunk)) => {
                    this.read_buf = chunk;
                    this.read_pos = 0;
                }
                // Channel closed: report EOF (0 bytes read).
                Poll::Ready(None) => return Poll::Ready(Ok(())),
                Poll::Pending => return Poll::Pending,
            }
        }
        let n = (this.read_buf.len() - this.read_pos).min(buf.remaining());
        buf.put_slice(&this.read_buf[this.read_pos..this.read_pos + n]);
        this.read_pos += n;
        Poll::Ready(Ok(()))
    }
}

#[cfg(unix)]
impl tokio::io::AsyncWrite for SshChannelIo {
    fn poll_write(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        match self.tx.send(buf.to_vec()) {
            Ok(()) => std::task::Poll::Ready(Ok(buf.len())),
            Err(_) => std::task::Poll::Ready(Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "ssh channel closed",
            ))),
        }
    }

    fn poll_flush(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }

    fn poll_shutdown(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        std::task::Poll::Ready(Ok(()))
    }
}
