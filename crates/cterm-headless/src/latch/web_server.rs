//! Web terminal server: HTTPS + WebSocket + xterm.js.
//!
//! Provides a browser-based terminal using Ed25519 challenge-response
//! authentication over WebSocket. Serves embedded static files for the
//! xterm.js UI over HTTPS.

use crate::latch::auth::AuthorizedKeys;
use crate::latch::bridge::{self, BridgeCommand};
use crate::session::SessionManager;
use cterm_app::config::LatchConfig;
use futures_util::{SinkExt, StreamExt};
use parking_lot::RwLock;
use rust_embed::Embed;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

/// Domain separator for Ed25519 challenge-response authentication.
const AUTH_DOMAIN: &[u8] = b"latch-web-auth-v1:";

/// Embedded static files for the web terminal.
#[derive(Embed)]
#[folder = "src/latch/web/static/"]
struct StaticFiles;

/// A wrapper that prepends buffered data before reading from the inner stream.
/// Used to replay the HTTP request to tokio-tungstenite after we've peeked at it.
struct ReplayStream<S> {
    prefix: Vec<u8>,
    prefix_pos: usize,
    inner: S,
}

impl<S: AsyncRead + Unpin> AsyncRead for ReplayStream<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();

        // First drain the prefix buffer
        if this.prefix_pos < this.prefix.len() {
            let remaining = &this.prefix[this.prefix_pos..];
            let to_copy = remaining.len().min(buf.remaining());
            buf.put_slice(&remaining[..to_copy]);
            this.prefix_pos += to_copy;
            return Poll::Ready(Ok(()));
        }

        // Then read from the inner stream
        Pin::new(&mut this.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for ReplayStream<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

/// Start the web terminal server as a background task.
pub async fn start_web_server(
    config: &LatchConfig,
    authorized_keys: Arc<RwLock<AuthorizedKeys>>,
    session_manager: Arc<SessionManager>,
) -> anyhow::Result<tokio::task::JoinHandle<()>> {
    let addr: SocketAddr = config.web_listen.parse().map_err(|e| {
        anyhow::anyhow!("Invalid web listen address '{}': {}", config.web_listen, e)
    })?;

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
                    let keys = authorized_keys.clone();
                    let sm = session_manager.clone();

                    tokio::spawn(async move {
                        let tls_stream = match acceptor.accept(stream).await {
                            Ok(s) => s,
                            Err(e) => {
                                log::debug!("TLS handshake failed from {}: {}", peer_addr, e);
                                return;
                            }
                        };

                        handle_connection(tls_stream, peer_addr, keys, sm, coalesce_ms).await;
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

/// Handle an HTTP/WebSocket connection.
///
/// Reads the HTTP request, then either serves a static file or
/// upgrades to WebSocket with Ed25519 authentication.
async fn handle_connection(
    mut stream: tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
    peer_addr: SocketAddr,
    authorized_keys: Arc<RwLock<AuthorizedKeys>>,
    session_manager: Arc<SessionManager>,
    coalesce_ms: u64,
) {
    // Read HTTP request into a buffer (max 8KB for headers)
    let mut header_buf = vec![0u8; 8192];
    let mut header_len = 0;
    loop {
        if header_len >= header_buf.len() {
            return; // Headers too large
        }
        let n =
            match tokio::io::AsyncReadExt::read(&mut stream, &mut header_buf[header_len..]).await {
                Ok(0) => return, // Connection closed
                Ok(n) => n,
                Err(_) => return,
            };
        header_len += n;

        // Check for end of HTTP headers (\r\n\r\n)
        if header_len >= 4 {
            if let Some(pos) = find_header_end(&header_buf[..header_len]) {
                header_buf.truncate(header_len);
                let headers_str = String::from_utf8_lossy(&header_buf[..pos]);

                // Parse request line and headers
                let mut lines = headers_str.lines();
                let request_line = match lines.next() {
                    Some(l) => l,
                    None => return,
                };

                let parts: Vec<&str> = request_line.split_whitespace().collect();
                if parts.len() < 2 {
                    return;
                }
                let method = parts[0];
                let path = parts[1];

                let mut is_websocket = false;
                let mut session_name = "default".to_string();

                for line in lines {
                    if let Some((name, value)) = line.split_once(':') {
                        let name_lower = name.trim().to_lowercase();
                        let value_trimmed = value.trim();
                        if name_lower == "upgrade"
                            && value_trimmed.eq_ignore_ascii_case("websocket")
                        {
                            is_websocket = true;
                        }
                    }
                }

                // Extract session name from path query
                if path.starts_with("/ws") {
                    if let Some((_, query)) = path.split_once('?') {
                        for param in query.split('&') {
                            if let Some(("session", v)) = param.split_once('=') {
                                session_name = v.to_string();
                            }
                        }
                    }
                }

                if method == "GET" && is_websocket && path.starts_with("/ws") {
                    // Replay the full HTTP request to tokio-tungstenite
                    let replay = ReplayStream {
                        prefix: header_buf,
                        prefix_pos: 0,
                        inner: stream,
                    };

                    let ws_stream = match tokio_tungstenite::accept_async(replay).await {
                        Ok(s) => s,
                        Err(e) => {
                            log::debug!("WebSocket upgrade failed from {}: {}", peer_addr, e);
                            return;
                        }
                    };

                    handle_authenticated_websocket(
                        ws_stream,
                        &session_name,
                        authorized_keys,
                        session_manager,
                        coalesce_ms,
                    )
                    .await;
                } else if method == "GET" {
                    serve_static_file(&mut stream, path).await;
                }

                return;
            }
        }
    }
}

/// Find the position of the end-of-headers marker (\r\n\r\n).
fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
}

/// Serve an embedded static file over HTTP.
async fn serve_static_file(stream: &mut (impl AsyncWrite + Unpin), path: &str) {
    let file_path = match path {
        "/" | "/index.html" => "index.html",
        _ => path.strip_prefix('/').unwrap_or(path),
    };

    let (status, content_type, body) = if let Some(file) = StaticFiles::get(file_path) {
        let ct = match file_path {
            p if p.ends_with(".html") => "text/html; charset=utf-8",
            p if p.ends_with(".js") => "application/javascript",
            p if p.ends_with(".css") => "text/css",
            _ => "application/octet-stream",
        };
        ("200 OK", ct, file.data.to_vec())
    } else {
        ("404 Not Found", "text/plain", b"404 Not Found".to_vec())
    };

    let response = format!(
        "HTTP/1.1 {status}\r\n\
         Content-Type: {content_type}\r\n\
         Content-Length: {}\r\n\
         Content-Security-Policy: default-src 'self'; script-src 'self' 'unsafe-inline'; connect-src wss: ws:;\r\n\
         X-Content-Type-Options: nosniff\r\n\
         X-Frame-Options: DENY\r\n\
         Connection: close\r\n\
         \r\n",
        body.len(),
    );

    let _ = tokio::io::AsyncWriteExt::write_all(stream, response.as_bytes()).await;
    let _ = tokio::io::AsyncWriteExt::write_all(stream, &body).await;
    let _ = tokio::io::AsyncWriteExt::flush(stream).await;
}

/// Handle a WebSocket with Ed25519 challenge-response auth.
async fn handle_authenticated_websocket<S>(
    mut ws: tokio_tungstenite::WebSocketStream<S>,
    session_name: &str,
    authorized_keys: Arc<RwLock<AuthorizedKeys>>,
    session_manager: Arc<SessionManager>,
    coalesce_ms: u64,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    // Reload authorized keys
    {
        let mut keys = authorized_keys.write();
        let _ = keys.reload();
    }

    // Generate 32-byte challenge
    let mut challenge = [0u8; 32];
    getrandom::fill(&mut challenge).expect("getrandom failed");

    // Send challenge: [0x01][32-byte challenge]
    let mut challenge_msg = Vec::with_capacity(33);
    challenge_msg.push(0x01);
    challenge_msg.extend_from_slice(&challenge);
    if ws
        .send(Message::Binary(challenge_msg.into()))
        .await
        .is_err()
    {
        return;
    }

    // Wait for auth response (30s timeout)
    let auth_result = tokio::time::timeout(std::time::Duration::from_secs(30), async {
        while let Some(Ok(msg)) = ws.next().await {
            if let Message::Binary(data) = msg {
                return verify_auth_response(&data, &challenge, &authorized_keys);
            }
        }
        false
    })
    .await;

    if matches!(auth_result, Ok(true)) {
        let _ = ws.send(Message::Binary(vec![0x00].into())).await;
        handle_websocket_session(ws, session_name, session_manager, coalesce_ms).await;
    } else {
        let _ = ws.send(Message::Binary(vec![0x02].into())).await;
        log::warn!("Web auth failed for session '{}'", session_name);
    }
}

/// Verify Ed25519 auth response.
///
/// Format: [4-byte pubkey_len BE][pubkey (OpenSSH wire)][4-byte sig_len BE][sig (64 bytes)]
fn verify_auth_response(
    data: &[u8],
    challenge: &[u8; 32],
    authorized_keys: &Arc<RwLock<AuthorizedKeys>>,
) -> bool {
    if data.len() < 8 {
        return false;
    }

    let pk_len = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
    if data.len() < 4 + pk_len + 4 {
        return false;
    }

    let pk_bytes = &data[4..4 + pk_len];
    let sig_offset = 4 + pk_len;
    let sig_len = u32::from_be_bytes([
        data[sig_offset],
        data[sig_offset + 1],
        data[sig_offset + 2],
        data[sig_offset + 3],
    ]) as usize;
    if data.len() < sig_offset + 4 + sig_len {
        return false;
    }
    let sig_bytes = &data[sig_offset + 4..sig_offset + 4 + sig_len];

    // Extract raw Ed25519 key from OpenSSH wire format
    let raw_pk = match extract_ed25519_raw_key(pk_bytes) {
        Some(k) if k.len() == 32 => k,
        _ => return false,
    };

    // Build OpenSSH string for authorized_keys check
    use base64::Engine;
    let pk_b64 = base64::engine::general_purpose::STANDARD.encode(pk_bytes);
    let openssh_str = format!("ssh-ed25519 {}", pk_b64);
    let russh_key = match russh::keys::PublicKey::from_openssh(&openssh_str) {
        Ok(k) => k,
        Err(_) => return false,
    };

    if !authorized_keys.read().contains(&russh_key) {
        return false;
    }

    // Verify signature: sign_data = AUTH_DOMAIN + challenge
    let mut sign_data = Vec::with_capacity(AUTH_DOMAIN.len() + challenge.len());
    sign_data.extend_from_slice(AUTH_DOMAIN);
    sign_data.extend_from_slice(challenge);

    if sig_bytes.len() != 64 {
        return false;
    }

    use ed25519_dalek::{Signature, Verifier, VerifyingKey};
    let raw_pk_arr: [u8; 32] = raw_pk.try_into().unwrap();
    let vk = match VerifyingKey::from_bytes(&raw_pk_arr) {
        Ok(k) => k,
        Err(_) => return false,
    };
    let sig = match Signature::from_slice(sig_bytes) {
        Ok(s) => s,
        Err(_) => return false,
    };

    vk.verify(&sign_data, &sig).is_ok()
}

/// Extract raw 32-byte Ed25519 key from OpenSSH wire format.
/// Wire: [4-byte type_len][type_str][4-byte key_len][key_bytes]
fn extract_ed25519_raw_key(wire: &[u8]) -> Option<Vec<u8>> {
    if wire.len() < 4 {
        return None;
    }
    let type_len = u32::from_be_bytes([wire[0], wire[1], wire[2], wire[3]]) as usize;
    let key_offset = 4 + type_len;
    if wire.len() < key_offset + 4 {
        return None;
    }
    let key_len = u32::from_be_bytes([
        wire[key_offset],
        wire[key_offset + 1],
        wire[key_offset + 2],
        wire[key_offset + 3],
    ]) as usize;
    if wire.len() < key_offset + 4 + key_len {
        return None;
    }
    Some(wire[key_offset + 4..key_offset + 4 + key_len].to_vec())
}

/// Handle a WebSocket terminal session (post-auth).
async fn handle_websocket_session<S>(
    ws: tokio_tungstenite::WebSocketStream<S>,
    session_name: &str,
    session_manager: Arc<SessionManager>,
    coalesce_ms: u64,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut ws_tx, mut ws_rx) = ws.split();

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

    let (cmd_tx, mut output_rx) = bridge::spawn_bridge(session, coalesce_ms);

    let ws_output_task = tokio::spawn(async move {
        while let Some(data) = output_rx.recv().await {
            if ws_tx.send(Message::Binary(data.into())).await.is_err() {
                break;
            }
        }
    });

    while let Some(Ok(msg)) = ws_rx.next().await {
        match msg {
            Message::Binary(data) => {
                let data = data.as_ref();
                if data.len() >= 5 && data[0] == 0x05 {
                    let cols = u16::from_be_bytes([data[1], data[2]]);
                    let rows = u16::from_be_bytes([data[3], data[4]]);
                    let _ = cmd_tx
                        .send(BridgeCommand::Resize(cols as u32, rows as u32))
                        .await;
                } else if !data.is_empty() && data[0] == 0x13 {
                    let _ = cmd_tx.send(BridgeCommand::Input(data[1..].to_vec())).await;
                } else {
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

    if cert_path.exists() && key_path.exists() {
        if let Ok(tls_config) = load_tls_config(&cert_path, &key_path) {
            log::info!("Loaded TLS certificate from {}", cert_path.display());
            return Ok(tls_config);
        }
    }

    let subject_alt_names = vec!["localhost".to_string()];
    let certified = rcgen::generate_simple_self_signed(subject_alt_names)?;

    let cert_pem = certified.cert.pem();
    let key_pem = certified.signing_key.serialize_pem();

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

fn load_tls_config(
    cert_path: &std::path::Path,
    key_path: &std::path::Path,
) -> anyhow::Result<rustls::ServerConfig> {
    let cert_pem = std::fs::read(cert_path)?;
    let key_pem = std::fs::read(key_path)?;

    let certs: Vec<_> = rustls_pemfile::certs(&mut &cert_pem[..]).collect::<Result<_, _>>()?;
    let key = rustls_pemfile::private_key(&mut &key_pem[..])?
        .ok_or_else(|| anyhow::anyhow!("No private key found in {}", key_path.display()))?;

    Ok(rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?)
}
