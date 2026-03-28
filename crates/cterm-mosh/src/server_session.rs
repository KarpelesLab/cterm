//! Mosh server session: UDP listener bridged to a terminal session.

use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::sync::mpsc;

use crate::crypto::MoshCrypto;
use crate::proto::{HostMessage, UserMessage};
use crate::ssp_server::SspServerState;
use crate::MoshError;

/// Commands sent to the mosh server session.
#[derive(Debug)]
pub enum MoshServerCommand {
    /// Send terminal output to the mosh client.
    Output(Vec<u8>),
    /// Notify the client of a terminal resize.
    Resize(u16, u16),
    /// Shut down the mosh server session.
    Shutdown,
}

/// Events received from the mosh server session.
#[derive(Debug)]
pub enum MoshServerEvent {
    /// Client sent keystrokes.
    Input(Vec<u8>),
    /// Client requests a resize.
    Resize(u16, u16),
    /// Session closed.
    Closed(Option<MoshError>),
}

/// Configuration for the mosh server.
pub struct MoshServerConfig {
    /// Base64-encoded 128-bit AES key.
    pub key: String,
    /// UDP port to listen on.
    pub port: u16,
    /// Bind address (e.g., "0.0.0.0").
    pub bind_addr: String,
}

/// A running mosh server session.
pub struct MoshServerSession {
    /// The UDP port the server is listening on.
    pub port: u16,
    /// The shared key (base64).
    pub key: String,
    /// Channel to send commands to the session.
    pub cmd_tx: mpsc::Sender<MoshServerCommand>,
    /// Channel to receive events from the session.
    pub event_rx: mpsc::Receiver<MoshServerEvent>,
}

impl MoshServerSession {
    /// Start a mosh server session.
    ///
    /// Binds a UDP socket, waits for the first valid client datagram,
    /// then runs the SSP event loop.
    pub async fn start(config: MoshServerConfig) -> Result<Self, MoshError> {
        let crypto = MoshCrypto::new(&config.key)?;
        let bind = format!("{}:{}", config.bind_addr, config.port);
        let socket = UdpSocket::bind(&bind)
            .await
            .map_err(|e| MoshError::UdpBindFailed(e.to_string()))?;

        let (cmd_tx, cmd_rx) = mpsc::channel(256);
        let (event_tx, event_rx) = mpsc::channel(256);

        let port = config.port;
        let key = config.key.clone();

        tokio::spawn(async move {
            if let Err(e) = run_server_loop(socket, crypto, cmd_rx, event_tx).await {
                log::error!("Mosh server session error: {}", e);
            }
        });

        Ok(Self {
            port,
            key,
            cmd_tx,
            event_rx,
        })
    }
}

/// Generate a random 128-bit key as base64.
pub fn generate_mosh_key() -> String {
    use base64::Engine;
    let mut key = [0u8; 16];
    // Use getrandom for crypto-safe random
    getrandom::fill(&mut key).expect("getrandom failed");
    base64::engine::general_purpose::STANDARD.encode(key)
}

/// Find a free UDP port in the given range.
pub async fn find_free_port(start: u16, end: u16) -> Result<u16, MoshError> {
    for port in start..=end {
        match UdpSocket::bind(format!("0.0.0.0:{}", port)).await {
            Ok(_) => return Ok(port),
            Err(_) => continue,
        }
    }
    Err(MoshError::UdpBindFailed(format!(
        "no free port in range {}-{}",
        start, end
    )))
}

/// Main server event loop.
async fn run_server_loop(
    socket: UdpSocket,
    crypto: MoshCrypto,
    mut cmd_rx: mpsc::Receiver<MoshServerCommand>,
    event_tx: mpsc::Sender<MoshServerEvent>,
) -> Result<(), MoshError> {
    let mut ssp = SspServerState::new(crypto);
    let mut buf = [0u8; 65536];
    let mut client_addr: Option<SocketAddr> = None;

    loop {
        let deadline = ssp.next_deadline();

        tokio::select! {
            // Receive UDP datagrams
            result = socket.recv_from(&mut buf) => {
                match result {
                    Ok((n, addr)) => {
                        // Try to decrypt — if successful, this is our client
                        match ssp.recv(&buf[..n]) {
                            Ok(Some(msgs)) => {
                                // Lock to this client address on first success
                                if client_addr.is_none() {
                                    client_addr = Some(addr);
                                    log::info!("Mosh client connected from {}", addr);
                                }
                                for msg in msgs {
                                    match msg {
                                        UserMessage::Keystroke(data) => {
                                            if event_tx.send(MoshServerEvent::Input(data)).await.is_err() {
                                                return Ok(());
                                            }
                                        }
                                        UserMessage::Resize(cols, rows) => {
                                            if event_tx.send(MoshServerEvent::Resize(cols, rows)).await.is_err() {
                                                return Ok(());
                                            }
                                        }
                                    }
                                }
                            }
                            Ok(None) => {} // Fragment pending, not yet complete
                            Err(_) => {} // Decrypt failed, ignore (could be a probe)
                        }
                    }
                    Err(e) => {
                        log::error!("UDP recv error: {}", e);
                        break;
                    }
                }
            }

            // Handle commands from the terminal bridge
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(MoshServerCommand::Output(data)) => {
                        ssp.queue(&[HostMessage::HostBytes(data)]);
                    }
                    Some(MoshServerCommand::Resize(cols, rows)) => {
                        ssp.queue(&[HostMessage::Resize(cols, rows)]);
                    }
                    Some(MoshServerCommand::Shutdown) | None => {
                        break;
                    }
                }
            }

            // SSP tick timer
            _ = tokio::time::sleep(deadline) => {}
        }

        // Send any pending datagrams
        if let Some(addr) = client_addr {
            match ssp.tick() {
                Ok(datagrams) => {
                    for dg in datagrams {
                        if let Err(e) = socket.send_to(&dg, addr).await {
                            log::error!("UDP send error: {}", e);
                        }
                    }
                }
                Err(e) => {
                    log::error!("SSP tick error: {}", e);
                    break;
                }
            }
        }

        // Check for idle timeout (no client activity for 60s)
        if ssp.idle_time() > Duration::from_secs(60) && client_addr.is_some() {
            log::info!("Mosh client idle timeout");
            break;
        }
    }

    let _ = event_tx.send(MoshServerEvent::Closed(None)).await;
    Ok(())
}
