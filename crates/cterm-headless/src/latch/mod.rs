//! Latch server: SSH, mosh, and web terminal access for ctermd sessions.
//!
//! When enabled via configuration, ctermd exposes:
//! - An SSH server for remote terminal access
//! - A mosh server for mobile/unreliable network connections
//! - A web terminal (HTTPS + WebSocket + xterm.js)
//! - A relay client for NAT traversal via relay.unixshells.com

pub mod auth;
pub mod bridge;
pub mod handler;
pub mod host_key;
pub mod relay;
pub mod renderer;
pub mod ssh_server;
pub mod web_server;

use crate::session::SessionManager;
use cterm_app::config::LatchConfig;
use std::sync::Arc;

/// Start all enabled latch services (SSH, mosh, web, relay).
///
/// Returns a handle that keeps the services alive. Drop it to shut down.
pub async fn start_latch(
    config: LatchConfig,
    session_manager: Arc<SessionManager>,
) -> anyhow::Result<LatchHandle> {
    let mut handles = Vec::new();

    if !config.enabled {
        return Ok(LatchHandle { _handles: handles });
    }

    // Load or generate host key
    let host_key = host_key::load_or_generate_host_key(&config)?;

    // Load authorized keys
    let authorized_keys = Arc::new(parking_lot::RwLock::new(auth::AuthorizedKeys::load(
        &config,
    )?));

    // Start SSH server
    let ssh_handle = ssh_server::start_ssh_server(
        &config,
        host_key,
        authorized_keys.clone(),
        session_manager.clone(),
    )
    .await?;
    handles.push(ssh_handle);

    log::info!("Latch SSH server started on {}", config.ssh_listen);

    // Start web terminal server if enabled
    if config.web_enabled {
        let web_handle =
            web_server::start_web_server(&config, authorized_keys.clone(), session_manager.clone())
                .await?;
        handles.push(web_handle);
        log::info!(
            "Latch web terminal started on https://{}",
            config.web_listen
        );
    }

    // Start relay client if enabled
    if config.relay_enabled {
        let relay_handle =
            relay::start_relay_client(&config, authorized_keys.clone(), session_manager.clone())
                .await?;
        handles.push(relay_handle);
        log::info!("Latch relay client connecting to {}", config.relay_host);
    }

    Ok(LatchHandle { _handles: handles })
}

/// Handle that keeps latch services alive. Services shut down when dropped.
pub struct LatchHandle {
    _handles: Vec<tokio::task::JoinHandle<()>>,
}
