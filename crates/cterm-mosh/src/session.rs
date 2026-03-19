//! High-level mosh session with event loop.

use std::net::SocketAddr;

use tokio::net::UdpSocket;
use tokio::sync::mpsc;

use crate::crypto::MoshCrypto;
use crate::proto::{HostMessage, UserMessage};
use crate::ssh_launch::launch_mosh_server;
use crate::ssp::SspState;
use crate::{MoshCommand, MoshConfig, MoshError, MoshEvent};

/// A running mosh session.
pub struct MoshSession {
    cmd_tx: mpsc::UnboundedSender<MoshCommand>,
    event_rx: mpsc::UnboundedReceiver<MoshEvent>,
}

impl MoshSession {
    /// Connect to a remote host via mosh.
    ///
    /// 1. Launches mosh-server via SSH
    /// 2. Creates crypto from key
    /// 3. Binds UDP socket, connects to target
    /// 4. Spawns event loop
    pub async fn connect(config: MoshConfig) -> Result<Self, MoshError> {
        // Launch mosh-server via SSH
        let info = launch_mosh_server(
            &config.host,
            config.proxy_jump.as_deref(),
            config.locale.as_deref(),
            config.term.as_deref(),
            &config.ssh_args,
        )
        .await?;

        log::info!(
            "mosh-server started on port {}, key len={}",
            info.port,
            info.key.len()
        );

        // Create crypto
        let crypto = MoshCrypto::new(&info.key)?;

        // Determine target address.
        // If relay (proxy_jump set), connect to the SSH host (relay rewrites the port).
        // Otherwise use the IP from mosh-server or the SSH host.
        let target_host = info
            .ip
            .as_deref()
            .unwrap_or_else(|| extract_hostname(&config.host));

        // Resolve hostname to address
        let target_addr = resolve_addr(target_host, info.port).await?;

        log::info!("connecting UDP to {}", target_addr);

        // Bind UDP socket
        let socket = UdpSocket::bind("0.0.0.0:0")
            .await
            .map_err(|e| MoshError::UdpBindFailed(e.to_string()))?;
        socket
            .connect(target_addr)
            .await
            .map_err(|e| MoshError::UdpConnectFailed(e.to_string()))?;

        let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
        let (event_tx, event_rx) = mpsc::unbounded_channel();

        // Send initial resize
        let _ = cmd_tx.send(MoshCommand::Resize(config.cols, config.rows));

        // Spawn event loop
        tokio::spawn(event_loop(socket, crypto, cmd_rx, event_tx));

        Ok(Self { cmd_tx, event_rx })
    }

    /// Send keystrokes to the remote.
    pub fn write(&self, data: &[u8]) {
        let _ = self.cmd_tx.send(MoshCommand::Write(data.to_vec()));
    }

    /// Notify the remote of terminal resize.
    pub fn resize(&self, cols: u16, rows: u16) {
        let _ = self.cmd_tx.send(MoshCommand::Resize(cols, rows));
    }

    /// Receive the next event from the session.
    pub async fn recv(&mut self) -> Option<MoshEvent> {
        self.event_rx.recv().await
    }

    /// Shut down the session.
    pub fn shutdown(&self) {
        let _ = self.cmd_tx.send(MoshCommand::Shutdown);
    }
}

/// Extract hostname from SSH destination (e.g., "user@host" → "host").
fn extract_hostname(dest: &str) -> &str {
    match dest.rfind('@') {
        Some(idx) => &dest[idx + 1..],
        None => dest,
    }
}

/// Resolve a hostname:port to a SocketAddr.
async fn resolve_addr(host: &str, port: u16) -> Result<SocketAddr, MoshError> {
    use tokio::net::lookup_host;
    let addr_str = format!("{}:{}", host, port);
    let mut addrs = lookup_host(&addr_str)
        .await
        .map_err(|e| MoshError::InvalidAddress(e.to_string()))?;
    addrs
        .next()
        .ok_or_else(|| MoshError::InvalidAddress(format!("no addresses found for {}", host)))
}

/// Main event loop for the mosh session.
async fn event_loop(
    socket: UdpSocket,
    crypto: MoshCrypto,
    mut cmd_rx: mpsc::UnboundedReceiver<MoshCommand>,
    event_tx: mpsc::UnboundedSender<MoshEvent>,
) {
    let mut ssp = SspState::new(crypto);
    let mut recv_buf = [0u8; 65536];

    // Force initial send (keepalive / resize)
    ssp.force_next_send();

    loop {
        let deadline = ssp.next_deadline();

        tokio::select! {
            // Receive UDP datagram
            result = socket.recv(&mut recv_buf) => {
                match result {
                    Ok(n) => {
                        match ssp.recv(&recv_buf[..n]) {
                            Ok(Some(msgs)) => {
                                for msg in msgs {
                                    match msg {
                                        HostMessage::HostBytes(data) => {
                                            if event_tx.send(MoshEvent::Output(data)).is_err() {
                                                return;
                                            }
                                        }
                                        HostMessage::Resize(_, _) | HostMessage::EchoAck(_) => {
                                            // Handled internally by SSP
                                        }
                                    }
                                }
                            }
                            Ok(None) => {} // Fragment not yet complete
                            Err(e) => {
                                log::warn!("mosh recv error: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("UDP recv error: {}", e);
                        let _ = event_tx.send(MoshEvent::Closed(Some(
                            MoshError::UdpRecvFailed(e.to_string()),
                        )));
                        return;
                    }
                }
            }

            // Process commands from the UI
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(MoshCommand::Write(data)) => {
                        ssp.queue(&[UserMessage::Keystroke(data)]);
                    }
                    Some(MoshCommand::Resize(cols, rows)) => {
                        ssp.queue(&[UserMessage::Resize(cols, rows)]);
                    }
                    Some(MoshCommand::Shutdown) | None => {
                        let _ = event_tx.send(MoshEvent::Closed(None));
                        return;
                    }
                }
            }

            // Timer tick — send keepalives / retransmit
            _ = tokio::time::sleep(deadline) => {
                match ssp.tick() {
                    Ok(datagrams) => {
                        for dg in datagrams {
                            if let Err(e) = socket.send(&dg).await {
                                log::warn!("UDP send error: {}", e);
                            }
                        }
                    }
                    Err(e) => {
                        log::error!("SSP tick error: {}", e);
                    }
                }
            }
        }

        // After processing commands/receives, also tick to send any queued data
        match ssp.tick() {
            Ok(datagrams) => {
                for dg in datagrams {
                    if let Err(e) = socket.send(&dg).await {
                        log::warn!("UDP send error: {}", e);
                    }
                }
            }
            Err(e) => {
                log::error!("SSP tick error: {}", e);
            }
        }

        // Check for idle timeout (60 seconds without any data)
        if ssp.idle_time() > std::time::Duration::from_secs(60) {
            log::warn!("mosh session idle for 60s");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_hostname_with_user() {
        assert_eq!(extract_hostname("user@example.com"), "example.com");
    }

    #[test]
    fn extract_hostname_without_user() {
        assert_eq!(extract_hostname("example.com"), "example.com");
    }
}
