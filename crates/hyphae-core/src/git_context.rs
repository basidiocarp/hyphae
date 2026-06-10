use std::path::Path;
use std::process::{Command, Stdio};

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

    // Pipe stdout/stderr so the git child cannot inherit the server's fd 1 —
    // for a stdio MCP server that fd is the JSON-RPC channel, and an inherited
    // child writing to it corrupts framing. wait_with_output() below drains both
    // pipes; this is only safe because these git subcommands emit a few bytes.
    // Revisit if git_output is ever extended to verbose-stderr commands.
    command.stdout(Stdio::piped());
    command.stderr(Stdio::piped());
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

    #[test]
    fn captures_branch_inside_git_repo() {
        // Hermetic regression test for the stdout-pipe fix: build a throwaway
        // git repo and confirm detect_git_context_from captures a non-empty
        // branch + worktree. Pre-fix (unpiped child stdout) wait_with_output()
        // read nothing, so both came back empty; post-fix they populate.
        // Self-skips when git is unavailable so constrained CI images don't red.
        let git_present = Command::new("git")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success());
        if !git_present {
            eprintln!("skipping captures_branch_inside_git_repo: git not on PATH");
            return;
        }

        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("hyphae-git-context-pos-{unique}"));
        fs::create_dir_all(&dir).unwrap();

        let run = |args: &[&str]| {
            Command::new("git")
                .current_dir(&dir)
                .args(args)
                .output()
                .expect("git command runs")
        };
        run(&["init", "-q"]);
        // A commit is required for `rev-parse --abbrev-ref HEAD` to resolve a branch.
        // Override config that would otherwise make the commit fail on a CI runner:
        // explicit identity (global may be unset), gpgsign off (global may set it true
        // with no key), and --no-verify (skip any global pre-commit/commit-msg hook).
        run(&[
            "-c",
            "user.email=test@example.com",
            "-c",
            "user.name=test",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--allow-empty",
            "--no-verify",
            "-q",
            "-m",
            "init",
        ]);

        let ctx = detect_git_context_from(Some(&dir));
        let _ = fs::remove_dir_all(&dir);

        assert!(
            ctx.branch.as_deref().is_some_and(|b| !b.is_empty()),
            "branch should be captured inside a git repo, got {:?}",
            ctx.branch
        );
        assert!(
            ctx.worktree.as_deref().is_some_and(|w| !w.is_empty()),
            "worktree should be captured inside a git repo, got {:?}",
            ctx.worktree
        );
    }
}
