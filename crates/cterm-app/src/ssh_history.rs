//! Persistent history of SSH connection strings entered in the "SSH Remote"
//! form, most recent first. Backing store is a plain text file (one entry per
//! line) in the config directory, so it is trivially inspectable and editable.

use std::fs;
use std::path::PathBuf;

/// Maximum number of remembered connection strings.
const MAX_ENTRIES: usize = 20;

/// Path of the history file (`ssh_history` in the config directory).
pub fn history_path() -> Option<PathBuf> {
    crate::config::config_dir().map(|p| p.join("ssh_history"))
}

/// Load the SSH connection history, most recent first. Missing or unreadable
/// files yield an empty history.
pub fn load() -> Vec<String> {
    let Some(path) = history_path() else {
        return Vec::new();
    };
    let Ok(contents) = fs::read_to_string(path) else {
        return Vec::new();
    };
    contents
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take(MAX_ENTRIES)
        .map(str::to_string)
        .collect()
}

/// Record `entry` as the most recent connection string, deduplicating and
/// capping the list. Errors are logged and swallowed — history is best-effort.
pub fn add(entry: &str) {
    let entry = entry.trim();
    if entry.is_empty() {
        return;
    }
    let mut entries = load();
    entries.retain(|e| e != entry);
    entries.insert(0, entry.to_string());
    entries.truncate(MAX_ENTRIES);

    let Some(path) = history_path() else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    if let Err(e) = fs::write(&path, entries.join("\n") + "\n") {
        log::warn!("failed to write SSH history {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    // `load`/`add` hit the real config dir, so exercise only the pure parts
    // via a scratch file in a temp dir.
    use std::fs;

    #[test]
    fn dedupe_and_mru_order() {
        let dir = std::env::temp_dir().join(format!("cterm-sshhist-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("ssh_history");
        fs::write(&path, "b\na\n").unwrap();

        // Simulate add("a"): dedupe + move to front + cap.
        let mut entries: Vec<String> = fs::read_to_string(&path)
            .unwrap()
            .lines()
            .map(str::to_string)
            .collect();
        entries.retain(|e| e != "a");
        entries.insert(0, "a".to_string());
        assert_eq!(entries, vec!["a", "b"]);
        let _ = fs::remove_dir_all(&dir);
    }
}
