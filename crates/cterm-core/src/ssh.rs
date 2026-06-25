//! Native SSH backend for [`crate::pty::Pty`], built on the pure-Rust
//! [`puressh`] library.
//!
//! An SSH tab no longer spawns the system `ssh` binary inside a local PTY.
//! Instead, [`SshPty`] opens a real SSH connection, allocates a remote
//! PTY-backed shell channel, and exposes the same blocking
//! read/write/resize/signal surface the local PTY does. puressh's
//! `OwnedChannelStream` is already a blocking `Read`/`Write`, so no socketpair
//! or file descriptor is involved.
//!
//! Authentication and host-key verification happen out of band (via the
//! puressh API) rather than in-band on a tty the way OpenSSH does. Callers
//! supply prompt callbacks (see [`SshConfig`]) so the surrounding UI can ask
//! the user about an unknown host key, a password, or a key passphrase.

use std::io::{self, Read};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use sha2::{Digest, Sha256};

#[cfg(unix)]
use puressh::agent::{Agent, AgentHostKey};
use puressh::auth::ClientCredential;
use puressh::client::{Client, Config, HostKeyPolicy, KnownHostsPolicy, TofuAction};
use puressh::known_hosts::KnownHosts;
use puressh::shared::{OwnedChannelStream, SharedClient};

use crate::pty::{PtyError, PtySize};

/// A host key presented by a server that is not (yet) trusted.
///
/// Passed to a [`HostKeyPrompt`] so the UI can show the user what they are
/// being asked to trust.
#[derive(Debug, Clone)]
pub struct HostKeyRequest {
    /// Hostname being connected to.
    pub host: String,
    /// Port being connected to.
    pub port: u16,
    /// SSH key type, e.g. `ssh-ed25519`.
    pub key_type: String,
    /// OpenSSH-style `SHA256:…` fingerprint of the key.
    pub fingerprint: String,
    /// Whether this host already had a *different* key on record (a mismatch,
    /// the security-relevant case) versus simply being unknown.
    pub changed: bool,
}

/// Callback invoked when a server presents an untrusted host key.
///
/// Returns `true` to accept (and persist) the key, `false` to abort the
/// connection. Runs on the connecting (background) thread, so an
/// implementation that needs to show UI must marshal to its UI thread and
/// block for the answer.
pub type HostKeyPrompt = Arc<dyn Fn(HostKeyRequest) -> bool + Send + Sync>;

/// Callback invoked to obtain a password for password authentication.
///
/// The argument is the server's prompt text (often empty). Returns `None` to
/// decline (no more password attempts).
pub type PasswordPrompt = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// Callback invoked to obtain the passphrase for an encrypted identity file.
///
/// The argument is the identity file path. Returns `None` to skip that key.
pub type PassphrasePrompt = Arc<dyn Fn(&str) -> Option<String> + Send + Sync>;

/// A bundle of interactive prompt callbacks for an SSH connection. Used to wire
/// in-process UI prompts (e.g. the remote-ctermd tunnel) where the prompts can
/// be shown directly rather than round-tripped over gRPC.
#[derive(Clone, Default)]
pub struct SshPrompts {
    pub host_key: Option<HostKeyPrompt>,
    pub password: Option<PasswordPrompt>,
    pub passphrase: Option<PassphrasePrompt>,
}

/// A `-L`-style local port forward: bind `local_port` locally and forward each
/// connection to `remote_host:remote_port` (resolved on the server).
#[derive(Clone, Debug)]
pub struct LocalForward {
    pub local_port: u16,
    pub remote_host: String,
    pub remote_port: u16,
}

/// Configuration for a native SSH connection.
///
/// Built by the application layer from its own SSH tab settings. The prompt
/// callbacks are optional: when absent, host-key verification is strict
/// (unknown keys are rejected) and no interactive password/passphrase entry is
/// attempted (authentication then relies on the agent and unencrypted keys).
#[derive(Clone, Default)]
pub struct SshConfig {
    /// Remote host to connect to.
    pub host: String,
    /// Remote port (defaults handled by the caller; 22 if unset).
    pub port: u16,
    /// Login user; defaults to the local user when `None`.
    pub username: Option<String>,
    /// Identity (private key) files to offer for public-key auth.
    pub identity_files: Vec<PathBuf>,
    /// `TERM` to request for the remote PTY (defaults to `xterm-256color`).
    pub term: Option<String>,
    /// Optional remote command to run instead of an interactive shell.
    pub remote_command: Option<String>,

    /// Local port forwards (`-L`).
    pub local_forwards: Vec<LocalForward>,

    /// ProxyJump-style jump host (`user@host[:port]`). Not yet supported by the
    /// puressh shell model; currently rejected if set.
    pub jump_host: Option<String>,
    /// Forward the local SSH agent (`-A`). Requires puressh serve-loop support
    /// not available alongside the multichannel shell; not yet wired.
    pub agent_forward: bool,
    /// Enable X11 forwarding (`-X`). Requires puressh serve-loop support not
    /// available alongside the multichannel shell; not yet wired.
    pub x11_forward: bool,

    /// Prompt for accepting unknown/changed host keys.
    pub host_key_prompt: Option<HostKeyPrompt>,
    /// Prompt for a login password.
    pub password_prompt: Option<PasswordPrompt>,
    /// Prompt for an identity-file passphrase.
    pub passphrase_prompt: Option<PassphrasePrompt>,
}

/// A native SSH session presenting a PTY-equivalent interface.
pub struct SshPty {
    /// Shared client handle (cheap to clone) used for writes and control.
    client: SharedClient,
    /// Channel id of the interactive shell.
    channel_id: u32,
    /// The shell's stdin/stdout stream. Taken out by the first
    /// [`Self::try_clone_reader`] call (the daemon's reader thread owns it).
    stream: Mutex<Option<OwnedChannelStream>>,
    /// Last requested size, for completeness.
    size: Mutex<PtySize>,
    /// Stop flag for `-L` forward listener threads; set on drop.
    forwards_stop: Arc<std::sync::atomic::AtomicBool>,
}

impl SshPty {
    /// Open the connection, authenticate, and start a remote shell.
    pub fn connect(config: SshConfig, size: PtySize) -> Result<Self, PtyError> {
        let client = connect_and_authenticate(&config)?;

        let shared = SharedClient::from(client);
        let term = config.term.as_deref().unwrap_or("xterm-256color");
        let stream = shared
            .shell(term, size.cols.max(1) as u32, size.rows.max(1) as u32)
            .map_err(|e| PtyError::Spawn(format!("SSH shell request failed: {e}")))?;
        let channel_id = stream.channel_id();

        // Start any `-L` local port forwards.
        let forwards_stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        for fwd in &config.local_forwards {
            start_local_forward(shared.clone(), fwd.clone(), Arc::clone(&forwards_stop));
        }

        Ok(Self {
            client: shared,
            channel_id,
            stream: Mutex::new(Some(stream)),
            size: Mutex::new(size),
            forwards_stop,
        })
    }

    pub fn child_pid(&self) -> i32 {
        // SSH sessions have no local child process.
        -1
    }

    pub fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.client.channel_send_data(self.channel_id, data)
    }

    pub fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut guard = self.stream.lock().unwrap();
        match guard.as_mut() {
            Some(stream) => stream.read(buf),
            None => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "SSH channel reader has been taken",
            )),
        }
    }

    pub fn resize(&self, rows: u16, cols: u16) -> io::Result<()> {
        if let Ok(mut size) = self.size.lock() {
            size.rows = rows;
            size.cols = cols;
        }
        self.client
            .send_window_change(self.channel_id, cols as u32, rows as u32, 0, 0)
            .map_err(|e| io::Error::other(format!("window-change: {e}")))
    }

    pub fn is_running(&mut self) -> bool {
        // The daemon detects exit when the reader stream hits EOF; until then,
        // treat the session as alive.
        let guard = self.stream.lock().unwrap();
        match guard.as_ref() {
            Some(stream) => stream.exit_status().is_none(),
            None => true,
        }
    }

    pub fn wait(&mut self) -> io::Result<i32> {
        // Drain the channel to EOF, then report the remote exit status.
        let mut guard = self.stream.lock().unwrap();
        if let Some(stream) = guard.as_mut() {
            let mut scratch = [0u8; 4096];
            while stream.read(&mut scratch)? != 0 {}
            return Ok(stream.exit_status().unwrap_or(0));
        }
        Ok(0)
    }

    pub fn try_wait(&mut self) -> io::Result<Option<i32>> {
        let guard = self.stream.lock().unwrap();
        Ok(guard.as_ref().and_then(|s| s.exit_status()))
    }

    pub fn send_signal(&self, _signal: i32) -> io::Result<()> {
        // puressh does not yet expose an out-of-band "signal" channel request;
        // closing the write half is the closest we can do for terminal signals.
        if let Ok(mut guard) = self.stream.lock() {
            if let Some(stream) = guard.as_mut() {
                let _ = stream.send_eof();
            }
        }
        Ok(())
    }

    /// Hand the channel stream to a reader (the daemon's per-session thread).
    ///
    /// `OwnedChannelStream` is itself a blocking `Read + Send`, so it *is* the
    /// reader; writes and resizes continue to go through the cloned
    /// [`SharedClient`]. Only the first call yields the stream.
    pub fn try_clone_reader(&self) -> io::Result<Box<dyn Read + Send>> {
        let mut guard = self.stream.lock().unwrap();
        match guard.take() {
            Some(stream) => Ok(Box::new(stream)),
            None => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "SSH channel reader already taken",
            )),
        }
    }
}

impl Drop for SshPty {
    fn drop(&mut self) {
        self.forwards_stop
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// Start a `-L` local port forward: bind `127.0.0.1:local_port` and forward each
/// accepted TCP connection to the remote target over a `direct-tcpip` channel.
fn start_local_forward(
    client: SharedClient,
    fwd: LocalForward,
    stop: Arc<std::sync::atomic::AtomicBool>,
) {
    use std::net::TcpListener;
    use std::sync::atomic::Ordering;

    let listener = match TcpListener::bind(("127.0.0.1", fwd.local_port)) {
        Ok(l) => l,
        Err(e) => {
            log::warn!("SSH -L: failed to bind local port {}: {e}", fwd.local_port);
            return;
        }
    };
    if listener.set_nonblocking(true).is_err() {
        log::warn!(
            "SSH -L: failed to set non-blocking on port {}",
            fwd.local_port
        );
    }

    std::thread::spawn(move || loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        match listener.accept() {
            Ok((tcp, _)) => {
                match client.open_direct_tcpip(
                    &fwd.remote_host,
                    fwd.remote_port,
                    "127.0.0.1",
                    fwd.local_port,
                ) {
                    Ok(channel) => spawn_tcp_channel_splice(tcp, client.clone(), channel),
                    Err(e) => log::warn!("SSH -L: open direct-tcpip failed: {e}"),
                }
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(e) => {
                log::debug!("SSH -L listener on {} ended: {e}", fwd.local_port);
                break;
            }
        }
    });
}

/// Bidirectionally splice a TCP stream and a `direct-tcpip` channel. Reads from
/// the channel use the owned stream; writes go through the shared client by
/// channel id, so the two directions run on independent threads.
fn spawn_tcp_channel_splice(
    tcp: std::net::TcpStream,
    client: SharedClient,
    channel: OwnedChannelStream,
) {
    use std::io::{Read, Write};

    let channel_id = channel.channel_id();
    let Ok(mut tcp_read) = tcp.try_clone() else {
        return;
    };
    let mut tcp_write = tcp;

    // TCP -> channel
    let client_w = client.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 32 * 1024];
        loop {
            match tcp_read.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if client_w.channel_send_data(channel_id, &buf[..n]).is_err() {
                        break;
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        let _ = client_w.channel_send_eof(channel_id);
    });

    // channel -> TCP
    let mut channel = channel;
    std::thread::spawn(move || {
        let mut buf = [0u8; 32 * 1024];
        loop {
            match channel.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if tcp_write.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = tcp_write.shutdown(std::net::Shutdown::Both);
    });
}

/// Connect to the host, verify the host key, and authenticate, returning the
/// authenticated puressh [`Client`]. Shared by the interactive shell
/// ([`SshPty`]) and the gRPC tunnel ([`SshTunnel`]).
fn connect_and_authenticate(config: &SshConfig) -> Result<Client, PtyError> {
    if config.jump_host.is_some() {
        return Err(PtyError::Spawn(
            "SSH jump hosts are not supported yet".to_string(),
        ));
    }

    let port = if config.port == 0 { 22 } else { config.port };

    // Host-key policy: strict known_hosts, optionally prompting via the UI.
    let known_hosts_path = default_known_hosts_path();
    let store = match &known_hosts_path {
        Some(path) => Arc::new(Mutex::new(
            KnownHosts::load(path).unwrap_or_else(|_| KnownHosts::new()),
        )),
        None => Arc::new(Mutex::new(KnownHosts::new())),
    };
    let mut policy = KnownHostsPolicy::strict(Arc::clone(&store));
    policy.save_path = known_hosts_path.clone();
    if let Some(prompt) = config.host_key_prompt.clone() {
        policy.on_unknown = TofuAction::Prompt(make_tofu(prompt.clone(), false));
        policy.on_mismatch = TofuAction::Prompt(make_tofu(prompt, true));
    }
    let cfg = Config {
        host_key_policy: HostKeyPolicy::KnownHosts(policy),
        timeout: None,
        algorithms: Default::default(),
    };

    let mut client = Client::connect_to_host(&config.host, port, cfg)
        .map_err(|e| PtyError::Spawn(format!("SSH connect to {}: {e}", config.host)))?;

    let user = config
        .username
        .clone()
        .or_else(default_username)
        .unwrap_or_else(|| "root".to_string());

    let credentials = build_credentials(config);
    if credentials.is_empty() {
        return Err(PtyError::Spawn(
            "no SSH credentials available (agent, identity files, or password)".to_string(),
        ));
    }
    client
        .authenticate(&user, credentials)
        .map_err(|e| PtyError::Spawn(format!("SSH authentication failed: {e}")))?;

    Ok(client)
}

/// Wrap a cterm [`HostKeyPrompt`] as a puressh TOFU prompt callback.
fn make_tofu(prompt: HostKeyPrompt, changed: bool) -> Arc<puressh::client::TofuPromptFn> {
    Arc::new(
        move |host: &str, port: u16, key_type: &str, key_blob: &[u8]| {
            prompt(HostKeyRequest {
                host: host.to_string(),
                port,
                key_type: key_type.to_string(),
                fingerprint: fingerprint_sha256(key_blob),
                changed,
            })
        },
    )
}

/// OpenSSH-style `SHA256:<base64-no-padding>` fingerprint of a key blob.
fn fingerprint_sha256(key_blob: &[u8]) -> String {
    use base64::Engine;
    let digest = Sha256::digest(key_blob);
    let b64 = base64::engine::general_purpose::STANDARD_NO_PAD.encode(digest);
    format!("SHA256:{b64}")
}

/// Routes keyboard-interactive prompts through a cterm [`PasswordPrompt`].
///
/// Each server prompt (`(text, echo)`) is answered by calling the password
/// callback with the prompt text. This covers the common single-prompt
/// password-over-keyboard-interactive case as well as multi-prompt challenges.
struct CallbackKbiResponder {
    prompt: PasswordPrompt,
}

impl puressh::auth::KeyboardInteractiveResponder for CallbackKbiResponder {
    fn respond(
        &mut self,
        _name: &str,
        _instruction: &str,
        prompts: &[(String, bool)],
    ) -> Vec<String> {
        prompts
            .iter()
            .map(|(text, _echo)| (self.prompt)(text).unwrap_or_default())
            .collect()
    }
}

/// Collect authentication credentials: agent identities, identity-file keys,
/// then interactive password and keyboard-interactive (if a prompt is
/// configured).
fn build_credentials(config: &SshConfig) -> Vec<ClientCredential> {
    let mut creds: Vec<ClientCredential> = Vec::new();

    // ssh-agent identities (Unix only; the agent protocol uses a Unix socket).
    #[cfg(unix)]
    {
        if let Ok(Some(agent)) = Agent::connect_env() {
            let agent = Arc::new(Mutex::new(agent));
            let identities = agent.lock().ok().and_then(|mut a| a.identities().ok());
            if let Some(identities) = identities {
                for ident in identities {
                    if let Ok(hk) = AgentHostKey::from_identity(Arc::clone(&agent), ident.key_blob)
                    {
                        creds.push(ClientCredential::PublicKey(Box::new(hk)));
                    }
                }
            }
        }
    }

    // Identity files. Offer each key's public half (read from its `.pub`) and
    // defer reading/decrypting the private key until the server selects it via
    // the publickey probe. This way an encrypted key such as `id_rsa` is never
    // decrypted — and never prompts for a passphrase — when an earlier key like
    // `id_ed25519` already authenticates. When none are configured, fall back
    // to the standard OpenSSH defaults (`~/.ssh/id_*`); this matters for GUI
    // launches, which don't inherit `SSH_AUTH_SOCK`.
    let identity_files = if config.identity_files.is_empty() {
        default_identity_files()
    } else {
        config.identity_files.clone()
    };
    for path in identity_files {
        match identity_offer(&path) {
            Some((public_blob, algorithm)) => {
                creds.push(ClientCredential::PublicKey(Box::new(LazyIdentity {
                    path,
                    public_blob,
                    algorithm,
                    passphrase_prompt: config.passphrase_prompt.clone(),
                    signer: Mutex::new(None),
                })));
            }
            None => log::warn!("ssh: no usable public key for identity {}", path.display()),
        }
    }

    // Interactive auth, last: both `password` and `keyboard-interactive` so we
    // work against servers that only offer one of them (e.g. PAM-backed hosts
    // typically use keyboard-interactive).
    if let Some(prompt) = config.password_prompt.clone() {
        let pw = prompt.clone();
        creds.push(ClientCredential::PasswordPrompt(Box::new(move |_retry| {
            pw("").map(|p| p.into())
        })));
        creds.push(ClientCredential::KeyboardInteractive(Box::new(
            CallbackKbiResponder { prompt },
        )));
    }

    creds
}

/// Load an identity file into a public-key credential, prompting for a
/// passphrase if the key is encrypted and a prompt is available.
/// A public-key credential whose private key is read and decrypted only when
/// the server selects it (puressh signs only after a successful publickey
/// probe). The public half offered during the probe comes from the `.pub` file
/// or an unencrypted private key, so an encrypted identity such as `id_rsa` is
/// never decrypted — and never prompts for a passphrase — unless the server
/// actually asks us to sign with it.
struct LazyIdentity {
    /// Private key file path, read lazily in `sign`.
    path: PathBuf,
    /// SSH wire-format public key, used for the offer/probe.
    public_blob: Vec<u8>,
    /// Signature algorithm advertised for this key.
    algorithm: &'static str,
    /// Passphrase prompt for an encrypted private key.
    passphrase_prompt: Option<PassphrasePrompt>,
    /// Decrypted signer, loaded (and prompted for) at most once.
    signer: Mutex<Option<Box<dyn puressh::hostkey::HostKey + Send>>>,
}

impl LazyIdentity {
    fn load_signer(
        &self,
    ) -> Result<Box<dyn puressh::hostkey::HostKey + Send>, puressh::error::Error> {
        let pem = std::fs::read_to_string(&self.path).map_err(puressh::error::Error::Io)?;
        // Unencrypted keys load without a passphrase.
        if let Ok(key) = puressh::key::PrivateKey::parse_openssh_pem(&pem, None) {
            return key.into_host_key();
        }
        // Encrypted: prompt for the passphrase, only now that it's needed.
        let prompt = self
            .passphrase_prompt
            .as_ref()
            .ok_or(puressh::error::Error::Crypto(
                "encrypted identity, no passphrase prompt",
            ))?;
        let phrase =
            prompt(&self.path.to_string_lossy()).ok_or(puressh::error::Error::AuthFailed)?;
        let key = puressh::key::PrivateKey::parse_openssh_pem(&pem, Some(phrase.as_bytes()))
            .map_err(|_| puressh::error::Error::Crypto("failed to decrypt identity"))?;
        key.into_host_key()
    }
}

impl puressh::hostkey::HostKey for LazyIdentity {
    fn algorithm(&self) -> &'static str {
        self.algorithm
    }

    fn public_blob(&self) -> Vec<u8> {
        self.public_blob.clone()
    }

    fn sign(&self, msg: &[u8]) -> Result<Vec<u8>, puressh::error::Error> {
        let mut guard = self
            .signer
            .lock()
            .map_err(|_| puressh::error::Error::Protocol("ssh identity mutex poisoned"))?;
        if guard.is_none() {
            *guard = Some(self.load_signer()?);
        }
        guard.as_ref().expect("signer loaded above").sign(msg)
    }
}

/// Read the public half of an identity for the userauth probe without touching
/// (or decrypting) the private key. Prefers the sibling `<path>.pub` file and
/// falls back to deriving it from an unencrypted private key.
fn identity_offer(path: &std::path::Path) -> Option<(Vec<u8>, &'static str)> {
    // Prefer the `.pub` file — it never requires the private key or passphrase.
    let mut pub_path = path.as_os_str().to_os_string();
    pub_path.push(".pub");
    if let Ok(line) = std::fs::read_to_string(PathBuf::from(pub_path)) {
        if let Ok(pk) = puressh::key::PublicKey::parse_authorized_keys_line(line.trim()) {
            return Some((pk.wire_blob(), advertised_algorithm(&pk)));
        }
    }
    // Fall back: derive the public key from an unencrypted private key (no prompt).
    let pem = std::fs::read_to_string(path).ok()?;
    let key = puressh::key::PrivateKey::parse_openssh_pem(&pem, None).ok()?;
    let pk = key.public_key();
    Some((pk.wire_blob(), advertised_algorithm(&pk)))
}

/// The signature algorithm to advertise for a public key. RSA keys are offered
/// as `rsa-sha2-512` to match the signer built by `PrivateKey::into_host_key`
/// (and because servers reject the legacy SHA-1 `ssh-rsa`).
fn advertised_algorithm(pk: &puressh::key::PublicKey) -> &'static str {
    match pk.algorithm() {
        "ssh-rsa" => "rsa-sha2-512",
        other => other,
    }
}

/// Best-effort local username.
fn default_username() -> Option<String> {
    std::env::var("USER")
        .ok()
        .or_else(|| std::env::var("USERNAME").ok())
        .filter(|s| !s.is_empty())
}

/// Path to the user's `known_hosts` file, if a home directory is known.
fn default_known_hosts_path() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(PathBuf::from(home).join(".ssh").join("known_hosts"))
}

/// Standard OpenSSH default identity files, in preference order, returned only
/// if they exist on disk. Used when no identity files are explicitly
/// configured, mirroring OpenSSH trying `~/.ssh/id_*` automatically.
fn default_identity_files() -> Vec<PathBuf> {
    let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) else {
        return Vec::new();
    };
    let ssh_dir = PathBuf::from(home).join(".ssh");
    ["id_ed25519", "id_ecdsa", "id_rsa", "id_dsa"]
        .iter()
        .map(|name| ssh_dir.join(name))
        .filter(|p| p.is_file())
        .collect()
}

// ============================================================================
// SSH tunnel: forward a local Unix socket to a remote Unix socket over SSH.
//
// This replaces the old `ssh -L local.sock:remote.sock host` invocation used to
// reach a remote ctermd's gRPC socket. It runs a serve loop on the puressh
// connection and opens a `direct-streamlocal@openssh.com` channel per accepted
// local connection.
// ============================================================================

/// A live SSH tunnel forwarding a local Unix socket to a remote Unix socket.
///
/// Dropping (or [`SshTunnel::close`]) stops the serve loop and removes the
/// local socket.
#[cfg(unix)]
pub struct SshTunnel {
    stop: Arc<std::sync::atomic::AtomicBool>,
    local_socket: PathBuf,
}

#[cfg(unix)]
impl SshTunnel {
    /// Connect and authenticate, run `setup_command` to learn the remote socket
    /// path (its last stdout line), then forward `local_socket` to it.
    pub fn connect(
        config: SshConfig,
        setup_command: &str,
        local_socket: PathBuf,
    ) -> Result<Self, PtyError> {
        use puressh::client::ClientHandlers;
        use std::os::unix::net::UnixListener;
        use std::sync::atomic::Ordering;

        let mut client = connect_and_authenticate(&config)?;

        // Run the setup command to discover the remote daemon socket path.
        let out = client
            .exec(setup_command)
            .map_err(|e| PtyError::Spawn(format!("SSH setup command failed: {e}")))?;
        let stdout = String::from_utf8_lossy(&out.stdout);
        let remote_socket = stdout.lines().last().unwrap_or("").trim().to_string();
        if remote_socket.is_empty() {
            return Err(PtyError::Spawn(format!(
                "remote setup returned no socket path (stderr: {})",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }

        // Bind the local Unix socket the gRPC client will connect to.
        let _ = std::fs::remove_file(&local_socket);
        if let Some(parent) = local_socket.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let listener = UnixListener::bind(&local_socket)
            .map_err(|e| PtyError::Spawn(format!("bind {}: {e}", local_socket.display())))?;
        listener.set_nonblocking(true).ok();

        // Pair a serve context (for opening channels from the accept thread)
        // with the handler set that the serve loop runs.
        let (handlers, ctx) = ClientHandlers::new().with_serve_context();
        let stop = handlers.stop.clone();

        // Pump thread: drives the connection and services ctx channel opens.
        let serve_stop = stop.clone();
        std::thread::spawn(move || {
            if let Err(e) = client.serve(handlers) {
                log::debug!("SSH tunnel serve loop ended: {e}");
            }
            serve_stop.store(true, Ordering::Relaxed);
        });

        // Accept thread: forward each local connection over a streamlocal channel.
        let accept_stop = stop.clone();
        std::thread::spawn(move || loop {
            if accept_stop.load(Ordering::Relaxed) {
                break;
            }
            match listener.accept() {
                Ok((local_stream, _)) => match ctx.open_direct_streamlocal(&remote_socket) {
                    Ok(channel) => spawn_unix_channel_splice(local_stream, channel),
                    Err(e) => log::warn!("SSH tunnel: open channel failed: {e}"),
                },
                Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                Err(e) => {
                    log::debug!("SSH tunnel accept loop ended: {e}");
                    break;
                }
            }
        });

        Ok(Self { stop, local_socket })
    }

    /// The local Unix socket path clients should connect to.
    pub fn local_socket(&self) -> &std::path::Path {
        &self.local_socket
    }

    /// Stop the tunnel and remove the local socket. Idempotent.
    pub fn close(&self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = std::fs::remove_file(&self.local_socket);
    }
}

#[cfg(unix)]
impl Drop for SshTunnel {
    fn drop(&mut self) {
        self.close();
    }
}

/// Bidirectionally splice a local Unix stream and an SSH channel stream, each
/// direction on its own thread (mirrors puressh's internal forward splice).
#[cfg(unix)]
fn spawn_unix_channel_splice(
    local: std::os::unix::net::UnixStream,
    channel: puressh::stream::ChannelStream,
) {
    use puressh::stream::ChannelEgress;
    use std::io::{Read, Write};

    let (chan_rx, chan_tx) = channel.into_raw();
    let Ok(mut local_in) = local.try_clone() else {
        let _ = chan_tx.send(ChannelEgress::Eof);
        let _ = chan_tx.send(ChannelEgress::Close);
        return;
    };
    let mut local_out = local;

    // local -> channel
    let chan_tx_a = chan_tx.clone();
    std::thread::spawn(move || {
        let mut buf = [0u8; 32 * 1024];
        loop {
            match local_in.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if chan_tx_a
                        .send(ChannelEgress::Data(buf[..n].to_vec()))
                        .is_err()
                    {
                        break;
                    }
                }
                Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
        let _ = chan_tx_a.send(ChannelEgress::Eof);
    });

    // channel -> local
    std::thread::spawn(move || {
        while let Ok(Some(chunk)) = chan_rx.recv() {
            if local_out.write_all(&chunk).is_err() {
                break;
            }
        }
        let _ = chan_tx.send(ChannelEgress::Close);
        let _ = local_out.shutdown(std::net::Shutdown::Both);
    });
}
