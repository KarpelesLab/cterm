//! Daemon-backed terminal session
//!
//! Provides `DaemonTab` which wraps a daemon session with a local Terminal
//! for rendering. The Terminal has no PTY — input is forwarded to the daemon
//! and raw output is streamed back and fed through the local parser/screen.

use crate::session::next_tab_id;
use cterm_client::{ClientError, CreateSessionOpts, DaemonConnection, SessionHandle};
use cterm_core::pty::PtyError;
use cterm_core::screen::ScreenConfig;
use cterm_core::term::{Terminal, WriteFn};

/// A tab backed by a daemon session
pub struct DaemonTab {
    /// Unique tab ID (same ID space as local tabs)
    pub id: u64,
    /// Local terminal for screen state and rendering (no PTY)
    pub terminal: Terminal,
    /// Handle to the daemon session
    pub session: SessionHandle,
    /// Tab title
    pub title: String,
    /// Custom title set by user
    pub custom_title: Option<String>,
    /// Tab color override
    pub color: Option<String>,
    /// Whether there's unread output
    pub has_unread: bool,
    /// Whether the remote process is still running
    pub running: bool,
}

impl DaemonTab {
    /// Create a new daemon-backed tab by creating a session on the daemon
    pub async fn new(
        conn: &DaemonConnection,
        cols: usize,
        rows: usize,
    ) -> Result<Self, DaemonTabError> {
        let session = conn
            .create_session(CreateSessionOpts {
                cols: cols as u32,
                rows: rows as u32,
                ..Default::default()
            })
            .await?;

        let mut terminal = Terminal::new(cols, rows, ScreenConfig::default());

        // Set up write callback to forward input to daemon
        let write_session = session.clone();
        let write_fn: WriteFn = Box::new(move |data: &[u8]| {
            let session = write_session.clone();
            let data = data.to_vec();
            // Fire-and-forget async write from sync context
            tokio::spawn(async move {
                if let Err(e) = session.write_input(&data).await {
                    log::error!("Failed to write to daemon session: {}", e);
                }
            });
            Ok(())
        });
        terminal.set_write_fn(write_fn);

        Ok(Self {
            id: next_tab_id(),
            terminal,
            session,
            title: "Terminal".into(),
            custom_title: None,
            color: None,
            has_unread: false,
            running: true,
        })
    }

    /// Create a new daemon-backed tab with a specific command
    pub async fn with_command(
        conn: &DaemonConnection,
        command: &str,
        args: &[String],
        cwd: Option<String>,
        cols: usize,
        rows: usize,
    ) -> Result<Self, DaemonTabError> {
        let session = conn
            .create_session(CreateSessionOpts {
                cols: cols as u32,
                rows: rows as u32,
                shell: Some(command.to_string()),
                args: args.to_vec(),
                cwd,
                ..Default::default()
            })
            .await?;

        let mut terminal = Terminal::new(cols, rows, ScreenConfig::default());

        // Set up write callback
        let write_session = session.clone();
        let write_fn: WriteFn = Box::new(move |data: &[u8]| {
            let session = write_session.clone();
            let data = data.to_vec();
            tokio::spawn(async move {
                if let Err(e) = session.write_input(&data).await {
                    log::error!("Failed to write to daemon session: {}", e);
                }
            });
            Ok(())
        });
        terminal.set_write_fn(write_fn);

        Ok(Self {
            id: next_tab_id(),
            terminal,
            session,
            title: command.to_string(),
            custom_title: None,
            color: None,
            has_unread: false,
            running: true,
        })
    }

    /// Attach to an existing daemon session
    pub async fn attach(
        conn: &DaemonConnection,
        session_id: &str,
        cols: usize,
        rows: usize,
    ) -> Result<Self, DaemonTabError> {
        let (session, initial_screen) = conn
            .attach_session(session_id, cols as u32, rows as u32)
            .await?;

        let mut terminal = Terminal::new(cols, rows, ScreenConfig::default());

        // If we got an initial screen snapshot, replay it
        if let Some(screen_data) = initial_screen {
            // Feed the screen data into the terminal
            // The screen data from attach contains the full screen state as proto
            // We'll handle this by applying the proto screen data directly
            apply_screen_snapshot(&mut terminal, &screen_data);
        }

        // Set up write callback
        let write_session = session.clone();
        let write_fn: WriteFn = Box::new(move |data: &[u8]| {
            let session = write_session.clone();
            let data = data.to_vec();
            tokio::spawn(async move {
                if let Err(e) = session.write_input(&data).await {
                    log::error!("Failed to write to daemon session: {}", e);
                }
            });
            Ok(())
        });
        terminal.set_write_fn(write_fn);

        Ok(Self {
            id: next_tab_id(),
            terminal,
            session,
            title: "Terminal".into(),
            custom_title: None,
            color: None,
            has_unread: false,
            running: true,
        })
    }

    /// Get the display title
    pub fn display_title(&self) -> &str {
        self.custom_title.as_ref().unwrap_or(&self.title)
    }

    /// Whether this is a remote session
    pub fn is_remote(&self) -> bool {
        self.session.is_remote()
    }

    /// Get the hostname of the daemon
    pub fn hostname(&self) -> &str {
        self.session.hostname()
    }

    /// Get the daemon session ID
    pub fn session_id(&self) -> &str {
        self.session.session_id()
    }

    /// Detach from this session (keep running in background)
    pub async fn detach(&self) -> Result<(), DaemonTabError> {
        self.session.detach().await?;
        Ok(())
    }
}

/// Apply a proto screen snapshot to a local terminal.
///
/// Restores full screen content including visible rows, scrollback,
/// cursor position, title, and terminal modes from the proto snapshot.
pub fn apply_screen_snapshot(
    terminal: &mut Terminal,
    screen_data: &cterm_proto::proto::GetScreenResponse,
) {
    use cterm_proto::convert::color::proto_to_color;
    use cterm_proto::convert::screen::proto_to_attrs;

    let screen = terminal.screen_mut();

    // Resize if needed
    if screen_data.cols > 0 && screen_data.rows > 0 {
        screen.resize(screen_data.cols as usize, screen_data.rows as usize);
    }

    // Restore visible rows
    for (row_idx, row) in screen_data.visible_rows.iter().enumerate() {
        for (col_idx, cell) in row.cells.iter().enumerate() {
            if let Some(grid_cell) = screen.grid_mut().get_mut(row_idx, col_idx) {
                grid_cell.c = cell.char.chars().next().unwrap_or(' ');
                if let Some(fg) = &cell.fg {
                    grid_cell.fg = proto_to_color(fg);
                }
                if let Some(bg) = &cell.bg {
                    grid_cell.bg = proto_to_color(bg);
                }
                if let Some(attrs) = &cell.attrs {
                    grid_cell.attrs = proto_to_attrs(attrs);
                }
            }
        }
    }

    // Restore scrollback
    if !screen_data.scrollback.is_empty() {
        use cterm_core::grid::Row;
        for proto_row in &screen_data.scrollback {
            let mut row = Row::new(screen_data.cols as usize);
            for (col_idx, cell) in proto_row.cells.iter().enumerate() {
                if let Some(grid_cell) = row.get_mut(col_idx) {
                    grid_cell.c = cell.char.chars().next().unwrap_or(' ');
                    if let Some(fg) = &cell.fg {
                        grid_cell.fg = proto_to_color(fg);
                    }
                    if let Some(bg) = &cell.bg {
                        grid_cell.bg = proto_to_color(bg);
                    }
                    if let Some(attrs) = &cell.attrs {
                        grid_cell.attrs = proto_to_attrs(attrs);
                    }
                }
            }
            screen.scrollback_mut().push_back(row);
        }
    }

    // Restore cursor
    if let Some(cursor) = &screen_data.cursor {
        screen.cursor.row = cursor.row as usize;
        screen.cursor.col = cursor.col as usize;
        screen.modes.show_cursor = cursor.visible;
    }

    // Restore title
    if !screen_data.title.is_empty() {
        screen.title = screen_data.title.clone();
    }

    // Restore terminal modes
    if let Some(modes) = &screen_data.modes {
        screen.modes.application_cursor = modes.application_cursor;
        screen.modes.application_keypad = modes.application_keypad;
        screen.modes.bracketed_paste = modes.bracketed_paste;
        screen.modes.focus_events = modes.focus_events;
    }
}

/// Error type for daemon tab operations
#[derive(Debug, thiserror::Error)]
pub enum DaemonTabError {
    #[error("Client error: {0}")]
    Client(#[from] ClientError),

    #[error("PTY error: {0}")]
    Pty(#[from] PtyError),
}
