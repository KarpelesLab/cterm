//! gRPC TerminalService implementation

use crate::convert::{
    cell_to_proto, event_to_proto, proto_to_key, proto_to_modifiers, screen_to_proto,
    screen_to_text,
};
use crate::proto::terminal_service_server::TerminalService;
use crate::proto::*;
use crate::session::SessionManager;
#[cfg(unix)]
use libc;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Notify;
use tokio_stream::{
    wrappers::errors::BroadcastStreamRecvError, wrappers::BroadcastStream, Stream, StreamExt,
};
use tonic::{Request, Response, Status};

/// TerminalService implementation
pub struct TerminalServiceImpl {
    session_manager: Arc<SessionManager>,
    /// Notifier used to trigger server shutdown from the shutdown RPC
    shutdown_notify: Arc<Notify>,
    /// Unique identifier for this daemon instance
    daemon_id: String,
    /// Time when the daemon was started
    start_time: Instant,
    /// Number of clients that have performed a handshake
    client_count: AtomicU32,
    /// Number of active output streams (proxy for connected clients)
    active_streams: Arc<AtomicU32>,
}

impl TerminalServiceImpl {
    /// Create a new TerminalService with a shutdown notifier
    pub fn new(session_manager: Arc<SessionManager>, shutdown_notify: Arc<Notify>) -> Self {
        Self {
            session_manager,
            shutdown_notify,
            daemon_id: uuid::Uuid::new_v4().to_string(),
            start_time: Instant::now(),
            client_count: AtomicU32::new(0),
            active_streams: Arc::new(AtomicU32::new(0)),
        }
    }
}

#[tonic::async_trait]
impl TerminalService for TerminalServiceImpl {
    // ========================================================================
    // Session Management
    // ========================================================================

    async fn create_session(
        &self,
        request: Request<CreateSessionRequest>,
    ) -> Result<Response<CreateSessionResponse>, Status> {
        let req = request.into_inner();

        let cols = req.cols.max(1) as usize;
        let rows = req.rows.max(1) as usize;

        let env: Vec<(String, String)> = req.env.into_iter().collect();

        let session = self
            .session_manager
            .create_session(
                cols,
                rows,
                req.shell,
                req.args,
                req.cwd.map(PathBuf::from),
                env,
                req.term,
            )
            .map_err(Status::from)?;

        Ok(Response::new(CreateSessionResponse {
            session_id: session.id.clone(),
            cols: cols as u32,
            rows: rows as u32,
        }))
    }

    async fn list_sessions(
        &self,
        _request: Request<ListSessionsRequest>,
    ) -> Result<Response<ListSessionsResponse>, Status> {
        let sessions = self.session_manager.list_sessions();

        let session_infos: Vec<SessionInfo> = sessions
            .iter()
            .map(|s| {
                let (cols, rows) = s.dimensions();
                SessionInfo {
                    session_id: s.id.clone(),
                    cols: cols as u32,
                    rows: rows as u32,
                    title: s.title(),
                    running: s.is_running(),
                    child_pid: s.child_pid().unwrap_or(0),
                    attached_clients: s.attached_clients(),
                    custom_title: s.custom_title(),
                }
            })
            .collect();

        Ok(Response::new(ListSessionsResponse {
            sessions: session_infos,
        }))
    }

    async fn get_session(
        &self,
        request: Request<GetSessionRequest>,
    ) -> Result<Response<GetSessionResponse>, Status> {
        let req = request.into_inner();
        let session = self
            .session_manager
            .get_session(&req.session_id)
            .map_err(Status::from)?;

        let (cols, rows) = session.dimensions();
        let info = SessionInfo {
            session_id: session.id.clone(),
            cols: cols as u32,
            rows: rows as u32,
            title: session.title(),
            running: session.is_running(),
            child_pid: session.child_pid().unwrap_or(0),
            attached_clients: session.attached_clients(),
            custom_title: session.custom_title(),
        };

        Ok(Response::new(GetSessionResponse {
            session: Some(info),
        }))
    }

    async fn destroy_session(
        &self,
        request: Request<DestroySessionRequest>,
    ) -> Result<Response<DestroySessionResponse>, Status> {
        let req = request.into_inner();
        self.session_manager
            .destroy_session(&req.session_id, req.signal)
            .map_err(Status::from)?;

        Ok(Response::new(DestroySessionResponse { success: true }))
    }

    // ========================================================================
    // Session Metadata
    // ========================================================================

    async fn set_session_title(
        &self,
        request: Request<SetSessionTitleRequest>,
    ) -> Result<Response<SetSessionTitleResponse>, Status> {
        let req = request.into_inner();
        let session = self
            .session_manager
            .get_session(&req.session_id)
            .map_err(Status::from)?;

        session.set_custom_title(req.custom_title);

        Ok(Response::new(SetSessionTitleResponse { success: true }))
    }

    // ========================================================================
    // Input
    // ========================================================================

    async fn write_input(
        &self,
        request: Request<WriteInputRequest>,
    ) -> Result<Response<WriteInputResponse>, Status> {
        let req = request.into_inner();
        let session = self
            .session_manager
            .get_session(&req.session_id)
            .map_err(Status::from)?;

        let bytes_written = session.write_input(&req.data).map_err(Status::from)?;

        Ok(Response::new(WriteInputResponse {
            bytes_written: bytes_written as u32,
        }))
    }

    async fn send_key(
        &self,
        request: Request<SendKeyRequest>,
    ) -> Result<Response<SendKeyResponse>, Status> {
        let req = request.into_inner();
        let session = self
            .session_manager
            .get_session(&req.session_id)
            .map_err(Status::from)?;

        let key = req
            .key
            .as_ref()
            .and_then(proto_to_key)
            .ok_or_else(|| Status::invalid_argument("Invalid key"))?;

        let modifiers = req
            .modifiers
            .as_ref()
            .map(proto_to_modifiers)
            .unwrap_or_default();

        let sequence = session.handle_key(key, modifiers).unwrap_or_default();

        // Write the sequence to the PTY
        if !sequence.is_empty() {
            session.write_input(&sequence).map_err(Status::from)?;
        }

        Ok(Response::new(SendKeyResponse { sequence }))
    }

    // ========================================================================
    // Output Streaming
    // ========================================================================

    type StreamOutputStream =
        Pin<Box<dyn Stream<Item = Result<OutputChunk, Status>> + Send + 'static>>;

    async fn stream_output(
        &self,
        request: Request<StreamOutputRequest>,
    ) -> Result<Response<Self::StreamOutputStream>, Status> {
        let req = request.into_inner();
        let session = self
            .session_manager
            .get_session(&req.session_id)
            .map_err(Status::from)?;

        session.attach();
        self.active_streams.fetch_add(1, Ordering::Relaxed);

        let rx = session.subscribe_output();
        let session_id = req.session_id.clone();
        let session_detach = session.clone();
        let active_streams = Arc::clone(&self.active_streams);
        let session_manager = Arc::clone(&self.session_manager);
        let shutdown_notify = Arc::clone(&self.shutdown_notify);
        let stream = BroadcastStream::new(rx).filter_map(move |result| match result {
            Ok(data) => Some(Ok(OutputChunk {
                data: data.data,
                timestamp_ms: data.timestamp_ms,
            })),
            Err(BroadcastStreamRecvError::Lagged(count)) => {
                log::warn!(
                    "stream_output: client lagged, dropped {} messages for session {}. \
                         Client terminal state may be stale until new output arrives.",
                    count,
                    session_id,
                );
                None
            }
        });

        // Wrap the stream to detach and check auto-shutdown when the client disconnects
        let stream = StreamNotify::new(stream, move || {
            session_detach.detach();
            let prev = active_streams.fetch_sub(1, Ordering::Relaxed);
            if prev == 1 && session_manager.session_count() == 0 && session_manager.had_sessions() {
                log::info!("No sessions and no connected clients, shutting down daemon");
                shutdown_notify.notify_one();
            }
        });

        Ok(Response::new(Box::pin(stream)))
    }

    // ========================================================================
    // Screen State
    // ========================================================================

    async fn get_screen(
        &self,
        request: Request<GetScreenRequest>,
    ) -> Result<Response<GetScreenResponse>, Status> {
        let req = request.into_inner();
        let session = self
            .session_manager
            .get_session(&req.session_id)
            .map_err(Status::from)?;

        let response =
            session.with_terminal(|term| screen_to_proto(term.screen(), req.include_scrollback));

        Ok(Response::new(response))
    }

    async fn get_cell(
        &self,
        request: Request<GetCellRequest>,
    ) -> Result<Response<GetCellResponse>, Status> {
        let req = request.into_inner();
        let session = self
            .session_manager
            .get_session(&req.session_id)
            .map_err(Status::from)?;

        let cell = session.with_terminal(|term| {
            term.screen()
                .get_cell(req.row as usize, req.col as usize)
                .cloned()
        });

        let cell = cell.ok_or_else(|| Status::out_of_range("Cell position out of range"))?;

        Ok(Response::new(GetCellResponse {
            cell: Some(cell_to_proto(&cell)),
        }))
    }

    async fn get_cursor(
        &self,
        request: Request<GetCursorRequest>,
    ) -> Result<Response<GetCursorResponse>, Status> {
        let req = request.into_inner();
        let session = self
            .session_manager
            .get_session(&req.session_id)
            .map_err(Status::from)?;

        let cursor = session.with_terminal(|term| {
            let screen = term.screen();
            CursorPosition {
                row: screen.cursor.row as u32,
                col: screen.cursor.col as u32,
                visible: screen.modes.show_cursor,
                style: CursorStyle::Block as i32,
            }
        });

        Ok(Response::new(GetCursorResponse {
            cursor: Some(cursor),
        }))
    }

    async fn get_screen_text(
        &self,
        request: Request<GetScreenTextRequest>,
    ) -> Result<Response<GetScreenTextResponse>, Status> {
        let req = request.into_inner();
        let session = self
            .session_manager
            .get_session(&req.session_id)
            .map_err(Status::from)?;

        let lines = session.with_terminal(|term| {
            screen_to_text(
                term.screen(),
                req.include_scrollback,
                req.start_row,
                req.end_row,
            )
        });

        Ok(Response::new(GetScreenTextResponse { lines }))
    }

    // ========================================================================
    // Control
    // ========================================================================

    async fn resize(
        &self,
        request: Request<ResizeRequest>,
    ) -> Result<Response<ResizeResponse>, Status> {
        let req = request.into_inner();
        let session = self
            .session_manager
            .get_session(&req.session_id)
            .map_err(Status::from)?;

        session.resize(req.cols as usize, req.rows as usize);

        Ok(Response::new(ResizeResponse { success: true }))
    }

    async fn send_signal(
        &self,
        request: Request<SendSignalRequest>,
    ) -> Result<Response<SendSignalResponse>, Status> {
        let req = request.into_inner();
        let session = self
            .session_manager
            .get_session(&req.session_id)
            .map_err(Status::from)?;

        session.send_signal(req.signal).map_err(Status::from)?;

        Ok(Response::new(SendSignalResponse { success: true }))
    }

    // ========================================================================
    // Event Streaming
    // ========================================================================

    type StreamEventsStream =
        Pin<Box<dyn Stream<Item = Result<TerminalEvent, Status>> + Send + 'static>>;

    async fn stream_events(
        &self,
        request: Request<StreamEventsRequest>,
    ) -> Result<Response<Self::StreamEventsStream>, Status> {
        let req = request.into_inner();
        let session = self
            .session_manager
            .get_session(&req.session_id)
            .map_err(Status::from)?;

        let rx = session.subscribe_events();
        let session_id = req.session_id.clone();
        let stream = BroadcastStream::new(rx).filter_map(move |result| match result {
            Ok(event) => Some(Ok(event_to_proto(&event))),
            Err(BroadcastStreamRecvError::Lagged(count)) => {
                log::warn!(
                    "stream_events: client lagged, dropped {} events for session {}",
                    count,
                    session_id,
                );
                None
            }
        });

        Ok(Response::new(Box::pin(stream)))
    }

    // ========================================================================
    // Connection Management (new RPCs)
    // ========================================================================

    async fn handshake(
        &self,
        request: Request<HandshakeRequest>,
    ) -> Result<Response<HandshakeResponse>, Status> {
        let req = request.into_inner();
        log::info!(
            "Client connected: {} (version {})",
            req.client_id,
            req.client_version
        );

        self.client_count.fetch_add(1, Ordering::Relaxed);

        let hostname = gethostname();

        Ok(Response::new(HandshakeResponse {
            daemon_id: self.daemon_id.clone(),
            daemon_version: env!("CARGO_PKG_VERSION").to_string(),
            is_local: true,
            hostname,
            protocol_version: 1,
        }))
    }

    async fn attach_session(
        &self,
        request: Request<AttachSessionRequest>,
    ) -> Result<Response<AttachSessionResponse>, Status> {
        let req = request.into_inner();
        let session = self
            .session_manager
            .get_session(&req.session_id)
            .map_err(Status::from)?;

        session.attach();

        // Resize to client dimensions if provided
        if req.cols > 0 && req.rows > 0 {
            session.resize(req.cols as usize, req.rows as usize);
        }

        let (cols, rows) = session.dimensions();
        let info = SessionInfo {
            session_id: session.id.clone(),
            cols: cols as u32,
            rows: rows as u32,
            title: session.title(),
            running: session.is_running(),
            child_pid: session.child_pid().unwrap_or(0),
            attached_clients: session.attached_clients(),
            custom_title: session.custom_title(),
        };

        let initial_screen = if req.want_screen_snapshot {
            Some(session.with_terminal(|term| screen_to_proto(term.screen(), true)))
        } else {
            None
        };

        Ok(Response::new(AttachSessionResponse {
            session: Some(info),
            initial_screen,
        }))
    }

    async fn detach_session(
        &self,
        request: Request<DetachSessionRequest>,
    ) -> Result<Response<DetachSessionResponse>, Status> {
        let req = request.into_inner();

        // Decrement attached count if session still exists
        if let Ok(session) = self.session_manager.get_session(&req.session_id) {
            session.detach();
        }

        if !req.keep_running {
            // Destroy the session
            self.session_manager
                .destroy_session(&req.session_id, None)
                .map_err(Status::from)?;
        }

        Ok(Response::new(DetachSessionResponse { success: true }))
    }

    // ========================================================================
    // Screen Update Streaming
    // ========================================================================

    type StreamScreenUpdatesStream =
        Pin<Box<dyn Stream<Item = Result<ScreenUpdate, Status>> + Send + 'static>>;

    async fn stream_screen_updates(
        &self,
        request: Request<StreamScreenUpdatesRequest>,
    ) -> Result<Response<Self::StreamScreenUpdatesStream>, Status> {
        let req = request.into_inner();
        let session = self
            .session_manager
            .get_session(&req.session_id)
            .map_err(Status::from)?;

        // For now, send a full screen update on each ContentChanged event
        let session_id = req.session_id.clone();
        let rx = session.subscribe_events();
        let session_ref = session.clone();
        let mut seq: u64 = 0;

        let stream = BroadcastStream::new(rx).filter_map(move |result| match result {
            Ok(event) => {
                if matches!(event, cterm_core::term::TerminalEvent::ContentChanged) {
                    seq += 1;
                    let screen_data =
                        session_ref.with_terminal(|term| screen_to_proto(term.screen(), false));
                    Some(Ok(ScreenUpdate {
                        session_id: session_id.clone(),
                        sequence: seq,
                        update_type: Some(screen_update::UpdateType::FullScreen(
                            FullScreenUpdate {
                                screen: Some(screen_data),
                            },
                        )),
                    }))
                } else if matches!(event, cterm_core::term::TerminalEvent::TitleChanged(_)) {
                    seq += 1;
                    let title = session_ref.title();
                    Some(Ok(ScreenUpdate {
                        session_id: session_id.clone(),
                        sequence: seq,
                        update_type: Some(screen_update::UpdateType::Title(TitleUpdate { title })),
                    }))
                } else {
                    None
                }
            }
            Err(BroadcastStreamRecvError::Lagged(count)) => {
                log::warn!(
                    "stream_screen_updates: client lagged, dropped {} events for session {}",
                    count,
                    session_id,
                );
                None
            }
        });

        Ok(Response::new(Box::pin(stream)))
    }

    // ========================================================================
    // Daemon Management
    // ========================================================================

    async fn get_daemon_info(
        &self,
        _request: Request<GetDaemonInfoRequest>,
    ) -> Result<Response<GetDaemonInfoResponse>, Status> {
        let hostname = gethostname();

        Ok(Response::new(GetDaemonInfoResponse {
            daemon_id: self.daemon_id.clone(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            hostname,
            session_count: self.session_manager.session_count() as u32,
            client_count: self.client_count.load(Ordering::Relaxed),
            uptime_secs: self.start_time.elapsed().as_secs(),
        }))
    }

    async fn shutdown(
        &self,
        request: Request<ShutdownRequest>,
    ) -> Result<Response<ShutdownResponse>, Status> {
        let req = request.into_inner();

        if !req.force && self.session_manager.session_count() > 0 {
            return Ok(Response::new(ShutdownResponse {
                success: false,
                reason: "Active sessions exist. Use force=true to override.".to_string(),
            }));
        }

        log::info!("Shutdown requested (force={})", req.force);

        // If force=true and sessions exist, destroy them all first
        if req.force {
            let sessions = self.session_manager.list_sessions();
            for session in &sessions {
                if let Err(e) = self.session_manager.destroy_session(&session.id, None) {
                    log::warn!(
                        "Failed to destroy session {} during shutdown: {}",
                        session.id,
                        e
                    );
                }
            }
        }

        // Trigger actual server shutdown
        self.shutdown_notify.notify_one();

        Ok(Response::new(ShutdownResponse {
            success: true,
            reason: String::new(),
        }))
    }
}

/// A stream wrapper that calls a callback when dropped (i.e. when the client disconnects).
struct StreamNotify<F: FnOnce()> {
    inner: Pin<Box<dyn Stream<Item = Result<OutputChunk, Status>> + Send>>,
    on_drop: Option<F>,
}

impl<F: FnOnce()> StreamNotify<F> {
    fn new<S>(inner: S, on_drop: F) -> Self
    where
        S: Stream<Item = Result<OutputChunk, Status>> + Send + 'static,
    {
        Self {
            inner: Box::pin(inner),
            on_drop: Some(on_drop),
        }
    }
}

impl<F: FnOnce()> Drop for StreamNotify<F> {
    fn drop(&mut self) {
        if let Some(f) = self.on_drop.take() {
            f();
        }
    }
}

// SAFETY: Both fields are Unpin — Pin<Box<...>> is always Unpin, and Option<F> is Unpin.
impl<F: FnOnce()> Unpin for StreamNotify<F> {}

impl<F: FnOnce()> Stream for StreamNotify<F> {
    type Item = Result<OutputChunk, Status>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Self::Item>> {
        self.inner.as_mut().poll_next(cx)
    }
}

fn gethostname() -> String {
    #[cfg(unix)]
    {
        let mut buf = [0u8; 256];
        if unsafe { libc::gethostname(buf.as_mut_ptr() as *mut _, buf.len()) } == 0 {
            let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
            String::from_utf8_lossy(&buf[..len]).to_string()
        } else {
            "unknown".to_string()
        }
    }
    #[cfg(not(unix))]
    {
        "unknown".to_string()
    }
}
