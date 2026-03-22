use crate::error::{Error, Result};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Returns true if `path` is inside a git repository (i.e. `git rev-parse`
/// succeeds in that directory).
pub fn is_git_repo(path: &Path) -> bool {
    Command::new("git")
        .arg("rev-parse")
        .arg("--git-dir")
        .current_dir(path)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Run `git diff --name-only <git_ref>` from `repo_root` and return the set
/// of relative paths of changed files.
///
/// Returns an error if git is not available, the repo_root is not a git repo,
/// or the ref is invalid.
pub fn git_changed_files(repo_root: &Path, git_ref: &str) -> Result<HashSet<PathBuf>> {
    let mut result = HashSet::new();

    // Run unstaged diff
    let unstaged = run_git_diff(repo_root, git_ref, false)?;
    result.extend(unstaged);

    // Run staged (cached) diff
    let staged = run_git_diff(repo_root, git_ref, true)?;
    result.extend(staged);

    Ok(result)
}

fn run_git_diff(repo_root: &Path, git_ref: &str, cached: bool) -> Result<HashSet<PathBuf>> {
    let mut cmd = Command::new("git");
    cmd.arg("diff").arg("--name-only");
    if cached {
        cmd.arg("--cached");
    }
    cmd.arg(git_ref);
    cmd.current_dir(repo_root);

    let output = cmd.output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Error::InvalidArgument("'git' command not found — is git installed?".to_string())
        } else {
            Error::InvalidArgument(format!("failed to spawn git: {}", e))
        }
    })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.to_lowercase().contains("not a git repository") {
            return Err(Error::InvalidArgument(format!(
                "git diff failed: not a git repository"
            )));
        }
        return Err(Error::InvalidArgument(format!(
            "invalid git ref '{}': {}",
            git_ref, stderr
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let paths = stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| PathBuf::from(l))
        .collect();

    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn git_available() -> bool {
        Command::new("git")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[test]
    fn test_is_git_repo_true() {
        if !git_available() {
            return;
        }
        // Create a fresh temp directory and initialise a git repo in it so
        // this test is environment-independent (the project directory itself
        // may not be inside a git repo in all CI/sandbox environments).
        let tmp = tempfile::tempdir().unwrap();
        let init_ok = Command::new("git")
            .args(["init"])
            .current_dir(tmp.path())
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !init_ok {
            // git init failed for some reason — skip gracefully.
            return;
        }
        assert!(is_git_repo(tmp.path()));
    }

    #[test]
    fn test_is_git_repo_false() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!is_git_repo(tmp.path()));
    }

    /// Initialise a bare-minimum git repo in `dir` with one commit so that
    /// HEAD and diff commands are usable.  Returns false if anything fails.
    fn init_git_repo_with_commit(dir: &std::path::Path) -> bool {
        let run = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(dir)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false)
        };
        // Initialise and create at least one commit so HEAD exists.
        run(&["init"])
            && run(&["config", "user.email", "test@example.com"])
            && run(&["config", "user.name", "Test"])
            && {
                // Create an empty file and commit it.
                std::fs::write(dir.join("init.txt"), b"init").is_ok()
            }
            && run(&["add", "."])
            && run(&["commit", "-m", "init"])
    }

    #[test]
    fn test_git_changed_files_invalid_ref() {
        if !git_available() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        if !init_git_repo_with_commit(tmp.path()) {
            return;
        }
        let result = git_changed_files(tmp.path(), "nonexistent-ref-xyz-abc-999");
        assert!(result.is_err());
    }

    #[test]
    fn test_git_changed_files_head_returns_hashset() {
        if !git_available() {
            return;
        }
        let tmp = tempfile::tempdir().unwrap();
        if !init_git_repo_with_commit(tmp.path()) {
            return;
        }
        let result = git_changed_files(tmp.path(), "HEAD");
        assert!(result.is_ok());
        // Result is a HashSet (possibly empty if there are no diffs against HEAD)
        let _set: HashSet<PathBuf> = result.unwrap();
    }
}
