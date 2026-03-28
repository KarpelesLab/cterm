//! SSH session handler implementing russh::server::Handler.

use crate::latch::auth::AuthorizedKeys;
use crate::latch::bridge::{self, BridgeCommand};
use crate::session::SessionManager;
use cterm_app::config::LatchConfig;
use parking_lot::RwLock;
use russh::keys::PublicKey;
use russh::server::{Auth, Handle, Msg, Session};
use russh::{Channel, ChannelId};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Per-connection SSH session handler.
pub struct SshSessionHandler {
    session_manager: Arc<SessionManager>,
    authorized_keys: Arc<RwLock<AuthorizedKeys>>,
    config: LatchConfig,
    peer_addr: Option<std::net::SocketAddr>,

    /// Username from SSH auth (becomes the session name).
    username: Option<String>,

    /// Active bridges per channel.
    bridges: HashMap<ChannelId, ChannelBridge>,

    /// PTY dimensions from pty_request (stored until shell_request).
    pending_pty: Option<(u32, u32)>,
}

/// State for an active SSH channel bridged to a session.
struct ChannelBridge {
    cmd_tx: mpsc::Sender<BridgeCommand>,
    /// Task forwarding bridge output to SSH channel.
    output_task: tokio::task::JoinHandle<()>,
}

impl SshSessionHandler {
    pub fn new(
        session_manager: Arc<SessionManager>,
        authorized_keys: Arc<RwLock<AuthorizedKeys>>,
        config: LatchConfig,
        peer_addr: Option<std::net::SocketAddr>,
    ) -> Self {
        Self {
            session_manager,
            authorized_keys,
            config,
            peer_addr,
            username: None,
            bridges: HashMap::new(),
            pending_pty: None,
        }
    }

    /// Start the output forwarding task: reads from bridge output and
    /// writes to the SSH channel via the session handle.
    fn start_output_forwarder(
        channel_id: ChannelId,
        handle: Handle,
        mut output_rx: mpsc::Receiver<Vec<u8>>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            while let Some(data) = output_rx.recv().await {
                if handle.data(channel_id, data).await.is_err() {
                    break;
                }
            }
            // Send EOF and close
            let _ = handle.eof(channel_id).await;
            let _ = handle.close(channel_id).await;
        })
    }
}

impl Drop for SshSessionHandler {
    fn drop(&mut self) {
        // Clean up all bridges on disconnect
        for (_, bridge) in self.bridges.drain() {
            let _ = bridge.cmd_tx.try_send(BridgeCommand::Disconnect);
            bridge.output_task.abort();
        }
    }
}

impl russh::server::Handler for SshSessionHandler {
    type Error = anyhow::Error;

    async fn auth_publickey(
        &mut self,
        user: &str,
        public_key: &PublicKey,
    ) -> Result<Auth, Self::Error> {
        // Reload authorized keys on each auth attempt
        {
            let mut keys = self.authorized_keys.write();
            if let Err(e) = keys.reload() {
                log::warn!("Failed to reload authorized keys: {}", e);
            }
        }

        let authorized = self.authorized_keys.read().contains(public_key);
        if authorized {
            log::info!("SSH auth success for '{}' from {:?}", user, self.peer_addr);
            self.username = Some(user.to_string());
            Ok(Auth::Accept)
        } else {
            log::warn!("SSH auth rejected for '{}' from {:?}", user, self.peer_addr);
            Ok(Auth::Reject {
                proceed_with_methods: None,
                partial_success: false,
            })
        }
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<Msg>,
        _session: &mut Session,
    ) -> Result<bool, Self::Error> {
        Ok(true)
    }

    async fn pty_request(
        &mut self,
        channel: ChannelId,
        _term: &str,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        _modes: &[(russh::Pty, u32)],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        self.pending_pty = Some((col_width, row_height));
        session.channel_success(channel)?;
        Ok(())
    }

    async fn shell_request(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let session_name = self
            .username
            .clone()
            .unwrap_or_else(|| "default".to_string());

        let (cols, rows) = self.pending_pty.unwrap_or((80, 24));

        // Get or create the named session
        let terminal_session = self.session_manager.get_or_create_named_session(
            &session_name,
            cols as usize,
            rows as usize,
            None,
            Vec::new(),
            None,
        )?;

        // Spawn bridge
        let (cmd_tx, output_rx) =
            bridge::spawn_bridge(terminal_session, self.config.render_coalesce_ms);

        // Start output forwarding: bridge → SSH channel
        let handle = session.handle();
        let output_task = Self::start_output_forwarder(channel, handle, output_rx);

        self.bridges.insert(
            channel,
            ChannelBridge {
                cmd_tx,
                output_task,
            },
        );

        session.channel_success(channel)?;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        let command = String::from_utf8_lossy(data);
        log::info!("SSH exec request: {}", command);

        // Check if this is a mosh-server request
        if command.contains("mosh-server") {
            // TODO: Phase 3 — mosh server handoff
            log::warn!("Mosh server exec not yet implemented");
            session.channel_failure(channel)?;
            return Ok(());
        }

        // For now, reject other exec requests
        session.channel_failure(channel)?;
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(bridge) = self.bridges.get(&channel) {
            let _ = bridge
                .cmd_tx
                .send(BridgeCommand::Input(data.to_vec()))
                .await;
        }
        Ok(())
    }

    async fn window_change_request(
        &mut self,
        channel: ChannelId,
        col_width: u32,
        row_height: u32,
        _pix_width: u32,
        _pix_height: u32,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(bridge) = self.bridges.get(&channel) {
            let _ = bridge
                .cmd_tx
                .send(BridgeCommand::Resize(col_width, row_height))
                .await;
        }
        session.channel_success(channel)?;
        Ok(())
    }

    async fn channel_close(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(bridge) = self.bridges.remove(&channel) {
            let _ = bridge.cmd_tx.send(BridgeCommand::Disconnect).await;
            bridge.output_task.abort();
        }
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if let Some(bridge) = self.bridges.remove(&channel) {
            let _ = bridge.cmd_tx.send(BridgeCommand::Disconnect).await;
            bridge.output_task.abort();
        }
        Ok(())
    }
}
