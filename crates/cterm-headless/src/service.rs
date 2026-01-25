//! gRPC TerminalService implementation

use crate::convert::{
    cell_to_proto, event_to_proto, proto_to_key, proto_to_modifiers, screen_to_proto,
    screen_to_text,
};
use crate::proto::terminal_service_server::TerminalService;
use crate::proto::*;
use crate::session::SessionManager;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use tokio_stream::{
    wrappers::errors::BroadcastStreamRecvError, wrappers::BroadcastStream, Stream, StreamExt,
};
use tonic::{Request, Response, Status};

/// TerminalService implementation
pub struct TerminalServiceImpl {
    session_manager: Arc<SessionManager>,
}

impl TerminalServiceImpl {
    /// Create a new TerminalService
    pub fn new(session_manager: Arc<SessionManager>) -> Self {
        Self { session_manager }
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

        let rx = session.subscribe_output();
        let stream = BroadcastStream::new(rx).filter_map(|result| {
            match result {
                Ok(data) => Some(Ok(OutputChunk {
                    data: data.data,
                    timestamp_ms: data.timestamp_ms,
                })),
                Err(BroadcastStreamRecvError::Lagged(_)) => {
                    // Skip lagged messages
                    None
                }
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
                visible: screen.cursor.visible,
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
        let stream = BroadcastStream::new(rx).filter_map(|result| match result {
            Ok(event) => Some(Ok(event_to_proto(&event))),
            Err(BroadcastStreamRecvError::Lagged(_)) => None,
        });

        Ok(Response::new(Box::pin(stream)))
    }
}
