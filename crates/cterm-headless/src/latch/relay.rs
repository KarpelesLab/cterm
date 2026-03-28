//! Relay client: persistent QUIC connection to relay server for NAT traversal.
//!
//! Maintains a QUIC connection to relay.unixshells.com, authenticates
//! with an Ed25519 device key, and accepts incoming streams for SSH,
//! mosh UDP bridging, and web connections.

use crate::latch::auth::AuthorizedKeys;
use crate::latch::bridge::{self, BridgeCommand};
use crate::session::SessionManager;
use cterm_app::config::LatchConfig;
use parking_lot::RwLock;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::AsyncReadExt;

/// ALPN protocol for the relay QUIC connection.
const RELAY_ALPN: &str = "latch-relay";

/// Domain separator for relay authentication signatures.
const AUTH_DOMAIN: &[u8] = b"latch-relay-auth-v1:";

/// Stream type bytes.
const STREAM_TYPE_SSH: u8 = 0x00;
const STREAM_TYPE_UDP: u8 = 0x01;

/// Start the relay client as a background task.
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
    let coalesce_ms = config.render_coalesce_ms;

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

                    accept_loop(connection, &authorized_keys, &session_manager, coalesce_ms).await;

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

/// Connect and authenticate with the relay server.
async fn connect_relay(
    host: &str,
    username: &str,
    device: &str,
    key_path: &std::path::Path,
) -> anyhow::Result<quinn::Connection> {
    let device_key = load_or_generate_device_key(key_path)?;

    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let mut tls_config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    tls_config.alpn_protocols = vec![RELAY_ALPN.as_bytes().to_vec()];

    let client_config = quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(tls_config)?,
    ));

    let addr: SocketAddr = tokio::net::lookup_host(host)
        .await?
        .next()
        .ok_or_else(|| anyhow::anyhow!("Failed to resolve {}", host))?;

    let endpoint = quinn::Endpoint::client("0.0.0.0:0".parse()?)?;
    let server_name = host.split(':').next().unwrap_or(host);
    let connection = endpoint
        .connect_with(client_config, addr, server_name)?
        .await?;

    authenticate(&connection, username, device, &device_key).await?;

    Ok(connection)
}

/// Perform the relay authentication handshake with Ed25519 signing.
async fn authenticate(
    connection: &quinn::Connection,
    username: &str,
    device: &str,
    device_key: &ssh_key::PrivateKey,
) -> anyhow::Result<()> {
    let (mut send, mut recv) = connection.open_bi().await?;

    // Step 1: Send identity [userLen:1][user][deviceLen:1][device][pubkeyLen:2][pubkey]
    let pubkey = device_key.public_key();
    let pubkey_openssh = pubkey
        .to_openssh()
        .map_err(|e| anyhow::anyhow!("Failed to encode public key: {}", e))?;
    // Extract just the key data (type + base64) from the OpenSSH string
    let pubkey_wire = pubkey_openssh.as_bytes();

    let mut identity = Vec::new();
    identity.push(username.len() as u8);
    identity.extend_from_slice(username.as_bytes());
    identity.push(device.len() as u8);
    identity.extend_from_slice(device.as_bytes());
    let pk_len = (pubkey_wire.len() as u16).to_be_bytes();
    identity.extend_from_slice(&pk_len);
    identity.extend_from_slice(pubkey_wire);

    send.write_all(&identity).await?;

    // Step 2: Read challenge
    let mut challenge_len_buf = [0u8; 1];
    recv.read_exact(&mut challenge_len_buf).await?;
    let challenge_len = challenge_len_buf[0] as usize;
    let mut challenge = vec![0u8; challenge_len];
    recv.read_exact(&mut challenge).await?;

    // Step 3: Sign challenge with Ed25519
    // Sign data = AUTH_DOMAIN + challenge
    let mut sign_data = Vec::with_capacity(AUTH_DOMAIN.len() + challenge.len());
    sign_data.extend_from_slice(AUTH_DOMAIN);
    sign_data.extend_from_slice(&challenge);

    // Use ed25519-dalek for signing
    let ed_key = match device_key.key_data() {
        ssh_key::private::KeypairData::Ed25519(kp) => kp,
        _ => return Err(anyhow::anyhow!("Device key must be Ed25519")),
    };

    let signing_key = ed25519_dalek::SigningKey::from_bytes(&ed_key.private.to_bytes());
    use ed25519_dalek::Signer;
    let signature = signing_key.sign(&sign_data);

    // SSH signature format: marshal as SSH signature wire format
    // [4-byte "ssh-ed25519" len]["ssh-ed25519"][4-byte sig len][64-byte sig]
    let sig_type = b"ssh-ed25519";
    let sig_bytes = signature.to_bytes();
    let mut ssh_sig = Vec::new();
    ssh_sig.extend_from_slice(&(sig_type.len() as u32).to_be_bytes());
    ssh_sig.extend_from_slice(sig_type);
    ssh_sig.extend_from_slice(&(sig_bytes.len() as u32).to_be_bytes());
    ssh_sig.extend_from_slice(&sig_bytes);

    let sig_len = (ssh_sig.len() as u16).to_be_bytes();
    send.write_all(&sig_len).await?;
    send.write_all(&ssh_sig).await?;
    send.finish()?;

    // Step 4: Read result
    let mut result = [0u8; 1];
    recv.read_exact(&mut result).await?;
    if result[0] != 1 {
        return Err(anyhow::anyhow!("auth rejected by relay server"));
    }

    log::info!("Relay authentication successful");
    Ok(())
}

/// Accept and dispatch incoming relay streams.
async fn accept_loop(
    connection: quinn::Connection,
    authorized_keys: &Arc<RwLock<AuthorizedKeys>>,
    session_manager: &Arc<SessionManager>,
    coalesce_ms: u64,
) {
    loop {
        match connection.accept_bi().await {
            Ok((_send, mut recv)) => {
                let mut type_buf = [0u8; 1];
                if recv.read_exact(&mut type_buf).await.is_err() {
                    continue;
                }

                match type_buf[0] {
                    STREAM_TYPE_SSH => {
                        // Read client IP header: [2-byte len][ip_string]
                        let mut ip_len_buf = [0u8; 2];
                        if recv.read_exact(&mut ip_len_buf).await.is_err() {
                            continue;
                        }
                        let ip_len = u16::from_be_bytes(ip_len_buf) as usize;
                        if ip_len > 512 {
                            continue;
                        }
                        let mut ip_buf = vec![0u8; ip_len];
                        if recv.read_exact(&mut ip_buf).await.is_err() {
                            continue;
                        }
                        let client_ip = String::from_utf8_lossy(&ip_buf).to_string();
                        log::info!("Relay: SSH stream from {}", client_ip);

                        // The remaining data on this stream is raw SSH protocol.
                        // This would be fed into russh as a socket.
                        // For now, we create a direct session bridge.
                        let sm = session_manager.clone();
                        let _keys = authorized_keys.clone();
                        tokio::spawn(async move {
                            handle_relay_ssh_stream(sm, _send, recv, &client_ip, coalesce_ms).await;
                        });
                    }
                    STREAM_TYPE_UDP => {
                        // Read target port and client IP
                        let mut port_buf = [0u8; 2];
                        if recv.read_exact(&mut port_buf).await.is_err() {
                            continue;
                        }
                        let target_port = u16::from_be_bytes(port_buf);
                        let mut ip_len_buf = [0u8; 2];
                        if recv.read_exact(&mut ip_len_buf).await.is_err() {
                            continue;
                        }
                        let ip_len = u16::from_be_bytes(ip_len_buf) as usize;
                        let mut ip_buf = vec![0u8; ip_len];
                        if recv.read_exact(&mut ip_buf).await.is_err() {
                            continue;
                        }
                        let client_ip = String::from_utf8_lossy(&ip_buf).to_string();

                        log::info!(
                            "Relay: UDP bridge for port {} from {}",
                            target_port,
                            client_ip
                        );

                        // Bridge UDP datagrams between QUIC stream and local UDP port
                        tokio::spawn(async move {
                            handle_relay_udp_bridge(_send, recv, target_port).await;
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

/// Handle a relay SSH stream by creating a terminal session bridge.
async fn handle_relay_ssh_stream(
    session_manager: Arc<SessionManager>,
    mut send: quinn::SendStream,
    mut recv: quinn::RecvStream,
    client_ip: &str,
    coalesce_ms: u64,
) {
    // For relay SSH, we create a named session and bridge I/O directly.
    // The relay SSH stream carries raw terminal data (not SSH protocol),
    // since the SSH handshake happens between the real client and the
    // relay's SSH forwarder.
    let session_name = "relay";

    let session = match session_manager.get_or_create_named_session(
        session_name,
        80,
        24,
        None,
        Vec::new(),
        None,
    ) {
        Ok(s) => s,
        Err(e) => {
            log::error!("Failed to create relay session: {}", e);
            return;
        }
    };

    let (cmd_tx, mut output_rx) = bridge::spawn_bridge(session, coalesce_ms);

    // Forward bridge output to QUIC send stream
    let output_task = tokio::spawn(async move {
        while let Some(data) = output_rx.recv().await {
            if send.write_all(&data).await.is_err() {
                break;
            }
        }
    });

    // Forward QUIC recv stream to bridge input
    let mut buf = [0u8; 8192];
    loop {
        match recv.read(&mut buf).await {
            Ok(Some(n)) if n > 0 => {
                if cmd_tx
                    .send(BridgeCommand::Input(buf[..n].to_vec()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
            _ => break,
        }
    }

    let _ = cmd_tx.send(BridgeCommand::Disconnect).await;
    output_task.abort();
    log::info!("Relay SSH stream from {} closed", client_ip);
}

/// Bridge UDP datagrams between a QUIC stream and a local UDP port.
///
/// Frame format on QUIC: [2-byte len BE][datagram]
async fn handle_relay_udp_bridge(
    mut send: quinn::SendStream,
    mut recv: quinn::RecvStream,
    target_port: u16,
) {
    // Connect to local UDP port (the mosh server)
    let local_socket = match tokio::net::UdpSocket::bind("127.0.0.1:0").await {
        Ok(s) => Arc::new(s),
        Err(e) => {
            log::error!("Failed to bind UDP socket for relay bridge: {}", e);
            return;
        }
    };
    let target_addr: SocketAddr = format!("127.0.0.1:{}", target_port).parse().unwrap();

    // QUIC → local UDP
    let socket_recv = local_socket.clone();
    let quic_to_udp = tokio::spawn(async move {
        loop {
            let mut len_buf = [0u8; 2];
            if recv.read_exact(&mut len_buf).await.is_err() {
                break;
            }
            let len = u16::from_be_bytes(len_buf) as usize;
            let mut data = vec![0u8; len];
            if recv.read_exact(&mut data).await.is_err() {
                break;
            }
            if socket_recv.send_to(&data, target_addr).await.is_err() {
                break;
            }
        }
    });

    // Local UDP → QUIC
    let udp_to_quic = tokio::spawn(async move {
        let mut buf = [0u8; 65536];
        loop {
            match local_socket.recv(&mut buf).await {
                Ok(n) => {
                    let len = (n as u16).to_be_bytes();
                    if send.write_all(&len).await.is_err() {
                        break;
                    }
                    if send.write_all(&buf[..n]).await.is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    tokio::select! {
        _ = quic_to_udp => {},
        _ = udp_to_quic => {},
    }
}

/// Load or generate the device Ed25519 key for relay authentication.
fn load_or_generate_device_key(path: &std::path::Path) -> anyhow::Result<ssh_key::PrivateKey> {
    if path.exists() {
        let key = ssh_key::PrivateKey::read_openssh_file(path)
            .map_err(|e| anyhow::anyhow!("Failed to load relay key: {}", e))?;
        return Ok(key);
    }

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
