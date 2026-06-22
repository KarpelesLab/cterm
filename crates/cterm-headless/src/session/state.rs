//! Session state management

use crate::bridge::PtyReader;
use crate::error::Result;
use cterm_core::screen::ScreenConfig;
use cterm_core::term::TerminalEvent;
#[cfg(unix)]
use cterm_core::Pty;
use cterm_core::{PtyConfig, PtySize, Terminal};
use parking_lot::RwLock;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::broadcast;

/// Output chunk with timestamp
#[derive(Clone, Debug)]
pub struct OutputData {
    pub data: Vec<u8>,
    pub timestamp_ms: u64,
}

/// Reply to an interactive SSH prompt (host key / password / passphrase).
#[derive(Clone, Debug, Default)]
pub struct PromptReply {
    /// For host-key prompts: whether the key was accepted.
    pub accept: bool,
    /// For password/passphrase prompts: the entered secret (None = cancelled).
    pub secret: Option<String>,
}

/// Session state wrapping a Terminal instance
pub struct SessionState {
    /// The terminal instance
    terminal: RwLock<Terminal>,

    /// Session ID
    pub id: String,

    /// Broadcast sender for output data
    output_tx: broadcast::Sender<OutputData>,

    /// Broadcast sender for terminal events
    event_tx: broadcast::Sender<TerminalEvent>,

    /// Number of currently attached clients
    attached_clients: AtomicU32,

    /// User-set custom title (overrides OSC title for display)
    custom_title: RwLock<String>,

    /// Tab color override (CSS hex, e.g. "#ff0000")
    tab_color: RwLock<String>,

    /// Template name used to create this session
    template_name: RwLock<String>,

    /// Whether this session has an unacknowledged bell alert
    alerted: std::sync::atomic::AtomicBool,

    /// Human-readable session name (for latch named sessions)
    session_name: RwLock<Option<String>>,

    /// True while an SSH session is still establishing its connection (no PTY
    /// yet). Keeps the session from being reaped as "dead" during connect.
    connecting: std::sync::atomic::AtomicBool,

    /// Broadcast of interactive SSH prompts (host key / password / passphrase)
    /// raised during connect; consumed by `StreamEvents` and surfaced to the UI.
    prompt_tx: broadcast::Sender<crate::proto::SessionPromptEvent>,

    /// Pending prompts awaiting a `RespondPrompt`, keyed by prompt id.
    prompt_registry: parking_lot::Mutex<HashMap<String, std::sync::mpsc::Sender<PromptReply>>>,

    /// Monotonic counter for generating prompt ids.
    prompt_counter: AtomicU64,
}

impl SessionState {
    /// Create a new session with the given configuration
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: String,
        cols: usize,
        rows: usize,
        shell: Option<String>,
        args: Vec<String>,
        cwd: Option<std::path::PathBuf>,
        env: Vec<(String, String)>,
        term: Option<String>,
        scrollback_lines: usize,
    ) -> Result<Arc<Self>> {
        let pty_config = PtyConfig {
            size: PtySize {
                cols: cols as u16,
                rows: rows as u16,
                ..Default::default()
            },
            shell,
            args,
            cwd,
            env,
            term,
        };

        let screen_config = ScreenConfig { scrollback_lines };
        let terminal = Terminal::with_shell(cols, rows, screen_config, &pty_config)?;

        // Create broadcast channels
        let (output_tx, _) = broadcast::channel(1024);
        let (event_tx, _) = broadcast::channel(256);

        let state = Arc::new(Self {
            terminal: RwLock::new(terminal),
            id,
            output_tx,
            event_tx,
            attached_clients: AtomicU32::new(0),
            custom_title: RwLock::new(String::new()),
            tab_color: RwLock::new(String::new()),
            template_name: RwLock::new(String::new()),
            session_name: RwLock::new(None),
            alerted: std::sync::atomic::AtomicBool::new(false),
            connecting: std::sync::atomic::AtomicBool::new(false),
            prompt_tx: broadcast::channel(16).0,
            prompt_registry: parking_lot::Mutex::new(HashMap::new()),
            prompt_counter: AtomicU64::new(0),
        });

        Ok(state)
    }

    /// Create a placeholder session for a native SSH connection that is still
    /// being established. It has a screen but no PTY yet; [`Self::is_running`]
    /// reports it as alive (via the `connecting` flag) so it is not reaped while
    /// connecting. Call [`Self::spawn_ssh_connect`] to drive the connection.
    pub fn new_ssh_connecting(
        id: String,
        cols: usize,
        rows: usize,
        scrollback_lines: usize,
    ) -> Arc<Self> {
        let screen_config = ScreenConfig { scrollback_lines };
        let terminal = Terminal::new(cols, rows, screen_config);

        let (output_tx, _) = broadcast::channel(1024);
        let (event_tx, _) = broadcast::channel(256);
        let (prompt_tx, _) = broadcast::channel(16);

        Arc::new(Self {
            terminal: RwLock::new(terminal),
            id,
            output_tx,
            event_tx,
            attached_clients: AtomicU32::new(0),
            custom_title: RwLock::new(String::new()),
            tab_color: RwLock::new(String::new()),
            template_name: RwLock::new(String::new()),
            session_name: RwLock::new(None),
            alerted: std::sync::atomic::AtomicBool::new(false),
            connecting: std::sync::atomic::AtomicBool::new(true),
            prompt_tx,
            prompt_registry: parking_lot::Mutex::new(HashMap::new()),
            prompt_counter: AtomicU64::new(0),
        })
    }

    /// Drive the SSH connection on a background task. Interactive prompts (host
    /// key, password, passphrase) are surfaced via [`Self::subscribe_prompts`]
    /// and answered with [`Self::respond_prompt`]. On success the PTY is
    /// attached and the reader started; on failure a `ProcessExited` event is
    /// broadcast.
    pub fn spawn_ssh_connect(
        self: &Arc<Self>,
        mut ssh_config: cterm_core::SshConfig,
        cols: usize,
        rows: usize,
    ) {
        let size = PtySize {
            cols: cols as u16,
            rows: rows as u16,
            ..Default::default()
        };
        let state = Arc::clone(self);

        tokio::spawn(async move {
            // Bind interactive prompt callbacks to this session.
            ssh_config.host_key_prompt = Some(state.host_key_prompt_callback());
            ssh_config.password_prompt = Some(state.password_prompt_callback());
            ssh_config.passphrase_prompt = Some(state.passphrase_prompt_callback());

            let connect_state = Arc::clone(&state);
            let result = tokio::task::spawn_blocking(move || {
                let _ = &connect_state; // keep the session alive for callbacks
                cterm_core::Pty::connect_ssh(ssh_config, size)
            })
            .await;

            state.connecting.store(false, Ordering::Relaxed);

            match result {
                Ok(Ok(pty)) => {
                    state.terminal.write().set_pty(pty);
                    if let Err(e) = state.start_reader() {
                        log::error!("Failed to start SSH reader for {}: {}", state.id, e);
                    }
                }
                Ok(Err(e)) => {
                    log::warn!("SSH connect failed for {}: {}", state.id, e);
                    state.process_output(format!("\r\nSSH connection failed: {e}\r\n").as_bytes());
                    state.broadcast_event(TerminalEvent::ProcessExited(1));
                }
                Err(e) => {
                    log::error!("SSH connect task panicked for {}: {}", state.id, e);
                    state.broadcast_event(TerminalEvent::ProcessExited(1));
                }
            }
        });
    }

    /// Whether this session is still establishing its SSH connection.
    pub fn is_connecting(&self) -> bool {
        self.connecting.load(Ordering::Relaxed)
    }

    /// Subscribe to interactive SSH prompts for this session.
    pub fn subscribe_prompts(&self) -> broadcast::Receiver<crate::proto::SessionPromptEvent> {
        self.prompt_tx.subscribe()
    }

    /// Emit a prompt and return a receiver that resolves when the client
    /// replies via [`Self::respond_prompt`]. Runs on the (blocking) connect
    /// thread, which parks on the returned receiver.
    fn emit_prompt(
        &self,
        event: crate::proto::SessionPromptEvent,
    ) -> std::sync::mpsc::Receiver<PromptReply> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.prompt_registry
            .lock()
            .insert(event.prompt_id.clone(), tx);
        let _ = self.prompt_tx.send(event);
        rx
    }

    /// Deliver a reply to a pending prompt. Returns false if unknown/expired.
    pub fn respond_prompt(&self, prompt_id: &str, reply: PromptReply) -> bool {
        if let Some(tx) = self.prompt_registry.lock().remove(prompt_id) {
            tx.send(reply).is_ok()
        } else {
            false
        }
    }

    fn next_prompt_id(&self) -> String {
        format!(
            "{}-{}",
            self.id,
            self.prompt_counter.fetch_add(1, Ordering::Relaxed)
        )
    }

    fn host_key_prompt_callback(self: &Arc<Self>) -> cterm_core::HostKeyPrompt {
        let state = Arc::clone(self);
        Arc::new(move |req: cterm_core::HostKeyRequest| {
            let prompt_id = state.next_prompt_id();
            let kind = if req.changed {
                crate::proto::PromptKind::HostkeyChanged
            } else {
                crate::proto::PromptKind::HostkeyUnknown
            };
            let rx = state.emit_prompt(crate::proto::SessionPromptEvent {
                prompt_id,
                kind: kind as i32,
                host: req.host,
                port: req.port as u32,
                key_type: req.key_type,
                fingerprint: req.fingerprint,
                text: String::new(),
            });
            rx.recv().map(|r| r.accept).unwrap_or(false)
        })
    }

    fn password_prompt_callback(self: &Arc<Self>) -> cterm_core::PasswordPrompt {
        let state = Arc::clone(self);
        Arc::new(move |text: &str| {
            let prompt_id = state.next_prompt_id();
            let rx = state.emit_prompt(crate::proto::SessionPromptEvent {
                prompt_id,
                kind: crate::proto::PromptKind::Password as i32,
                host: String::new(),
                port: 0,
                key_type: String::new(),
                fingerprint: String::new(),
                text: text.to_string(),
            });
            rx.recv().ok().and_then(|r| r.secret)
        })
    }

    fn passphrase_prompt_callback(self: &Arc<Self>) -> cterm_core::PassphrasePrompt {
        let state = Arc::clone(self);
        Arc::new(move |path: &str| {
            let prompt_id = state.next_prompt_id();
            let rx = state.emit_prompt(crate::proto::SessionPromptEvent {
                prompt_id,
                kind: crate::proto::PromptKind::Passphrase as i32,
                host: String::new(),
                port: 0,
                key_type: String::new(),
                fingerprint: String::new(),
                text: format!("Enter passphrase for {path}"),
            });
            rx.recv().ok().and_then(|r| r.secret)
        })
    }

    /// Reconstruct a session from a raw PTY file descriptor (used during relaunch).
    ///
    /// # Safety
    /// The caller must ensure `fd` is a valid PTY master FD and `child_pid` is correct.
    #[cfg(unix)]
    #[allow(clippy::too_many_arguments)]
    pub unsafe fn from_raw_fd(
        id: String,
        fd: i32,
        child_pid: i32,
        cols: usize,
        rows: usize,
        custom_title: String,
        tab_color: String,
        template_name: String,
        scrollback_lines: usize,
    ) -> Result<Arc<Self>> {
        let pty = Pty::from_raw_fd(fd, child_pid);
        let screen_config = ScreenConfig { scrollback_lines };
        let mut terminal = Terminal::new(cols, rows, screen_config);
        terminal.set_pty(pty);

        let (output_tx, _) = broadcast::channel(1024);
        let (event_tx, _) = broadcast::channel(256);

        let state = Arc::new(Self {
            terminal: RwLock::new(terminal),
            id,
            output_tx,
            event_tx,
            attached_clients: AtomicU32::new(0),
            custom_title: RwLock::new(custom_title),
            tab_color: RwLock::new(tab_color),
            template_name: RwLock::new(template_name),
            session_name: RwLock::new(None),
            alerted: std::sync::atomic::AtomicBool::new(false),
            connecting: std::sync::atomic::AtomicBool::new(false),
            prompt_tx: broadcast::channel(16).0,
            prompt_registry: parking_lot::Mutex::new(HashMap::new()),
            prompt_counter: AtomicU64::new(0),
        });

        Ok(state)
    }

    /// Start the PTY reader task
    pub fn start_reader(self: &Arc<Self>) -> Result<Arc<Self>> {
        let pty_reader = self.terminal.read().pty_reader();

        if let Some(reader) = pty_reader {
            let state = Arc::clone(self);
            // Spawn the reader task - it will run until the PTY closes
            tokio::spawn(async move {
                let pty_reader = PtyReader::new(reader);
                pty_reader.run(Arc::clone(&state)).await;
                // Notify subscribers that the process has exited
                log::debug!(
                    "PTY closed for session {}, broadcasting ProcessExited",
                    state.id
                );
                state.broadcast_event(TerminalEvent::ProcessExited(0));
            });
        }

        Ok(Arc::clone(self))
    }

    /// Increment the attached client count
    pub fn attach(&self) {
        self.attached_clients.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement the attached client count
    pub fn detach(&self) {
        self.attached_clients.fetch_sub(1, Ordering::Relaxed);
    }

    /// Get the number of currently attached clients
    pub fn attached_clients(&self) -> u32 {
        self.attached_clients.load(Ordering::Relaxed)
    }

    /// Get the terminal dimensions
    pub fn dimensions(&self) -> (usize, usize) {
        let term = self.terminal.read();
        (term.cols(), term.rows())
    }

    /// Get the terminal title
    pub fn title(&self) -> String {
        self.terminal.read().title().to_string()
    }

    /// Get the user-set custom title
    pub fn custom_title(&self) -> String {
        self.custom_title.read().clone()
    }

    /// Set a custom title (empty string to clear)
    pub fn set_custom_title(&self, title: String) {
        *self.custom_title.write() = title;
    }

    /// Get the tab color override
    pub fn tab_color(&self) -> String {
        self.tab_color.read().clone()
    }

    /// Set the tab color override (empty string to clear)
    pub fn set_tab_color(&self, color: String) {
        *self.tab_color.write() = color;
    }

    /// Get the template name
    pub fn template_name(&self) -> String {
        self.template_name.read().clone()
    }

    /// Set the template name
    pub fn set_template_name(&self, name: String) {
        *self.template_name.write() = name;
    }

    /// Get the human-readable session name (for latch)
    pub fn session_name(&self) -> Option<String> {
        self.session_name.read().clone()
    }

    /// Set the human-readable session name
    pub fn set_session_name(&self, name: Option<String>) {
        *self.session_name.write() = name;
    }

    /// Whether this session has an unacknowledged bell alert.
    pub fn is_alerted(&self) -> bool {
        self.alerted.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Set the alerted state and broadcast a bell event if newly alerted.
    pub fn set_alerted(&self, alerted: bool) {
        let was_alerted = self
            .alerted
            .swap(alerted, std::sync::atomic::Ordering::Relaxed);
        if alerted && !was_alerted {
            self.broadcast_event(TerminalEvent::Bell);
        }
    }

    /// Check if the terminal is still running
    pub fn is_running(&self) -> bool {
        // A session still establishing its SSH connection has no PTY yet but
        // must not be treated as dead.
        self.connecting.load(Ordering::Relaxed) || self.terminal.write().is_running()
    }

    /// Get the child process ID
    pub fn child_pid(&self) -> Option<i32> {
        self.terminal.read().child_pid()
    }

    /// Check if a non-shell foreground process is running (PID-based).
    #[cfg(unix)]
    pub fn has_foreground_process(&self) -> bool {
        self.terminal.read().has_foreground_process()
    }

    /// Check if a non-shell foreground process is running (stub for non-Unix).
    #[cfg(not(unix))]
    pub fn has_foreground_process(&self) -> bool {
        false
    }

    /// Get the name of the foreground process (for display only).
    #[cfg(unix)]
    pub fn foreground_process_name(&self) -> Option<String> {
        self.terminal.read().foreground_process_name()
    }

    /// Get the name of the foreground process (stub for non-Unix).
    #[cfg(not(unix))]
    pub fn foreground_process_name(&self) -> Option<String> {
        None
    }

    /// Write input to the terminal
    pub fn write_input(&self, data: &[u8]) -> Result<usize> {
        let mut term = self.terminal.write();
        term.write(data)?;
        Ok(data.len())
    }

    /// Resize the terminal
    pub fn resize(&self, cols: usize, rows: usize) {
        self.terminal.write().resize(cols, rows);
    }

    /// Send a signal to the child process
    pub fn send_signal(&self, signal: i32) -> Result<()> {
        self.terminal.read().send_signal(signal)?;
        Ok(())
    }

    /// Process PTY output data
    pub fn process_output(&self, data: &[u8]) -> Vec<TerminalEvent> {
        self.terminal.write().process(data)
    }

    /// Broadcast output data to subscribers
    pub fn broadcast_output(&self, data: OutputData) {
        let _ = self.output_tx.send(data);
    }

    /// Broadcast a terminal event to subscribers
    pub fn broadcast_event(&self, event: TerminalEvent) {
        let _ = self.event_tx.send(event);
    }

    /// Subscribe to output stream
    pub fn subscribe_output(&self) -> broadcast::Receiver<OutputData> {
        self.output_tx.subscribe()
    }

    /// Subscribe to event stream
    pub fn subscribe_events(&self) -> broadcast::Receiver<TerminalEvent> {
        self.event_tx.subscribe()
    }

    /// Handle a key press and return the escape sequence
    pub fn handle_key(
        &self,
        key: cterm_core::term::Key,
        modifiers: cterm_core::term::Modifiers,
    ) -> Option<Vec<u8>> {
        self.terminal.read().handle_key(key, modifiers)
    }

    /// Get a reference to the terminal (for reading screen state)
    pub fn with_terminal<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&Terminal) -> R,
    {
        let term = self.terminal.read();
        f(&term)
    }

    /// Get a mutable reference to the terminal
    pub fn with_terminal_mut<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut Terminal) -> R,
    {
        let mut term = self.terminal.write();
        f(&mut term)
    }
}
