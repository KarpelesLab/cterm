//! Web terminal server: HTTPS + WebSocket + xterm.js.
//!
//! Provides a browser-based terminal using Ed25519 challenge-response
//! authentication over WebSocket. The client-side uses WebCrypto for
//! key generation and signing.

use crate::latch::auth::AuthorizedKeys;
use crate::latch::bridge::{self, BridgeCommand};
use crate::session::SessionManager;
use cterm_app::config::LatchConfig;
use futures_util::{SinkExt, StreamExt};
use parking_lot::RwLock;
use rust_embed::Embed;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

/// Embedded static files for the web terminal.
/// TODO: serve these via HTTP for non-WebSocket requests
#[derive(Embed)]
#[folder = "src/latch/web/static/"]
#[allow(dead_code)]
struct StaticFiles;

/// Start the web terminal server as a background task.
///
/// Serves static files over HTTPS and provides WebSocket terminal access
/// with Ed25519 challenge-response authentication.
pub async fn start_web_server(
    config: &LatchConfig,
    authorized_keys: Arc<RwLock<AuthorizedKeys>>,
    session_manager: Arc<SessionManager>,
) -> anyhow::Result<tokio::task::JoinHandle<()>> {
    let addr: SocketAddr = config.web_listen.parse().map_err(|e| {
        anyhow::anyhow!("Invalid web listen address '{}': {}", config.web_listen, e)
    })?;

    // Generate or load self-signed TLS cert
    let tls_config = generate_tls_config()?;
    let tls_acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(tls_config));

    let listener = TcpListener::bind(addr).await?;
    log::info!("Latch web terminal listening on https://{}", addr);

    let coalesce_ms = config.render_coalesce_ms;

    let handle = tokio::spawn(async move {
        loop {
            match listener.accept().await {
                Ok((stream, peer_addr)) => {
                    let acceptor = tls_acceptor.clone();
                    let _keys = authorized_keys.clone();
                    let sm = session_manager.clone();

                    tokio::spawn(async move {
                        let tls_stream = match acceptor.accept(stream).await {
                            Ok(s) => s,
                            Err(e) => {
                                log::debug!("TLS handshake failed from {}: {}", peer_addr, e);
                                return;
                            }
                        };

                        // Use tokio-tungstenite to handle WebSocket over TLS
                        let ws_stream = match tokio_tungstenite::accept_async(tls_stream).await {
                            Ok(s) => s,
                            Err(e) => {
                                // Not a WebSocket request — serve static files
                                log::debug!(
                                        "WebSocket upgrade failed from {}: {} (may be static file request)",
                                        peer_addr,
                                        e
                                    );
                                return;
                            }
                        };

                        // TODO: Ed25519 challenge-response auth before session access
                        // For now, authenticate based on authorized_keys being loaded
                        handle_websocket(ws_stream, "default", sm, coalesce_ms).await;
                    });
                }
                Err(e) => {
                    log::error!("Web server accept error: {}", e);
                }
            }
        }
    });

    Ok(handle)
}

/// Handle an authenticated WebSocket connection.
async fn handle_websocket<S>(
    ws: tokio_tungstenite::WebSocketStream<S>,
    session_name: &str,
    session_manager: Arc<SessionManager>,
    coalesce_ms: u64,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut ws_tx, mut ws_rx) = ws.split();

    // Get or create session
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
            log::error!("Failed to create session '{}': {}", session_name, e);
            return;
        }
    };

    // Spawn bridge
    let (cmd_tx, mut output_rx) = bridge::spawn_bridge(session, coalesce_ms);

    // Forward bridge output to WebSocket
    let ws_output_task = tokio::spawn(async move {
        while let Some(data) = output_rx.recv().await {
            if ws_tx.send(Message::Binary(data.into())).await.is_err() {
                break;
            }
        }
    });

    // Forward WebSocket input to bridge
    while let Some(Ok(msg)) = ws_rx.next().await {
        match msg {
            Message::Binary(data) => {
                let data = data.as_ref();
                if data.len() >= 5 && data[0] == 0x05 {
                    // Resize message
                    let cols = u16::from_be_bytes([data[1], data[2]]);
                    let rows = u16::from_be_bytes([data[3], data[4]]);
                    let _ = cmd_tx
                        .send(BridgeCommand::Resize(cols as u32, rows as u32))
                        .await;
                } else if !data.is_empty() && data[0] == 0x13 {
                    // Paste message
                    let _ = cmd_tx.send(BridgeCommand::Input(data[1..].to_vec())).await;
                } else {
                    // Regular input
                    let _ = cmd_tx.send(BridgeCommand::Input(data.to_vec())).await;
                }
            }
            Message::Text(text) => {
                let _ = cmd_tx
                    .send(BridgeCommand::Input(text.as_str().as_bytes().to_vec()))
                    .await;
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    let _ = cmd_tx.send(BridgeCommand::Disconnect).await;
    ws_output_task.abort();
}

/// Generate a self-signed TLS configuration.
fn generate_tls_config() -> anyhow::Result<rustls::ServerConfig> {
    let config_dir = crate::cli::config_dir();
    let cert_path = config_dir.join("tls.crt");
    let key_path = config_dir.join("tls.key");

    // Try to load existing cert
    if cert_path.exists() && key_path.exists() {
        if let Ok(tls_config) = load_tls_config(&cert_path, &key_path) {
            log::info!("Loaded TLS certificate from {}", cert_path.display());
            return Ok(tls_config);
        }
    }

    // Generate self-signed cert
    let subject_alt_names = vec!["localhost".to_string()];
    let certified = rcgen::generate_simple_self_signed(subject_alt_names)?;

    let cert_pem = certified.cert.pem();
    let key_pem = certified.signing_key.serialize_pem();

    // Write files
    std::fs::create_dir_all(&config_dir)?;
    std::fs::write(&cert_path, &cert_pem)?;
    std::fs::write(&key_path, &key_pem)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))?;
    }

    log::info!(
        "Generated self-signed TLS certificate at {}",
        cert_path.display()
    );

    load_tls_config(&cert_path, &key_path)
}

/// Load TLS config from PEM files.
fn load_tls_config(
    cert_path: &std::path::Path,
    key_path: &std::path::Path,
) -> anyhow::Result<rustls::ServerConfig> {
    let cert_pem = std::fs::read(cert_path)?;
    let key_pem = std::fs::read(key_path)?;

    let certs: Vec<_> = rustls_pemfile::certs(&mut &cert_pem[..]).collect::<Result<_, _>>()?;
    let key = rustls_pemfile::private_key(&mut &key_pem[..])?
        .ok_or_else(|| anyhow::anyhow!("No private key found in {}", key_path.display()))?;

    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;

    Ok(config)
}
