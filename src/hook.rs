use crate::error::{Error, Result};
use std::path::{Path, PathBuf};

const MARKER_BEGIN: &str = "# BEGIN timebomb";
const MARKER_END: &str = "# END timebomb";

/// The block inserted into (or appended to) the pre-commit hook.
const HOOK_BLOCK: &str = "# BEGIN timebomb\ntimebomb check --since HEAD .\n# END timebomb\n";

/// Content of a freshly-created pre-commit hook file.
const NEW_HOOK_CONTENT: &str =
    "#!/bin/sh\nset -e\n# BEGIN timebomb\ntimebomb check --since HEAD .\n# END timebomb\n";

/// Walk up from `path` looking for a `.git` directory or file.
fn find_git_dir(path: &Path) -> Result<PathBuf> {
    let mut current = path.to_path_buf();
    loop {
        let candidate = current.join(".git");
        if candidate.exists() {
            // `.git` may be a file (git worktrees) or a directory — both are valid.
            return Ok(candidate);
        }
        match current.parent() {
            Some(parent) => current = parent.to_path_buf(),
            None => {
                return Err(Error::InvalidArgument(
                    "no .git directory found; is this a git repository?".to_string(),
                ))
            }
        }
    }
}

/// Return true if the hook file already contains the timebomb marker block.
fn hook_has_timebomb_block(content: &str) -> bool {
    content.contains(MARKER_BEGIN)
}

/// Remove the timebomb marker block from `content`, returning the cleaned string.
fn remove_timebomb_block(content: &str) -> String {
    let mut out = String::with_capacity(content.len());
    let mut inside = false;
    for line in content.lines() {
        if line.trim() == MARKER_BEGIN {
            inside = true;
            continue;
        }
        if line.trim() == MARKER_END {
            inside = false;
            continue;
        }
        if !inside {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// Set the executable bit on a file (Unix only; no-op on other platforms).
#[cfg(unix)]
fn make_executable(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::metadata(path).map_err(|e| Error::Io {
        source: e,
        path: Some(path.to_path_buf()),
    })?;
    let mut perms = meta.permissions();
    // Add owner + group + other execute bits.
    let mode = perms.mode() | 0o111;
    perms.set_mode(mode);
    std::fs::set_permissions(path, perms).map_err(|e| Error::Io {
        source: e,
        path: Some(path.to_path_buf()),
    })
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<()> {
    Ok(())
}

/// Install the timebomb pre-commit hook.
///
/// - If the hook already contains the timebomb block, prints a message and exits 0.
/// - If the hook file does not exist, creates it with a shebang + hook block.
/// - If the hook file exists but has no timebomb block, appends the block.
/// - When `yes` is false the user is prompted before any write.
pub fn run_hook_install(path: &Path, yes: bool) -> Result<i32> {
    let git_dir = find_git_dir(path)?;
    let hooks_dir = git_dir.join("hooks");

    // Ensure the hooks directory exists.
    if !hooks_dir.exists() {
        std::fs::create_dir_all(&hooks_dir).map_err(|e| Error::Io {
            source: e,
            path: Some(hooks_dir.clone()),
        })?;
    }

    let hook_path = hooks_dir.join("pre-commit");

    // Check if already installed.
    if hook_path.exists() {
        let existing = std::fs::read_to_string(&hook_path).map_err(|e| Error::Io {
            source: e,
            path: Some(hook_path.clone()),
        })?;
        if hook_has_timebomb_block(&existing) {
            println!(
                "timebomb hook is already installed at {}",
                hook_path.display()
            );
            return Ok(0);
        }

        // Append to existing hook.
        if !yes {
            println!(
                "Will append timebomb block to existing hook at {}",
                hook_path.display()
            );
            println!("Proceed? [y/N] ");
            let mut input = String::new();
            std::io::stdin()
                .read_line(&mut input)
                .map_err(|e| Error::Io {
                    source: e,
                    path: None,
                })?;
            if !input.trim().eq_ignore_ascii_case("y") {
                println!("Aborted.");
                return Ok(0);
            }
        }

        let new_content = format!("{}\n{}", existing.trim_end(), HOOK_BLOCK);
        std::fs::write(&hook_path, &new_content).map_err(|e| Error::Io {
            source: e,
            path: Some(hook_path.clone()),
        })?;
        make_executable(&hook_path)?;
        println!("timebomb hook appended to {}", hook_path.display());
    } else {
        // Create a new hook file.
        if !yes {
            println!("Will create new hook file at {}", hook_path.display());
            println!("Proceed? [y/N] ");
            let mut input = String::new();
            std::io::stdin()
                .read_line(&mut input)
                .map_err(|e| Error::Io {
                    source: e,
                    path: None,
                })?;
            if !input.trim().eq_ignore_ascii_case("y") {
                println!("Aborted.");
                return Ok(0);
            }
        }

        std::fs::write(&hook_path, NEW_HOOK_CONTENT).map_err(|e| Error::Io {
            source: e,
            path: Some(hook_path.clone()),
        })?;
        make_executable(&hook_path)?;
        println!("timebomb hook installed at {}", hook_path.display());
    }

    Ok(0)
}

/// Uninstall the timebomb pre-commit hook.
///
/// - If the hook file does not exist or has no timebomb block, prints a message and exits 0.
/// - If the resulting cleaned file is empty (or only whitespace), deletes it.
/// - Otherwise writes the cleaned content back.
pub fn run_hook_uninstall(path: &Path, yes: bool) -> Result<i32> {
    let git_dir = find_git_dir(path)?;
    let hook_path = git_dir.join("hooks").join("pre-commit");

    if !hook_path.exists() {
        println!("No pre-commit hook found — nothing to uninstall.");
        return Ok(0);
    }

    let content = std::fs::read_to_string(&hook_path).map_err(|e| Error::Io {
        source: e,
        path: Some(hook_path.clone()),
    })?;

    if !hook_has_timebomb_block(&content) {
        println!("timebomb hook is not installed — nothing to uninstall.");
        return Ok(0);
    }

    if !yes {
        println!("Will remove timebomb block from {}", hook_path.display());
        println!("Proceed? [y/N] ");
        let mut input = String::new();
        std::io::stdin()
            .read_line(&mut input)
            .map_err(|e| Error::Io {
                source: e,
                path: None,
            })?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("Aborted.");
            return Ok(0);
        }
    }

    let cleaned = remove_timebomb_block(&content);

    // Lines that count as "real content" (non-empty after trim).
    let has_real_content = cleaned
        .lines()
        .any(|l| !l.trim().is_empty() && l.trim() != "#!/bin/sh" && l.trim() != "set -e");

    if !has_real_content {
        std::fs::remove_file(&hook_path).map_err(|e| Error::Io {
            source: e,
            path: Some(hook_path.clone()),
        })?;
        println!("timebomb hook removed (file deleted — it only contained the timebomb block).");
    } else {
        std::fs::write(&hook_path, &cleaned).map_err(|e| Error::Io {
            source: e,
            path: Some(hook_path.clone()),
        })?;
        println!("timebomb block removed from {}", hook_path.display());
    }

    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Create a minimal fake git repo structure in `tmp` (just a `.git/hooks/` dir).
    fn create_fake_git(tmp: &std::path::Path) {
        std::fs::create_dir_all(tmp.join(".git").join("hooks")).unwrap();
    }

    #[test]
    fn test_hook_install_creates_new_file() {
        let tmp = tempfile::tempdir().unwrap();
        create_fake_git(tmp.path());

        let result = run_hook_install(tmp.path(), true).unwrap();
        assert_eq!(result, 0);

        let hook_path = tmp.path().join(".git").join("hooks").join("pre-commit");
        assert!(hook_path.exists(), "pre-commit hook file should be created");

        let content = std::fs::read_to_string(&hook_path).unwrap();
        assert!(content.contains(MARKER_BEGIN));
        assert!(content.contains(MARKER_END));
        assert!(content.contains("timebomb check --since HEAD ."));

        // Check executable bit on Unix.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(&hook_path).unwrap();
            assert_ne!(
                meta.permissions().mode() & 0o111,
                0,
                "hook should be executable"
            );
        }
    }

    #[test]
    fn test_hook_install_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        create_fake_git(tmp.path());

        // Install twice.
        run_hook_install(tmp.path(), true).unwrap();
        run_hook_install(tmp.path(), true).unwrap();

        let hook_path = tmp.path().join(".git").join("hooks").join("pre-commit");
        let content = std::fs::read_to_string(&hook_path).unwrap();

        // The marker should appear exactly once.
        let count = content.matches(MARKER_BEGIN).count();
        assert_eq!(count, 1, "marker block should appear exactly once");
    }

    #[test]
    fn test_hook_install_appends_to_existing_hook() {
        let tmp = tempfile::tempdir().unwrap();
        create_fake_git(tmp.path());

        let hook_path = tmp.path().join(".git").join("hooks").join("pre-commit");
        {
            let mut f = std::fs::File::create(&hook_path).unwrap();
            writeln!(f, "#!/bin/sh").unwrap();
            writeln!(f, "echo 'existing hook'").unwrap();
        }

        run_hook_install(tmp.path(), true).unwrap();

        let content = std::fs::read_to_string(&hook_path).unwrap();
        assert!(
            content.contains("echo 'existing hook'"),
            "original content preserved"
        );
        assert!(content.contains(MARKER_BEGIN), "timebomb block appended");
        assert!(content.contains("timebomb check --since HEAD ."));
    }

    #[test]
    fn test_hook_uninstall_removes_block() {
        let tmp = tempfile::tempdir().unwrap();
        create_fake_git(tmp.path());

        run_hook_install(tmp.path(), true).unwrap();

        let hook_path = tmp.path().join(".git").join("hooks").join("pre-commit");
        assert!(hook_path.exists());

        run_hook_uninstall(tmp.path(), true).unwrap();

        // File should be gone (it only had the timebomb block).
        assert!(
            !hook_path.exists(),
            "hook file should be deleted when it only had the block"
        );
    }

    #[test]
    fn test_hook_uninstall_preserves_other_content() {
        let tmp = tempfile::tempdir().unwrap();
        create_fake_git(tmp.path());

        let hook_path = tmp.path().join(".git").join("hooks").join("pre-commit");
        {
            let mut f = std::fs::File::create(&hook_path).unwrap();
            writeln!(f, "#!/bin/sh").unwrap();
            writeln!(f, "echo 'my other check'").unwrap();
        }

        run_hook_install(tmp.path(), true).unwrap();
        run_hook_uninstall(tmp.path(), true).unwrap();

        // File should still exist with the other content.
        assert!(
            hook_path.exists(),
            "hook file should remain (has other content)"
        );
        let content = std::fs::read_to_string(&hook_path).unwrap();
        assert!(
            !content.contains(MARKER_BEGIN),
            "timebomb marker should be gone"
        );
        assert!(
            content.contains("my other check"),
            "other content preserved"
        );
    }

    #[test]
    fn test_hook_uninstall_on_missing_hook() {
        let tmp = tempfile::tempdir().unwrap();
        create_fake_git(tmp.path());

        // Uninstall without ever installing — should succeed with exit code 0.
        let result = run_hook_uninstall(tmp.path(), true).unwrap();
        assert_eq!(result, 0);
    }

    #[test]
    fn test_remove_timebomb_block_basic() {
        let input = "line before\n# BEGIN timebomb\ntimebomb check --since HEAD .\n# END timebomb\nline after\n";
        let output = remove_timebomb_block(input);
        assert!(!output.contains(MARKER_BEGIN));
        assert!(!output.contains(MARKER_END));
        assert!(output.contains("line before"));
        assert!(output.contains("line after"));
    }

    #[test]
    fn test_hook_has_timebomb_block() {
        assert!(hook_has_timebomb_block(
            "some content\n# BEGIN timebomb\nstuff\n# END timebomb\n"
        ));
        assert!(!hook_has_timebomb_block("just a regular hook\n"));
    }

    #[test]
    fn test_find_git_dir_not_found() {
        // A directory with no .git anywhere up the tree will fail.
        // Use /tmp directly — it should have no .git unless someone put one there.
        // This test is best-effort; skip if /tmp itself somehow has .git.
        let tmp = tempfile::tempdir().unwrap();
        let result = find_git_dir(tmp.path());
        // Should fail — no .git in the temp dir.
        assert!(result.is_err());
    }

    #[test]
    fn test_find_git_dir_found() {
        let tmp = tempfile::tempdir().unwrap();
        create_fake_git(tmp.path());
        let result = find_git_dir(tmp.path());
        assert!(result.is_ok());
        assert!(result.unwrap().ends_with(".git"));
    }

    #[test]
    fn test_find_git_dir_found_from_subdirectory() {
        // find_git_dir should walk up and find .git even from a nested subdirectory.
        let tmp = tempfile::tempdir().unwrap();
        create_fake_git(tmp.path());
        let subdir = tmp.path().join("a").join("b").join("c");
        std::fs::create_dir_all(&subdir).unwrap();
        let result = find_git_dir(&subdir);
        assert!(result.is_ok());
    }

    #[test]
    fn test_remove_timebomb_block_no_block_is_noop() {
        let input = "#!/bin/sh\necho 'no timebomb here'\n";
        let output = remove_timebomb_block(input);
        // Content is unchanged except possibly trailing newline normalisation.
        assert!(output.contains("echo 'no timebomb here'"));
        assert!(!output.contains(MARKER_BEGIN));
    }

    #[test]
    fn test_remove_timebomb_block_preserves_surrounding_lines() {
        let input = "\
#!/bin/sh\n\
echo before\n\
# BEGIN timebomb\n\
timebomb check --since HEAD .\n\
# END timebomb\n\
echo after\n\
";
        let output = remove_timebomb_block(input);
        assert!(!output.contains(MARKER_BEGIN));
        assert!(!output.contains(MARKER_END));
        assert!(output.contains("echo before"));
        assert!(output.contains("echo after"));
        assert!(!output.contains("timebomb check"));
    }

    #[test]
    fn test_hook_install_creates_hooks_dir_if_missing() {
        // The fake git dir has no hooks/ subdirectory — install should create it.
        let tmp = tempfile::tempdir().unwrap();
        // Create .git directly (no hooks/ subdir).
        std::fs::create_dir_all(tmp.path().join(".git")).unwrap();

        let result = run_hook_install(tmp.path(), true);
        assert!(result.is_ok());

        let hooks_dir = tmp.path().join(".git").join("hooks");
        assert!(hooks_dir.exists());
        assert!(hooks_dir.join("pre-commit").exists());
    }

    #[test]
    fn test_hook_uninstall_no_timebomb_in_existing_hook() {
        // File exists but has no timebomb block — uninstall should succeed silently.
        let tmp = tempfile::tempdir().unwrap();
        create_fake_git(tmp.path());

        let hook_path = tmp.path().join(".git").join("hooks").join("pre-commit");
        std::fs::write(&hook_path, "#!/bin/sh\necho 'unrelated'\n").unwrap();

        let result = run_hook_uninstall(tmp.path(), true).unwrap();
        assert_eq!(result, 0);
        // File still exists, content unchanged.
        let content = std::fs::read_to_string(&hook_path).unwrap();
        assert!(content.contains("unrelated"));
    }

    #[test]
    fn test_new_hook_content_is_executable_script() {
        // The content written for a fresh hook must have a shebang and set -e.
        assert!(NEW_HOOK_CONTENT.starts_with("#!/bin/sh"));
        assert!(NEW_HOOK_CONTENT.contains("set -e"));
        assert!(NEW_HOOK_CONTENT.contains(MARKER_BEGIN));
        assert!(NEW_HOOK_CONTENT.contains(MARKER_END));
    }

    #[test]
    fn test_hook_block_constant_is_valid() {
        // HOOK_BLOCK itself must contain both markers and the check command.
        assert!(HOOK_BLOCK.contains(MARKER_BEGIN));
        assert!(HOOK_BLOCK.contains(MARKER_END));
        assert!(HOOK_BLOCK.contains("timebomb check"));
    }
}
