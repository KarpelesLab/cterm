//! Log capture for in-app log viewing
//!
//! Captures log messages in a ring buffer while forwarding to env_logger,
//! allowing users to view application logs without running from a terminal.

use std::collections::VecDeque;
use std::sync::Mutex;

use log::{Level, LevelFilter, Log, Metadata, Record};

/// Maximum number of log entries to keep
const MAX_LOG_ENTRIES: usize = 10000;

/// A captured log entry
#[derive(Clone)]
pub struct LogEntry {
    pub level: Level,
    pub target: String,
    pub message: String,
    pub timestamp: std::time::SystemTime,
}

impl LogEntry {
    /// Format the log entry for display
    pub fn format(&self) -> String {
        use std::time::UNIX_EPOCH;

        let duration = self
            .timestamp
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let secs = duration.as_secs() % 86400; // Time of day in seconds
        let hours = secs / 3600;
        let mins = (secs % 3600) / 60;
        let secs = secs % 60;
        let millis = duration.subsec_millis();

        let level_str = match self.level {
            Level::Error => "ERROR",
            Level::Warn => "WARN ",
            Level::Info => "INFO ",
            Level::Debug => "DEBUG",
            Level::Trace => "TRACE",
        };

        format!(
            "{:02}:{:02}:{:02}.{:03} {} [{}] {}",
            hours, mins, secs, millis, level_str, self.target, self.message
        )
    }
}

/// Ring buffer for log entries
struct LogBuffer {
    entries: VecDeque<LogEntry>,
}

impl LogBuffer {
    fn new() -> Self {
        Self {
            entries: VecDeque::with_capacity(MAX_LOG_ENTRIES),
        }
    }

    fn push(&mut self, entry: LogEntry) {
        if self.entries.len() >= MAX_LOG_ENTRIES {
            self.entries.pop_front();
        }
        self.entries.push_back(entry);
    }

    fn get_all(&self) -> Vec<LogEntry> {
        self.entries.iter().cloned().collect()
    }
}

/// Global log buffer
static LOG_BUFFER: Mutex<Option<LogBuffer>> = Mutex::new(None);

/// Logger that captures to ring buffer and forwards to env_logger
struct CapturingLogger {
    env_logger: env_logger::Logger,
}

impl Log for CapturingLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        self.env_logger.enabled(metadata)
    }

    fn log(&self, record: &Record) {
        // Capture to ring buffer
        if self.enabled(record.metadata()) {
            let entry = LogEntry {
                level: record.level(),
                target: record.target().to_string(),
                message: format!("{}", record.args()),
                timestamp: std::time::SystemTime::now(),
            };

            if let Ok(mut guard) = LOG_BUFFER.lock() {
                if let Some(ref mut buffer) = *guard {
                    buffer.push(entry);
                }
            }
        }

        // Forward to env_logger
        self.env_logger.log(record);
    }

    fn flush(&self) {
        self.env_logger.flush();
    }
}

/// Initialize the capturing logger
///
/// This should be called instead of `env_logger::init()`.
pub fn init() {
    // Initialize the buffer
    {
        let mut guard = LOG_BUFFER.lock().unwrap();
        *guard = Some(LogBuffer::new());
    }

    // Build env_logger
    let env_logger =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).build();

    let max_level = env_logger.filter();

    // Create capturing logger
    let logger = CapturingLogger { env_logger };

    // Set as global logger
    log::set_boxed_logger(Box::new(logger)).expect("Failed to set logger");
    log::set_max_level(max_level);
}

/// Get all captured log entries
pub fn get_logs() -> Vec<LogEntry> {
    if let Ok(guard) = LOG_BUFFER.lock() {
        if let Some(ref buffer) = *guard {
            return buffer.get_all();
        }
    }
    Vec::new()
}

/// Get logs formatted as a single string
pub fn get_logs_formatted() -> String {
    get_logs()
        .iter()
        .map(|e| e.format())
        .collect::<Vec<_>>()
        .join("\n")
}
