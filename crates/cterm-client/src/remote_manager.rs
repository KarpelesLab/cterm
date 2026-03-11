//! Remote connection manager
//!
//! Caches `DaemonConnection` instances by remote name so that multiple
//! tabs targeting the same remote share a single SSH tunnel.

use crate::connection::DaemonConnection;
#[cfg(not(unix))]
use crate::error::ClientError;
use crate::error::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Manages connections to remote ctermd instances.
///
/// Each remote (identified by name) gets at most one SSH tunnel.
/// Callers obtain a `DaemonConnection` through [`get_or_connect`],
/// which reuses an existing connection or establishes a new one.
#[derive(Clone)]
pub struct RemoteManager {
    connections: Arc<Mutex<HashMap<String, DaemonConnection>>>,
}

impl RemoteManager {
    pub fn new() -> Self {
        Self {
            connections: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Get an existing connection for `remote_name`, or connect via SSH to `host`.
    ///
    /// The `host` parameter is the SSH destination (e.g. `user@hostname`).
    #[cfg(unix)]
    pub async fn get_or_connect(&self, remote_name: &str, host: &str) -> Result<DaemonConnection> {
        let mut map = self.connections.lock().await;

        if let Some(conn) = map.get(remote_name) {
            // TODO: health check / reconnect if the tunnel died
            return Ok(conn.clone());
        }

        log::info!("Connecting to remote '{}' ({})", remote_name, host);
        let conn = DaemonConnection::connect_ssh(host).await?;
        map.insert(remote_name.to_string(), conn.clone());
        Ok(conn)
    }

    /// Get an existing connection for `remote_name`, or connect via SSH to `host`.
    #[cfg(not(unix))]
    pub async fn get_or_connect(&self, remote_name: &str, _host: &str) -> Result<DaemonConnection> {
        Err(ClientError::Connection(format!(
            "Remote connections are not supported on this platform (remote: {})",
            remote_name
        )))
    }

    /// Remove a cached connection (e.g. on disconnect).
    pub async fn disconnect(&self, remote_name: &str) {
        self.connections.lock().await.remove(remote_name);
    }
}

impl Default for RemoteManager {
    fn default() -> Self {
        Self::new()
    }
}
