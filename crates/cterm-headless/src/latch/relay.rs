//! Relay client: persistent QUIC connection to relay server for NAT traversal.
//!
//! Maintains a QUIC connection to relay.unixshells.com, authenticates
//! with an Ed25519 device key, and accepts incoming streams for SSH,
//! mosh UDP bridging, and web connections.

use crate::latch::auth::AuthorizedKeys;
use crate::session::SessionManager;
use cterm_app::config::LatchConfig;
use parking_lot::RwLock;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

/// ALPN protocol for the relay QUIC connection.
const RELAY_ALPN: &str = "latch-relay";

/// Domain separator for relay authentication signatures.
const AUTH_DOMAIN: &str = "latch-relay-auth-v1:";

/// Stream type bytes for dispatching incoming relay streams.
const STREAM_TYPE_SSH: u8 = 0x00;
const STREAM_TYPE_UDP: u8 = 0x01;
const STREAM_TYPE_WEB: u8 = 0x02;
const STREAM_TYPE_CONTROL: u8 = 0x04;

/// Relay connection configuration.
struct RelayConfig {
    /// Relay server host (e.g., "relay.unixshells.com:443")
    host: String,
    /// Device username for the relay account
    username: String,
    /// Device name
    device: String,
    /// Path to device Ed25519 key
    key_path: std::path::PathBuf,
}

/// Start the relay client as a background task.
///
/// Maintains a persistent QUIC connection to the relay server with
/// auto-reconnect and exponential backoff.
pub async fn start_relay_client(
    config: &LatchConfig,
    authorized_keys: Arc<RwLock<AuthorizedKeys>>,
    session_manager: Arc<SessionManager>,
) -> anyhow::Result<tokio::task::JoinHandle<()>> {
    let relay_host = format!("{}:443", config.relay_host);
    let username = config
        .relay_username
        .clone()
        .ok_or_else(|| anyhow::anyhow!("relay_username required for relay connection"))?;
    let device = config.relay_device.clone().unwrap_or_else(|| {
        hostname::get().map_or("unknown".into(), |h| h.to_string_lossy().into())
    });

    let key_path = crate::cli::config_dir().join("relay.key");

    log::info!(
        "Starting relay client: {}@{} -> {}",
        username,
        device,
        relay_host
    );

    let handle = tokio::spawn(async move {
        let mut backoff = Duration::from_secs(1);
        let max_backoff = Duration::from_secs(30);

        loop {
            log::info!("Connecting to relay server {}...", relay_host);

            match connect_relay(&relay_host, &username, &device, &key_path).await {
                Ok(connection) => {
                    backoff = Duration::from_secs(1);
                    log::info!("Connected to relay server");

                    // Accept incoming streams until connection dies
                    accept_loop(connection, &authorized_keys, &session_manager).await;

                    log::warn!("Relay connection lost, reconnecting...");
                }
                Err(e) => {
                    if e.to_string().contains("auth rejected") {
                        log::error!("Relay authentication rejected — check relay credentials");
                        return;
                    }
                    log::warn!("Relay connection failed: {}, retrying in {:?}", e, backoff);
                }
            }

            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(max_backoff);
        }
    });

    Ok(handle)
}

/// Connect to the relay server and authenticate.
async fn connect_relay(
    host: &str,
    username: &str,
    device: &str,
    key_path: &std::path::Path,
) -> anyhow::Result<quinn::Connection> {
    // Load or generate device key
    let _device_key = load_or_generate_device_key(key_path)?;

    // Configure QUIC client
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let mut tls_config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    tls_config.alpn_protocols = vec![RELAY_ALPN.as_bytes().to_vec()];

    let client_config = quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(tls_config)?,
    ));

    // Resolve address
    let addr: SocketAddr = tokio::net::lookup_host(host)
        .await?
        .next()
        .ok_or_else(|| anyhow::anyhow!("Failed to resolve {}", host))?;

    // Connect
    let endpoint = quinn::Endpoint::client("0.0.0.0:0".parse()?)?;
    let server_name = host.split(':').next().unwrap_or(host);
    let connection = endpoint
        .connect_with(client_config, addr, server_name)?
        .await?;

    // Authenticate
    authenticate(&connection, username, device, key_path).await?;

    Ok(connection)
}

/// Perform the relay authentication handshake.
async fn authenticate(
    connection: &quinn::Connection,
    username: &str,
    device: &str,
    _key_path: &std::path::Path,
) -> anyhow::Result<()> {
    let (mut send, mut recv) = connection.open_bi().await?;

    // Step 1: Send identity
    let mut identity = Vec::new();
    identity.push(username.len() as u8);
    identity.extend_from_slice(username.as_bytes());
    identity.push(device.len() as u8);
    identity.extend_from_slice(device.as_bytes());
    // TODO: append pubkey with 2-byte length prefix
    identity.extend_from_slice(&[0, 0]); // placeholder pubkey length

    send.write_all(&identity).await?;

    // Step 2: Read challenge
    let mut challenge_len_buf = [0u8; 1];
    recv.read_exact(&mut challenge_len_buf).await?;
    let challenge_len = challenge_len_buf[0] as usize;
    let mut challenge = vec![0u8; challenge_len];
    recv.read_exact(&mut challenge).await?;

    // Step 3: Sign challenge
    // TODO: sign with device key using AUTH_DOMAIN separator
    let sig = Vec::new(); // placeholder
    let sig_len = (sig.len() as u16).to_be_bytes();
    send.write_all(&sig_len).await?;
    send.write_all(&sig).await?;
    send.finish()?;

    // Step 4: Read result
    let mut result = [0u8; 1];
    recv.read_exact(&mut result).await?;
    if result[0] != 1 {
        return Err(anyhow::anyhow!("auth rejected by relay server"));
    }

    Ok(())
}

/// Accept and dispatch incoming relay streams.
async fn accept_loop(
    connection: quinn::Connection,
    _authorized_keys: &Arc<RwLock<AuthorizedKeys>>,
    _session_manager: &Arc<SessionManager>,
) {
    loop {
        match connection.accept_bi().await {
            Ok((send, mut recv)) => {
                // Read stream type byte
                let mut type_buf = [0u8; 1];
                if recv.read_exact(&mut type_buf).await.is_err() {
                    continue;
                }

                match type_buf[0] {
                    STREAM_TYPE_SSH => {
                        log::info!("Relay: incoming SSH stream");
                        // TODO: read IP header, create SSH bridge
                        tokio::spawn(async move {
                            let _ = (send, recv);
                        });
                    }
                    STREAM_TYPE_UDP => {
                        log::info!("Relay: incoming UDP bridge stream");
                        // TODO: read target port + IP, create UDP relay
                        tokio::spawn(async move {
                            let _ = (send, recv);
                        });
                    }
                    STREAM_TYPE_WEB => {
                        log::info!("Relay: incoming web stream");
                        tokio::spawn(async move {
                            let _ = (send, recv);
                        });
                    }
                    other => {
                        log::debug!("Relay: unknown stream type {}", other);
                    }
                }
            }
            Err(e) => {
                log::warn!("Relay accept error: {}", e);
                break;
            }
        }
    }
}

/// Load or generate the device Ed25519 key for relay authentication.
fn load_or_generate_device_key(path: &std::path::Path) -> anyhow::Result<ssh_key::PrivateKey> {
    if path.exists() {
        let key = ssh_key::PrivateKey::read_openssh_file(path)
            .map_err(|e| anyhow::anyhow!("Failed to load relay key: {}", e))?;
        return Ok(key);
    }

    // Generate new key
    let key =
        ssh_key::PrivateKey::random(&mut ssh_key::rand_core::OsRng, ssh_key::Algorithm::Ed25519)
            .map_err(|e| anyhow::anyhow!("Failed to generate relay key: {}", e))?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    key.write_openssh_file(path, ssh_key::LineEnding::LF)
        .map_err(|e| anyhow::anyhow!("Failed to write relay key: {}", e))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }

    log::info!("Generated new relay device key at {}", path.display());
    Ok(key)
}
