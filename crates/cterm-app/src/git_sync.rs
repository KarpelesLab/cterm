//! Git-backed configuration sync
//!
//! Provides git operations for syncing configuration across machines.

use std::path::Path;
use std::process::Command;

use thiserror::Error;

/// Git sync errors
#[derive(Error, Debug)]
pub enum GitError {
    #[error("Git command failed: {0}")]
    CommandFailed(String),

    #[error("Invalid path")]
    InvalidPath,

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
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
    if !is_git_repo(config_dir) {
        return Ok(false);
    }

    // Check if we have a remote configured
    if get_remote_url(config_dir).is_none() {
        return Ok(false);
    }

    // First fetch to see what's available
    run_git(config_dir, &["fetch", "origin"])?;

    // Determine which branch to pull (main or master)
    let branch = if run_git(config_dir, &["rev-parse", "--verify", "origin/main"]).is_ok() {
        "main"
    } else if run_git(config_dir, &["rev-parse", "--verify", "origin/master"]).is_ok() {
        "master"
    } else {
        log::warn!("No main or master branch found on remote");
        return Ok(false);
    };

    // Try to pull with rebase and autostash
    let output = run_git(
        config_dir,
        &["pull", "--rebase", "--autostash", "origin", branch],
    )?;

    // Check if "Already up to date" or similar
    let up_to_date = output.contains("Already up to date")
        || output.contains("up-to-date")
        || output.contains("Already up-to-date");

    Ok(!up_to_date)
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

/// Run a git command and return stdout
fn run_git(dir: &Path, args: &[&str]) -> Result<String, GitError> {
    log::trace!("Running git {:?} in {}", args, dir.display());

    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(GitError::Io)?;

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
}
