//! SessionHandle - wraps a session ID with convenient methods

use crate::connection::DaemonInfo;
use crate::error::Result;
use cterm_proto::proto::terminal_service_client::TerminalServiceClient;
use cterm_proto::proto::*;
use std::sync::Arc;
use tokio::sync::Mutex;
use tonic::transport::Channel;

/// Handle to a terminal session on ctermd
#[derive(Clone)]
pub struct SessionHandle {
    session_id: String,
    client: Arc<Mutex<TerminalServiceClient<Channel>>>,
    daemon_info: Arc<DaemonInfo>,
}

impl SessionHandle {
    pub(crate) fn new(
        session_id: String,
        client: Arc<Mutex<TerminalServiceClient<Channel>>>,
        daemon_info: Arc<DaemonInfo>,
    ) -> Self {
        Self {
            session_id,
            client,
            daemon_info,
        }
    }

    /// Get the session ID
    pub fn session_id(&self) -> &str {
        &self.session_id
    }

    /// Whether this session is on a remote daemon
    pub fn is_remote(&self) -> bool {
        !self.daemon_info.is_local
    }

    /// Get the hostname of the daemon running this session
    pub fn hostname(&self) -> &str {
        &self.daemon_info.hostname
    }

    /// Write raw input bytes to the PTY
    pub async fn write_input(&self, data: &[u8]) -> Result<u32> {
        let response = self
            .client
            .lock()
            .await
            .write_input(WriteInputRequest {
                session_id: self.session_id.clone(),
                data: data.to_vec(),
            })
            .await?;

        Ok(response.into_inner().bytes_written)
    }

    /// Send a key event
    pub async fn send_key(&self, key: Key, modifiers: Modifiers) -> Result<Vec<u8>> {
        let response = self
            .client
            .lock()
            .await
            .send_key(SendKeyRequest {
                session_id: self.session_id.clone(),
                key: Some(key),
                modifiers: Some(modifiers),
            })
            .await?;

        Ok(response.into_inner().sequence)
    }

    /// Get the full screen state
    pub async fn get_screen(&self, include_scrollback: bool) -> Result<GetScreenResponse> {
        let response = self
            .client
            .lock()
            .await
            .get_screen(GetScreenRequest {
                session_id: self.session_id.clone(),
                include_scrollback,
            })
            .await?;

        Ok(response.into_inner())
    }

    /// Get screen text as lines
    pub async fn get_screen_text(&self, include_scrollback: bool) -> Result<Vec<String>> {
        let response = self
            .client
            .lock()
            .await
            .get_screen_text(GetScreenTextRequest {
                session_id: self.session_id.clone(),
                include_scrollback,
                start_row: None,
                end_row: None,
            })
            .await?;

        Ok(response.into_inner().lines)
    }

    /// Get cursor position
    pub async fn get_cursor(&self) -> Result<CursorPosition> {
        let response = self
            .client
            .lock()
            .await
            .get_cursor(GetCursorRequest {
                session_id: self.session_id.clone(),
            })
            .await?;

        response
            .into_inner()
            .cursor
            .ok_or_else(|| crate::error::ClientError::SessionNotFound(self.session_id.clone()))
    }

    /// Resize the terminal
    pub async fn resize(&self, cols: u32, rows: u32) -> Result<()> {
        self.client
            .lock()
            .await
            .resize(ResizeRequest {
                session_id: self.session_id.clone(),
                cols,
                rows,
            })
            .await?;

        Ok(())
    }

    /// Send a signal to the child process
    pub async fn send_signal(&self, signal: i32) -> Result<()> {
        self.client
            .lock()
            .await
            .send_signal(SendSignalRequest {
                session_id: self.session_id.clone(),
                signal,
            })
            .await?;

        Ok(())
    }

    /// Subscribe to raw PTY output
    pub async fn stream_output(&self) -> Result<tonic::Streaming<OutputChunk>> {
        let response = self
            .client
            .lock()
            .await
            .stream_output(StreamOutputRequest {
                session_id: self.session_id.clone(),
            })
            .await?;

        Ok(response.into_inner())
    }

    /// Subscribe to terminal events (title changes, bell, process exit, etc.)
    pub async fn stream_events(&self) -> Result<tonic::Streaming<TerminalEvent>> {
        let response = self
            .client
            .lock()
            .await
            .stream_events(StreamEventsRequest {
                session_id: self.session_id.clone(),
            })
            .await?;

        Ok(response.into_inner())
    }

    /// Subscribe to screen updates (for remote rendering)
    pub async fn stream_screen_updates(&self) -> Result<tonic::Streaming<ScreenUpdate>> {
        let response = self
            .client
            .lock()
            .await
            .stream_screen_updates(StreamScreenUpdatesRequest {
                session_id: self.session_id.clone(),
            })
            .await?;

        Ok(response.into_inner())
    }

    /// Detach from this session (keep it running in background)
    pub async fn detach(&self) -> Result<()> {
        self.client
            .lock()
            .await
            .detach_session(DetachSessionRequest {
                session_id: self.session_id.clone(),
                keep_running: true,
            })
            .await?;

        Ok(())
    }

    /// Destroy this session
    pub async fn destroy(&self) -> Result<()> {
        self.client
            .lock()
            .await
            .destroy_session(DestroySessionRequest {
                session_id: self.session_id.clone(),
                signal: None,
            })
            .await?;

        Ok(())
    }

    /// Set a custom title for this session (persists across reconnects)
    pub async fn set_custom_title(&self, title: &str) -> Result<()> {
        self.client
            .lock()
            .await
            .set_session_title(SetSessionTitleRequest {
                session_id: self.session_id.clone(),
                custom_title: title.to_string(),
            })
            .await?;
        Ok(())
    }

    /// Get session info
    pub async fn info(&self) -> Result<SessionInfo> {
        let response = self
            .client
            .lock()
            .await
            .get_session(GetSessionRequest {
                session_id: self.session_id.clone(),
            })
            .await?;

        response
            .into_inner()
            .session
            .ok_or_else(|| crate::error::ClientError::SessionNotFound(self.session_id.clone()))
    }
}
