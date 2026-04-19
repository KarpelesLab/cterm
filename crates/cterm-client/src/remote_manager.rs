//! Remote connection manager
//!
//! Caches `DaemonConnection` instances by remote name so that multiple
//! tabs targeting the same remote share a single SSH tunnel.

use crate::connection::DaemonConnection;
#[cfg(unix)]
use crate::connection::SshTunnelHandle;
#[cfg(not(unix))]
use crate::error::ClientError;
use crate::error::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// One cached remote: the gRPC connection plus (on unix) a handle to the
/// SSH tunnel process so `disconnect()` can actually tear it down.
#[derive(Clone)]
struct RemoteEntry {
    conn: DaemonConnection,
    #[cfg(unix)]
    tunnel: SshTunnelHandle,
}

/// Manages connections to remote ctermd instances.
///
/// Each remote (identified by name) gets at most one SSH tunnel.
/// Callers obtain a `DaemonConnection` through [`get_or_connect`],
/// which reuses an existing connection or establishes a new one.
#[derive(Clone)]
pub struct RemoteManager {
    connections: Arc<Mutex<HashMap<String, RemoteEntry>>>,
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
    /// When `compress` is true, SSH compression (`-C`) is enabled on the tunnel.
    #[cfg(unix)]
    pub async fn get_or_connect(
        &self,
        remote_name: &str,
        host: &str,
        compress: bool,
    ) -> Result<DaemonConnection> {
        let mut map = self.connections.lock().await;

        if let Some(entry) = map.get(remote_name) {
            // TODO: health check / reconnect if the tunnel died
            return Ok(entry.conn.clone());
        }

        log::info!("Connecting to remote '{}' ({})", remote_name, host);
        let (conn, tunnel) = DaemonConnection::connect_ssh(host, compress).await?;
        map.insert(
            remote_name.to_string(),
            RemoteEntry {
                conn: conn.clone(),
                tunnel,
            },
        );
        Ok(conn)
    }

    /// Get an existing connection for `remote_name`, or connect via SSH to `host`.
    #[cfg(not(unix))]
    pub async fn get_or_connect(
        &self,
        remote_name: &str,
        _host: &str,
        _compress: bool,
    ) -> Result<DaemonConnection> {
        Err(ClientError::Connection(format!(
            "Remote connections are not supported on this platform (remote: {})",
            remote_name
        )))
    }

    /// Disconnect from a remote: kill the SSH tunnel (which breaks every
    /// gRPC channel using it) and drop the cache entry. The remote ctermd's
    /// sessions are NOT killed — they survive on the server, ready to be
    /// reattached on a future `get_or_connect`.
    pub async fn disconnect(&self, remote_name: &str) {
        let entry = self.connections.lock().await.remove(remote_name);
        #[cfg(unix)]
        if let Some(entry) = entry {
            entry.tunnel.kill();
        }
        #[cfg(not(unix))]
        let _ = entry;
    }
}

impl Default for RemoteManager {
    fn default() -> Self {
        Self::new()
    }
}
