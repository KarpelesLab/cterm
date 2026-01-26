//! Git-backed configuration sync
//!
//! Provides git operations for syncing configuration across machines.

use std::path::Path;
use std::process::Command;

use thiserror::Error;

/// Git sync errors
#[derive(Error, Debug, Clone)]
pub enum GitError {
    #[error("Git command failed: {0}")]
    CommandFailed(String),

    #[error("Invalid path")]
    InvalidPath,

    #[error("IO error: {0}")]
    Io(String),

    #[error("Merge conflict: {0}")]
    MergeConflict(String),
}

impl From<std::io::Error> for GitError {
    fn from(e: std::io::Error) -> Self {
        GitError::Io(e.to_string())
    }
}

/// Helper function to convert io::Error to GitError
fn io_err(e: std::io::Error) -> GitError {
    GitError::Io(e.to_string())
}

/// Git sync status information
#[derive(Debug, Clone, Default)]
pub struct SyncStatus {
    /// Whether the config directory is a git repository
    pub is_repo: bool,
    /// Remote URL if configured
    pub remote_url: Option<String>,
    /// Current branch name
    pub branch: Option<String>,
    /// Last commit message
    pub last_commit: Option<String>,
    /// Last commit timestamp (Unix timestamp)
    pub last_commit_time: Option<i64>,
    /// Whether there are uncommitted local changes
    pub has_local_changes: bool,
    /// Whether there are unpushed commits
    pub has_unpushed: bool,
    /// Number of commits ahead of remote
    pub commits_ahead: u32,
    /// Number of commits behind remote
    pub commits_behind: u32,
}

/// Result of a pull operation
#[derive(Debug, Clone)]
pub enum PullResult {
    /// Already up to date
    UpToDate,
    /// Changes were pulled successfully
    Updated,
    /// Conflicts were resolved automatically
    ConflictsResolved(Vec<String>),
    /// No remote configured
    NoRemote,
    /// Not a git repository
    NotARepo,
}

/// Check if the config directory is a git repository
pub fn is_git_repo(config_dir: &Path) -> bool {
    config_dir.join(".git").exists()
}

/// Get the remote URL for origin (if any)
pub fn get_remote_url(config_dir: &Path) -> Option<String> {
    if !is_git_repo(config_dir) {
        return None;
    }

    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(config_dir)
        .output()
        .ok()?;

    if output.status.success() {
        let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if url.is_empty() {
            None
        } else {
            Some(url)
        }
    } else {
        None
    }
}

/// Get detailed sync status for the config directory
pub fn get_sync_status(config_dir: &Path) -> SyncStatus {
    let mut status = SyncStatus::default();

    if !is_git_repo(config_dir) {
        return status;
    }

    status.is_repo = true;
    status.remote_url = get_remote_url(config_dir);

    // Get current branch
    if let Ok(branch) = run_git(config_dir, &["rev-parse", "--abbrev-ref", "HEAD"]) {
        status.branch = Some(branch.trim().to_string());
    }

    // Get last commit message and time
    if let Ok(log) = run_git(config_dir, &["log", "-1", "--format=%s%n%ct"]) {
        let lines: Vec<&str> = log.trim().lines().collect();
        if lines.len() >= 2 {
            status.last_commit = Some(lines[0].to_string());
            if let Ok(ts) = lines[1].parse::<i64>() {
                status.last_commit_time = Some(ts);
            }
        }
    }

    // Check for local changes
    if let Ok(porcelain) = run_git(config_dir, &["status", "--porcelain"]) {
        status.has_local_changes = !porcelain.trim().is_empty();
    }

    // Check ahead/behind status (requires fetch first, so use cached info)
    if let Some(ref branch) = status.branch {
        let upstream = format!("origin/{}", branch);
        // Check how many commits ahead
        if let Ok(ahead) = run_git(
            config_dir,
            &["rev-list", "--count", &format!("{}..HEAD", upstream)],
        ) {
            status.commits_ahead = ahead.trim().parse().unwrap_or(0);
        }
        // Check how many commits behind
        if let Ok(behind) = run_git(
            config_dir,
            &["rev-list", "--count", &format!("HEAD..{}", upstream)],
        ) {
            status.commits_behind = behind.trim().parse().unwrap_or(0);
        }
        status.has_unpushed = status.commits_ahead > 0;
    }

    status
}

/// Result of initializing a git repo with a remote
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InitResult {
    /// Repository was initialized, no remote content
    Initialized,
    /// Remote content was pulled, config should be reloaded
    PulledRemote,
    /// Remote was updated, existing local content preserved
    UpdatedRemote,
}

/// Initialize git repo and set remote.
/// Returns `InitResult::PulledRemote` if remote content was pulled and config should be reloaded.
pub fn init_with_remote(config_dir: &Path, remote_url: &str) -> Result<InitResult, GitError> {
    let was_repo = is_git_repo(config_dir);
    let had_commits = was_repo && run_git(config_dir, &["rev-parse", "HEAD"]).is_ok();

    // git init (if not already a repo)
    if !was_repo {
        run_git(config_dir, &["init"])?;
        log::info!("Initialized git repository in config directory");
    }

    // Set or update remote
    if get_remote_url(config_dir).is_some() {
        run_git(config_dir, &["remote", "set-url", "origin", remote_url])?;
        log::info!("Updated git remote URL to: {}", remote_url);
    } else {
        run_git(config_dir, &["remote", "add", "origin", remote_url])?;
        log::info!("Added git remote origin: {}", remote_url);
    }

    // Try to fetch and set up tracking branch
    if run_git(config_dir, &["fetch", "origin"]).is_err() {
        // Fetch failed (maybe empty repo or network issue)
        return Ok(InitResult::Initialized);
    }

    // Check if remote has main or master branch
    let has_main = run_git(config_dir, &["rev-parse", "--verify", "origin/main"]).is_ok();
    let has_master = run_git(config_dir, &["rev-parse", "--verify", "origin/master"]).is_ok();

    if !has_main && !has_master {
        // Remote is empty
        return Ok(InitResult::Initialized);
    }

    let branch = if has_main { "main" } else { "master" };

    if !had_commits {
        // No local commits - checkout the remote branch (this loads remote config)
        let _ = run_git(
            config_dir,
            &["checkout", "-b", branch, &format!("origin/{}", branch)],
        );
        log::info!(
            "Checked out remote {} branch - config should be reloaded",
            branch
        );
        Ok(InitResult::PulledRemote)
    } else {
        // Local commits exist - set up tracking
        let _ = run_git(config_dir, &["branch", "-u", &format!("origin/{}", branch)]);
        Ok(InitResult::UpdatedRemote)
    }
}

/// Pull from remote (with rebase to avoid merge commits)
/// Returns true if changes were pulled
pub fn pull(config_dir: &Path) -> Result<bool, GitError> {
    match pull_with_conflict_resolution(config_dir)? {
        PullResult::Updated | PullResult::ConflictsResolved(_) => Ok(true),
        _ => Ok(false),
    }
}

/// Pull from remote with detailed result and conflict resolution
pub fn pull_with_conflict_resolution(config_dir: &Path) -> Result<PullResult, GitError> {
    if !is_git_repo(config_dir) {
        return Ok(PullResult::NotARepo);
    }

    // Check if we have a remote configured
    if get_remote_url(config_dir).is_none() {
        return Ok(PullResult::NoRemote);
    }

    // Stash any local changes first
    let had_changes = run_git(config_dir, &["status", "--porcelain"])
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);

    if had_changes {
        let _ = run_git(config_dir, &["stash", "push", "-m", "cterm-auto-stash"]);
    }

    // Fetch to see what's available
    run_git(config_dir, &["fetch", "origin"])?;

    // Determine which branch to pull (main or master)
    let branch = if run_git(config_dir, &["rev-parse", "--verify", "origin/main"]).is_ok() {
        "main"
    } else if run_git(config_dir, &["rev-parse", "--verify", "origin/master"]).is_ok() {
        "master"
    } else {
        log::warn!("No main or master branch found on remote");
        if had_changes {
            let _ = run_git(config_dir, &["stash", "pop"]);
        }
        return Ok(PullResult::NoRemote);
    };

    // Try to pull with rebase
    let pull_result = run_git(config_dir, &["pull", "--rebase", "origin", branch]);

    let result = match pull_result {
        Ok(output) => {
            let up_to_date = output.contains("Already up to date")
                || output.contains("up-to-date")
                || output.contains("Already up-to-date");

            if up_to_date {
                PullResult::UpToDate
            } else {
                PullResult::Updated
            }
        }
        Err(_) => {
            // Rebase failed - likely conflicts
            log::warn!("Pull with rebase failed, attempting conflict resolution");

            // Abort the rebase
            let _ = run_git(config_dir, &["rebase", "--abort"]);

            // Try a different approach: reset to remote and reapply local changes
            let resolved_files = resolve_conflicts_by_taking_remote(config_dir, branch)?;

            if resolved_files.is_empty() {
                PullResult::Updated
            } else {
                PullResult::ConflictsResolved(resolved_files)
            }
        }
    };

    // Restore stashed changes if we had any
    if had_changes {
        // Try to apply stash - if it conflicts, resolve by preferring stash (local) changes
        if run_git(config_dir, &["stash", "pop"]).is_err() {
            log::warn!("Stash pop had conflicts, resolving by keeping local changes");
            // Get list of conflicted files
            if let Ok(status) = run_git(config_dir, &["status", "--porcelain"]) {
                for line in status.lines() {
                    if line.starts_with("UU ") || line.starts_with("AA ") {
                        let file = line[3..].trim();
                        // For config files, keep the local (stashed) version
                        let _ = run_git(config_dir, &["checkout", "--theirs", file]);
                        let _ = run_git(config_dir, &["add", file]);
                    }
                }
            }
            // Drop the stash since we've handled it
            let _ = run_git(config_dir, &["stash", "drop"]);
        }
    }

    Ok(result)
}

/// Resolve conflicts by taking remote version for config files
/// This is a simple strategy that keeps remote config and discards conflicting local changes
fn resolve_conflicts_by_taking_remote(
    config_dir: &Path,
    branch: &str,
) -> Result<Vec<String>, GitError> {
    let mut resolved_files = Vec::new();

    // Hard reset to remote branch
    let remote_ref = format!("origin/{}", branch);
    run_git(config_dir, &["reset", "--hard", &remote_ref])?;

    log::info!("Reset to remote {} to resolve conflicts", remote_ref);
    resolved_files.push(format!("Reset to {}", remote_ref));

    Ok(resolved_files)
}

/// Commit and push all changes
pub fn commit_and_push(config_dir: &Path, message: &str) -> Result<(), GitError> {
    if !is_git_repo(config_dir) {
        return Err(GitError::CommandFailed("Not a git repository".to_string()));
    }

    // Ensure we have git user configured (use defaults if not)
    ensure_git_user(config_dir)?;

    // Stage all changes
    run_git(config_dir, &["add", "-A"])?;

    // Check if there are changes to commit
    let status = run_git(config_dir, &["status", "--porcelain"])?;
    if status.trim().is_empty() {
        log::debug!("No changes to commit");
        return Ok(()); // Nothing to commit
    }

    // Commit
    run_git(config_dir, &["commit", "-m", message])?;
    log::info!("Committed changes: {}", message);

    // Push (try main first, fall back to master)
    let push_result = run_git(config_dir, &["push", "-u", "origin", "main"]);
    if push_result.is_err() {
        run_git(config_dir, &["push", "-u", "origin", "master"])?;
    }

    log::info!("Pushed changes to remote");
    Ok(())
}

/// Ensure git user is configured (required for commits)
fn ensure_git_user(config_dir: &Path) -> Result<(), GitError> {
    // Check if user.name is set
    let name_set = run_git(config_dir, &["config", "user.name"]).is_ok();
    let email_set = run_git(config_dir, &["config", "user.email"]).is_ok();

    if !name_set {
        // Set a default name for this repo only
        run_git(config_dir, &["config", "user.name", "cterm"])?;
    }

    if !email_set {
        // Set a default email for this repo only
        run_git(config_dir, &["config", "user.email", "cterm@localhost"])?;
    }

    Ok(())
}

/// Clone a git repository to the specified directory
/// Creates parent directories if needed
pub fn clone_repo(remote_url: &str, target_dir: &Path) -> Result<(), GitError> {
    // Create parent directory if it doesn't exist
    if let Some(parent) = target_dir.parent() {
        if !parent.exists() {
            std::fs::create_dir_all(parent).map_err(io_err)?;
            log::info!("Created parent directory: {}", parent.display());
        }
    }

    // Clone the repository
    let output = Command::new("git")
        .args(["clone", remote_url])
        .arg(target_dir)
        .output()
        .map_err(io_err)?;

    if output.status.success() {
        log::info!(
            "Cloned repository {} to {}",
            remote_url,
            target_dir.display()
        );
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        log::error!("Git clone failed: {}", stderr);
        Err(GitError::CommandFailed(stderr))
    }
}

/// Prepare a template's working directory
/// If the directory doesn't exist but a git remote is configured, clone it
/// Returns Ok(true) if cloning was performed, Ok(false) if directory already existed
pub fn prepare_working_directory(
    working_dir: &Path,
    git_remote: Option<&str>,
) -> Result<bool, GitError> {
    if working_dir.exists() {
        return Ok(false);
    }

    match git_remote {
        Some(remote) if !remote.is_empty() => {
            log::info!(
                "Working directory {} does not exist, cloning from {}",
                working_dir.display(),
                remote
            );
            clone_repo(remote, working_dir)?;
            Ok(true)
        }
        _ => {
            // No git remote, just create the directory
            std::fs::create_dir_all(working_dir).map_err(io_err)?;
            log::info!("Created working directory: {}", working_dir.display());
            Ok(false)
        }
    }
}

/// Get the git remote URL for any directory (not just config dir)
/// Works on any directory that contains a .git subdirectory
pub fn get_directory_remote_url(dir: &Path) -> Option<String> {
    if !dir.join(".git").exists() {
        return None;
    }

    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(dir)
        .output()
        .ok()?;

    if output.status.success() {
        let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if url.is_empty() {
            None
        } else {
            Some(url)
        }
    } else {
        None
    }
}

/// Run a git command and return stdout
fn run_git(dir: &Path, args: &[&str]) -> Result<String, GitError> {
    log::trace!("Running git {:?} in {}", args, dir.display());

    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(io_err)?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        log::debug!("Git command failed: {:?} - {}", args, stderr);
        Err(GitError::CommandFailed(stderr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_is_git_repo_false() {
        let temp = TempDir::new().unwrap();
        assert!(!is_git_repo(temp.path()));
    }

    #[test]
    fn test_is_git_repo_true() {
        let temp = TempDir::new().unwrap();
        fs::create_dir(temp.path().join(".git")).unwrap();
        assert!(is_git_repo(temp.path()));
    }

    #[test]
    fn test_get_remote_url_no_repo() {
        let temp = TempDir::new().unwrap();
        assert!(get_remote_url(temp.path()).is_none());
    }

    #[test]
    fn test_get_remote_url_repo_no_remote() {
        let temp = TempDir::new().unwrap();
        // Initialize a real git repo
        run_git(temp.path(), &["init"]).unwrap();
        // No remote configured yet
        assert!(get_remote_url(temp.path()).is_none());
    }

    #[test]
    fn test_get_remote_url_with_remote() {
        let temp = TempDir::new().unwrap();
        run_git(temp.path(), &["init"]).unwrap();
        run_git(
            temp.path(),
            &["remote", "add", "origin", "https://example.com/repo.git"],
        )
        .unwrap();
        assert_eq!(
            get_remote_url(temp.path()),
            Some("https://example.com/repo.git".to_string())
        );
    }

    #[test]
    fn test_init_result_equality() {
        assert_eq!(InitResult::Initialized, InitResult::Initialized);
        assert_eq!(InitResult::PulledRemote, InitResult::PulledRemote);
        assert_eq!(InitResult::UpdatedRemote, InitResult::UpdatedRemote);
        assert_ne!(InitResult::Initialized, InitResult::PulledRemote);
    }

    #[test]
    fn test_init_with_remote_new_repo() {
        let temp = TempDir::new().unwrap();
        // Use a fake remote that won't be reachable - init should still work
        // but fetch will fail, returning Initialized
        let result = init_with_remote(temp.path(), "https://example.com/fake.git").unwrap();
        assert_eq!(result, InitResult::Initialized);
        // Verify repo was created
        assert!(is_git_repo(temp.path()));
        // Verify remote was set
        assert_eq!(
            get_remote_url(temp.path()),
            Some("https://example.com/fake.git".to_string())
        );
    }

    #[test]
    fn test_init_with_remote_updates_existing_remote() {
        let temp = TempDir::new().unwrap();
        run_git(temp.path(), &["init"]).unwrap();
        run_git(
            temp.path(),
            &[
                "remote",
                "add",
                "origin",
                "https://old.example.com/repo.git",
            ],
        )
        .unwrap();

        // Update remote
        let result = init_with_remote(temp.path(), "https://new.example.com/repo.git").unwrap();
        assert_eq!(result, InitResult::Initialized); // Fetch fails, so Initialized
        assert_eq!(
            get_remote_url(temp.path()),
            Some("https://new.example.com/repo.git".to_string())
        );
    }

    #[test]
    fn test_pull_not_a_repo() {
        let temp = TempDir::new().unwrap();
        let result = pull(temp.path()).unwrap();
        assert!(!result);
    }

    #[test]
    fn test_pull_no_remote() {
        let temp = TempDir::new().unwrap();
        run_git(temp.path(), &["init"]).unwrap();
        let result = pull(temp.path()).unwrap();
        assert!(!result);
    }

    #[test]
    fn test_commit_and_push_not_a_repo() {
        let temp = TempDir::new().unwrap();
        let result = commit_and_push(temp.path(), "Test commit");
        assert!(result.is_err());
        if let Err(GitError::CommandFailed(msg)) = result {
            assert!(msg.contains("Not a git repository"));
        } else {
            panic!("Expected CommandFailed error");
        }
    }

    #[test]
    fn test_commit_and_push_no_changes() {
        let temp = TempDir::new().unwrap();
        run_git(temp.path(), &["init"]).unwrap();
        // No files to commit, should succeed without doing anything
        let result = commit_and_push(temp.path(), "Test commit");
        assert!(result.is_ok());
    }

    #[test]
    fn test_ensure_git_user_succeeds() {
        let temp = TempDir::new().unwrap();
        run_git(temp.path(), &["init"]).unwrap();

        // ensure_git_user should succeed
        ensure_git_user(temp.path()).unwrap();

        // After calling ensure_git_user, we should be able to read user.name
        // (either from local config or global config)
        let name_result = run_git(temp.path(), &["config", "user.name"]);
        assert!(name_result.is_ok(), "user.name should be readable");

        let email_result = run_git(temp.path(), &["config", "user.email"]);
        assert!(email_result.is_ok(), "user.email should be readable");
    }

    #[test]
    fn test_ensure_git_user_sets_local_when_no_global() {
        // This test verifies behavior when no global config exists
        // by checking local config specifically
        let temp = TempDir::new().unwrap();
        run_git(temp.path(), &["init"]).unwrap();

        // Check if global user.name is set
        let has_global = run_git(temp.path(), &["config", "--global", "user.name"]).is_ok();

        ensure_git_user(temp.path()).unwrap();

        if !has_global {
            // Only check local config if there's no global
            let local_name =
                run_git(temp.path(), &["config", "--local", "user.name"]).unwrap_or_default();
            assert_eq!(local_name.trim(), "cterm");
        }
        // If global exists, ensure_git_user correctly doesn't override it
    }

    #[test]
    fn test_git_error_display() {
        let err = GitError::CommandFailed("test error".to_string());
        assert_eq!(format!("{}", err), "Git command failed: test error");

        let err = GitError::InvalidPath;
        assert_eq!(format!("{}", err), "Invalid path");
    }

    #[test]
    fn test_get_directory_remote_url_no_git() {
        let temp = TempDir::new().unwrap();
        assert!(get_directory_remote_url(temp.path()).is_none());
    }

    #[test]
    fn test_get_directory_remote_url_with_remote() {
        let temp = TempDir::new().unwrap();
        run_git(temp.path(), &["init"]).unwrap();
        // Use example.com to avoid git URL rewriting that may be configured for github.com
        run_git(
            temp.path(),
            &[
                "remote",
                "add",
                "origin",
                "https://example.com/test/repo.git",
            ],
        )
        .unwrap();
        assert_eq!(
            get_directory_remote_url(temp.path()),
            Some("https://example.com/test/repo.git".to_string())
        );
    }

    #[test]
    fn test_prepare_working_directory_exists() {
        let temp = TempDir::new().unwrap();
        // Directory already exists, should return Ok(false)
        let result = prepare_working_directory(temp.path(), None).unwrap();
        assert!(!result);
    }

    #[test]
    fn test_prepare_working_directory_create_no_remote() {
        let temp = TempDir::new().unwrap();
        let new_dir = temp.path().join("subdir");
        // Directory doesn't exist, no remote, should create it
        let result = prepare_working_directory(&new_dir, None).unwrap();
        assert!(!result); // false because no cloning was done
        assert!(new_dir.exists());
    }

    #[test]
    fn test_prepare_working_directory_clone_fails_invalid_remote() {
        let temp = TempDir::new().unwrap();
        let new_dir = temp.path().join("cloned");
        // Invalid remote URL should fail
        let result = prepare_working_directory(&new_dir, Some("not-a-valid-url"));
        assert!(result.is_err());
    }

    #[test]
    fn test_clone_repo_creates_parent_dirs() {
        let temp = TempDir::new().unwrap();
        let deep_path = temp.path().join("a").join("b").join("c");
        // Clone will fail because of invalid URL, but parent dirs should be created
        let _ = clone_repo("not-a-valid-url", &deep_path);
        // Parent should exist even if clone failed
        assert!(temp.path().join("a").join("b").exists());
    }

    // =========================================================================
    // Integration tests using local bare repository as remote
    // =========================================================================

    /// Helper to create a bare repository that can be used as a "remote"
    fn create_bare_remote(temp: &TempDir) -> std::path::PathBuf {
        let bare_path = temp.path().join("remote.git");
        fs::create_dir_all(&bare_path).unwrap();
        run_git(&bare_path, &["init", "--bare"]).unwrap();
        bare_path
    }

    /// Helper to create a working repo with a remote pointing to a bare repo
    fn create_repo_with_remote(temp: &TempDir, bare_path: &Path) -> std::path::PathBuf {
        let repo_path = temp.path().join("repo");
        fs::create_dir_all(&repo_path).unwrap();
        run_git(&repo_path, &["init"]).unwrap();

        // Configure user for commits
        run_git(&repo_path, &["config", "user.name", "Test User"]).unwrap();
        run_git(&repo_path, &["config", "user.email", "test@example.com"]).unwrap();

        // Add remote - use file:// URL for local bare repo
        let remote_url = format!("file://{}", bare_path.display());
        run_git(&repo_path, &["remote", "add", "origin", &remote_url]).unwrap();

        repo_path
    }

    /// Helper to create a file and commit it
    fn create_and_commit_file(repo: &Path, filename: &str, content: &str, message: &str) {
        fs::write(repo.join(filename), content).unwrap();
        run_git(repo, &["add", filename]).unwrap();
        run_git(repo, &["commit", "-m", message]).unwrap();
    }

    #[test]
    fn test_push_to_bare_remote() {
        let temp = TempDir::new().unwrap();
        let bare = create_bare_remote(&temp);
        let repo = create_repo_with_remote(&temp, &bare);

        // Create initial commit
        create_and_commit_file(&repo, "config.toml", "key = \"value\"", "Initial commit");

        // Push should succeed
        let result = run_git(&repo, &["push", "-u", "origin", "main"]);
        // If main doesn't work, try master (git version dependent)
        if result.is_err() {
            run_git(&repo, &["branch", "-M", "master"]).unwrap();
            run_git(&repo, &["push", "-u", "origin", "master"]).unwrap();
        }

        // Verify remote has the commit
        let log = run_git(&bare, &["log", "--oneline"]).unwrap();
        assert!(log.contains("Initial commit"));
    }

    #[test]
    fn test_commit_and_push_to_bare_remote() {
        let temp = TempDir::new().unwrap();
        let bare = create_bare_remote(&temp);
        let repo = create_repo_with_remote(&temp, &bare);

        // Create initial commit and push to establish branch
        create_and_commit_file(&repo, "init.txt", "init", "Initial commit");
        let _ = run_git(&repo, &["push", "-u", "origin", "main"]).or_else(|_| {
            run_git(&repo, &["branch", "-M", "master"])?;
            run_git(&repo, &["push", "-u", "origin", "master"])
        });

        // Now test commit_and_push
        fs::write(repo.join("config.toml"), "setting = true").unwrap();
        commit_and_push(&repo, "Update config").unwrap();

        // Verify remote has both commits
        let log = run_git(&bare, &["log", "--oneline"]).unwrap();
        assert!(log.contains("Update config"));
    }

    #[test]
    fn test_pull_from_bare_remote() {
        let temp = TempDir::new().unwrap();
        let bare = create_bare_remote(&temp);

        // Create first repo, commit, and push
        let repo1 = create_repo_with_remote(&temp, &bare);
        create_and_commit_file(&repo1, "config.toml", "version = 1", "Version 1");
        let _ = run_git(&repo1, &["push", "-u", "origin", "main"]).or_else(|_| {
            run_git(&repo1, &["branch", "-M", "master"])?;
            run_git(&repo1, &["push", "-u", "origin", "master"])
        });

        // Clone to second repo
        let repo2 = temp.path().join("repo2");
        let remote_url = format!("file://{}", bare.display());
        clone_repo(&remote_url, &repo2).unwrap();

        // Configure user in repo2
        run_git(&repo2, &["config", "user.name", "Test User"]).unwrap();
        run_git(&repo2, &["config", "user.email", "test@example.com"]).unwrap();

        // Make changes in repo1 and push
        create_and_commit_file(&repo1, "config.toml", "version = 2", "Version 2");
        run_git(&repo1, &["push"]).unwrap();

        // Pull in repo2
        let result = pull_with_conflict_resolution(&repo2).unwrap();
        assert!(matches!(result, PullResult::Updated));

        // Verify repo2 has the new content
        let content = fs::read_to_string(repo2.join("config.toml")).unwrap();
        assert!(content.contains("version = 2"));
    }

    #[test]
    fn test_pull_already_up_to_date() {
        let temp = TempDir::new().unwrap();
        let bare = create_bare_remote(&temp);
        let repo = create_repo_with_remote(&temp, &bare);

        // Create initial commit and push
        create_and_commit_file(&repo, "config.toml", "data = 1", "Initial");
        let _ = run_git(&repo, &["push", "-u", "origin", "main"]).or_else(|_| {
            run_git(&repo, &["branch", "-M", "master"])?;
            run_git(&repo, &["push", "-u", "origin", "master"])
        });

        // Pull should report up to date
        let result = pull_with_conflict_resolution(&repo).unwrap();
        assert!(matches!(result, PullResult::UpToDate));
    }

    #[test]
    fn test_sync_status_commits_ahead() {
        let temp = TempDir::new().unwrap();
        let bare = create_bare_remote(&temp);
        let repo = create_repo_with_remote(&temp, &bare);

        // Create initial commit and push
        create_and_commit_file(&repo, "config.toml", "v1", "Initial");
        let _ = run_git(&repo, &["push", "-u", "origin", "main"]).or_else(|_| {
            run_git(&repo, &["branch", "-M", "master"])?;
            run_git(&repo, &["push", "-u", "origin", "master"])
        });

        // Make local commit without pushing
        create_and_commit_file(&repo, "config.toml", "v2", "Local change");

        let status = get_sync_status(&repo);
        assert!(status.is_repo);
        assert!(status.commits_ahead >= 1);
        assert_eq!(status.commits_behind, 0);
        assert!(!status.has_local_changes); // Changes are committed
        assert!(status.has_unpushed);
    }

    #[test]
    fn test_sync_status_commits_behind() {
        let temp = TempDir::new().unwrap();
        let bare = create_bare_remote(&temp);

        // Create repo1 and push
        let repo1 = create_repo_with_remote(&temp, &bare);
        create_and_commit_file(&repo1, "config.toml", "v1", "Initial");
        let _ = run_git(&repo1, &["push", "-u", "origin", "main"]).or_else(|_| {
            run_git(&repo1, &["branch", "-M", "master"])?;
            run_git(&repo1, &["push", "-u", "origin", "master"])
        });

        // Clone to repo2
        let repo2 = temp.path().join("repo2");
        let remote_url = format!("file://{}", bare.display());
        clone_repo(&remote_url, &repo2).unwrap();
        run_git(&repo2, &["config", "user.name", "Test"]).unwrap();
        run_git(&repo2, &["config", "user.email", "test@example.com"]).unwrap();

        // Push new commit from repo1
        create_and_commit_file(&repo1, "config.toml", "v2", "Remote change");
        run_git(&repo1, &["push"]).unwrap();

        // Fetch in repo2 to update remote tracking
        run_git(&repo2, &["fetch"]).unwrap();

        let status = get_sync_status(&repo2);
        assert!(status.is_repo);
        assert_eq!(status.commits_ahead, 0);
        assert!(status.commits_behind >= 1);
    }

    #[test]
    fn test_sync_status_local_changes() {
        let temp = TempDir::new().unwrap();
        let bare = create_bare_remote(&temp);
        let repo = create_repo_with_remote(&temp, &bare);

        // Create initial commit
        create_and_commit_file(&repo, "config.toml", "v1", "Initial");
        let _ = run_git(&repo, &["push", "-u", "origin", "main"]).or_else(|_| {
            run_git(&repo, &["branch", "-M", "master"])?;
            run_git(&repo, &["push", "-u", "origin", "master"])
        });

        // Make uncommitted changes
        fs::write(repo.join("config.toml"), "v2").unwrap();

        let status = get_sync_status(&repo);
        assert!(status.has_local_changes);
    }

    #[test]
    fn test_conflict_resolution_remote_wins() {
        let temp = TempDir::new().unwrap();
        let bare = create_bare_remote(&temp);

        // Create repo1 and push
        let repo1 = create_repo_with_remote(&temp, &bare);
        create_and_commit_file(&repo1, "config.toml", "original", "Initial");
        let _ = run_git(&repo1, &["push", "-u", "origin", "main"]).or_else(|_| {
            run_git(&repo1, &["branch", "-M", "master"])?;
            run_git(&repo1, &["push", "-u", "origin", "master"])
        });

        // Clone to repo2
        let repo2 = temp.path().join("repo2");
        let remote_url = format!("file://{}", bare.display());
        clone_repo(&remote_url, &repo2).unwrap();
        run_git(&repo2, &["config", "user.name", "Test"]).unwrap();
        run_git(&repo2, &["config", "user.email", "test@example.com"]).unwrap();

        // Make conflicting changes: repo1 changes and pushes
        create_and_commit_file(&repo1, "config.toml", "remote version", "Remote update");
        run_git(&repo1, &["push"]).unwrap();

        // repo2 makes a different change to the same file and commits
        create_and_commit_file(&repo2, "config.toml", "local version", "Local update");

        // Pull should resolve conflict (remote wins based on the implementation)
        let result = pull_with_conflict_resolution(&repo2).unwrap();

        // The implementation does hard reset to remote on rebase failure,
        // so remote version should win
        let content = fs::read_to_string(repo2.join("config.toml")).unwrap();
        // After conflict resolution, content should be from remote
        assert!(
            matches!(result, PullResult::ConflictsResolved(_))
                || matches!(result, PullResult::Updated),
            "Expected conflict resolution or update, got {:?}",
            result
        );
        assert!(
            content.contains("remote version"),
            "Remote version should win, got: {}",
            content
        );
    }

    #[test]
    fn test_pull_with_local_uncommitted_changes() {
        let temp = TempDir::new().unwrap();
        let bare = create_bare_remote(&temp);

        // Create repo1 and push
        let repo1 = create_repo_with_remote(&temp, &bare);
        create_and_commit_file(&repo1, "config.toml", "v1", "Initial");
        create_and_commit_file(&repo1, "other.toml", "data", "Add other file");
        let _ = run_git(&repo1, &["push", "-u", "origin", "main"]).or_else(|_| {
            run_git(&repo1, &["branch", "-M", "master"])?;
            run_git(&repo1, &["push", "-u", "origin", "master"])
        });

        // Clone to repo2
        let repo2 = temp.path().join("repo2");
        let remote_url = format!("file://{}", bare.display());
        clone_repo(&remote_url, &repo2).unwrap();
        run_git(&repo2, &["config", "user.name", "Test"]).unwrap();
        run_git(&repo2, &["config", "user.email", "test@example.com"]).unwrap();

        // Make changes in repo1 and push (to a different file)
        create_and_commit_file(&repo1, "other.toml", "new data", "Update other");
        run_git(&repo1, &["push"]).unwrap();

        // Make uncommitted local changes in repo2 (to config.toml)
        fs::write(repo2.join("config.toml"), "local uncommitted").unwrap();

        // Pull should stash, pull, and restore local changes
        let result = pull_with_conflict_resolution(&repo2);
        assert!(result.is_ok(), "Pull should succeed: {:?}", result);

        // Local uncommitted changes to config.toml should be preserved
        let config_content = fs::read_to_string(repo2.join("config.toml")).unwrap();
        assert!(
            config_content.contains("local uncommitted"),
            "Local changes should be preserved, got: {}",
            config_content
        );

        // Remote changes to other.toml should be pulled
        let other_content = fs::read_to_string(repo2.join("other.toml")).unwrap();
        assert!(
            other_content.contains("new data"),
            "Remote changes should be pulled, got: {}",
            other_content
        );
    }

    #[test]
    fn test_init_with_remote_pulls_existing_content() {
        let temp = TempDir::new().unwrap();
        let bare = create_bare_remote(&temp);

        // Create repo1 and push initial content
        let repo1 = create_repo_with_remote(&temp, &bare);
        create_and_commit_file(&repo1, "config.toml", "remote content", "Initial");
        let _ = run_git(&repo1, &["push", "-u", "origin", "main"]).or_else(|_| {
            run_git(&repo1, &["branch", "-M", "master"])?;
            run_git(&repo1, &["push", "-u", "origin", "master"])
        });

        // Create new repo and init with remote
        let repo2 = temp.path().join("repo2");
        fs::create_dir_all(&repo2).unwrap();
        let remote_url = format!("file://{}", bare.display());
        let result = init_with_remote(&repo2, &remote_url).unwrap();

        // Should pull remote content
        assert_eq!(result, InitResult::PulledRemote);

        // Verify content was pulled
        let content = fs::read_to_string(repo2.join("config.toml")).unwrap();
        assert!(content.contains("remote content"));
    }

    #[test]
    fn test_clone_and_verify_content() {
        let temp = TempDir::new().unwrap();
        let bare = create_bare_remote(&temp);

        // Create source repo with content
        let source = create_repo_with_remote(&temp, &bare);
        create_and_commit_file(
            &source,
            "config.toml",
            "[general]\ntheme = \"dark\"",
            "Config",
        );

        // Create subdirectory and file
        fs::create_dir_all(source.join("themes")).unwrap();
        fs::write(source.join("themes/custom.toml"), "name = \"custom\"").unwrap();
        run_git(&source, &["add", "themes/custom.toml"]).unwrap();
        run_git(&source, &["commit", "-m", "Add theme"]).unwrap();

        let _ = run_git(&source, &["push", "-u", "origin", "main"]).or_else(|_| {
            run_git(&source, &["branch", "-M", "master"])?;
            run_git(&source, &["push", "-u", "origin", "master"])
        });

        // Clone to new location
        let clone_path = temp.path().join("cloned");
        let remote_url = format!("file://{}", bare.display());
        clone_repo(&remote_url, &clone_path).unwrap();

        // Verify all content
        assert!(clone_path.join("config.toml").exists());
        assert!(clone_path.join("themes/custom.toml").exists());

        let config = fs::read_to_string(clone_path.join("config.toml")).unwrap();
        assert!(config.contains("theme = \"dark\""));

        let theme = fs::read_to_string(clone_path.join("themes/custom.toml")).unwrap();
        assert!(theme.contains("name = \"custom\""));
    }

    #[test]
    fn test_multiple_commits_sync() {
        let temp = TempDir::new().unwrap();
        let bare = create_bare_remote(&temp);

        // Create repo1 and push
        let repo1 = create_repo_with_remote(&temp, &bare);
        create_and_commit_file(&repo1, "config.toml", "v1", "Version 1");
        let _ = run_git(&repo1, &["push", "-u", "origin", "main"]).or_else(|_| {
            run_git(&repo1, &["branch", "-M", "master"])?;
            run_git(&repo1, &["push", "-u", "origin", "master"])
        });

        // Clone to repo2
        let repo2 = temp.path().join("repo2");
        let remote_url = format!("file://{}", bare.display());
        clone_repo(&remote_url, &repo2).unwrap();
        run_git(&repo2, &["config", "user.name", "Test"]).unwrap();
        run_git(&repo2, &["config", "user.email", "test@example.com"]).unwrap();

        // Make multiple commits in repo1
        create_and_commit_file(&repo1, "config.toml", "v2", "Version 2");
        create_and_commit_file(&repo1, "config.toml", "v3", "Version 3");
        create_and_commit_file(&repo1, "config.toml", "v4", "Version 4");
        run_git(&repo1, &["push"]).unwrap();

        // Fetch and check status before pull
        run_git(&repo2, &["fetch"]).unwrap();
        let status = get_sync_status(&repo2);
        assert!(status.commits_behind >= 3, "Should be 3+ commits behind");

        // Pull all commits
        let result = pull_with_conflict_resolution(&repo2).unwrap();
        assert!(matches!(result, PullResult::Updated));

        // Verify final state
        let content = fs::read_to_string(repo2.join("config.toml")).unwrap();
        assert!(content.contains("v4"));

        let status = get_sync_status(&repo2);
        assert_eq!(status.commits_behind, 0);
    }
}
