//! SSH server using russh.

use crate::latch::auth::AuthorizedKeys;
use crate::latch::handler::SshSessionHandler;
use crate::session::SessionManager;
use cterm_app::config::LatchConfig;
use parking_lot::RwLock;
use russh::keys::PrivateKey;
use russh::server::Server as _;
use std::sync::Arc;

/// The russh Server implementation. Creates a new handler for each connection.
#[derive(Clone)]
pub struct LatchSshServer {
    pub(crate) session_manager: Arc<SessionManager>,
    pub(crate) authorized_keys: Arc<RwLock<AuthorizedKeys>>,
    pub(crate) config: LatchConfig,
}

impl russh::server::Server for LatchSshServer {
    type Handler = SshSessionHandler;

    fn new_client(&mut self, peer_addr: Option<std::net::SocketAddr>) -> Self::Handler {
        log::info!("SSH connection from {:?}", peer_addr);
        SshSessionHandler::new(
            self.session_manager.clone(),
            self.authorized_keys.clone(),
            self.config.clone(),
            peer_addr,
        )
    }
}

/// Start the SSH server as a background tokio task.
pub async fn start_ssh_server(
    config: &LatchConfig,
    host_key: PrivateKey,
    authorized_keys: Arc<RwLock<AuthorizedKeys>>,
    session_manager: Arc<SessionManager>,
) -> anyhow::Result<tokio::task::JoinHandle<()>> {
    let russh_config = russh::server::Config {
        keys: vec![host_key],
        auth_rejection_time: std::time::Duration::from_secs(1),
        auth_rejection_time_initial: Some(std::time::Duration::from_secs(0)),
        methods: russh::MethodSet::from(&[russh::MethodKind::PublicKey][..]),
        ..Default::default()
    };
    let russh_config = Arc::new(russh_config);

    let addr: std::net::SocketAddr = config.ssh_listen.parse().map_err(|e| {
        anyhow::anyhow!("Invalid SSH listen address '{}': {}", config.ssh_listen, e)
    })?;

    let mut server = LatchSshServer {
        session_manager,
        authorized_keys,
        config: config.clone(),
    };

    log::info!("Latch SSH server listening on {}", addr);

    let handle = tokio::spawn(async move {
        if let Err(e) = server.run_on_address(russh_config, addr).await {
            log::error!("Latch SSH server error: {}", e);
        }
    });

    Ok(handle)
}
