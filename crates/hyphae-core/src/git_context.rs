use std::path::Path;
use std::process::Command;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GitContext {
    pub branch: Option<String>,
    pub worktree: Option<String>,
}

#[must_use]
pub fn detect_git_context_from(cwd: Option<&Path>) -> GitContext {
    GitContext {
        branch: git_output(["rev-parse", "--abbrev-ref", "HEAD"], cwd)
            .filter(|value| !value.is_empty()),
        worktree: git_output(["rev-parse", "--show-toplevel"], cwd)
            .filter(|value| !value.is_empty()),
    }
}

/// Returns the current git HEAD commit hash, or None if not in a git repo.
#[must_use]
pub fn current_git_hash(cwd: Option<&Path>) -> Option<String> {
    git_output(["rev-parse", "HEAD"], cwd).filter(|s| !s.is_empty())
}

fn git_output<const N: usize>(args: [&str; N], cwd: Option<&Path>) -> Option<String> {
    let mut command = Command::new("git");
    command.args(args);
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }

    let child = command.spawn().ok()?;
    let deadline = std::time::Duration::from_secs(5);
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });

    match rx.recv_timeout(deadline) {
        Ok(Ok(out)) if out.status.success() => {
            let value = String::from_utf8(out.stdout).ok()?;
            Some(value.trim().to_string())
        }
        _ => {
            tracing::debug!("hyphae: git context detection timed out or failed");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn returns_empty_context_outside_git_repo() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hyphae-git-context-{unique}"));
        fs::create_dir_all(&dir).unwrap();
        let ctx = detect_git_context_from(Some(&dir));
        let _ = fs::remove_dir_all(&dir);
        assert_eq!(ctx, GitContext::default());
    }
}
