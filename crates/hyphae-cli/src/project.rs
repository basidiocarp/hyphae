use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use hyphae_core::{GitContext, detect_git_context_from};
use spore::logging::{SpanContext, subprocess_span};

const PROJECT_GIT_TIMEOUT: Duration = Duration::from_secs(5);

/// Detect a project name from the current environment.
/// Resolution order: git repo basename → cwd basename → None
pub fn detect_project() -> Option<String> {
    let mut span_context = SpanContext::for_app("hyphae").with_tool("project_detection");
    if let Ok(cwd) = std::env::current_dir() {
        span_context = span_context.with_workspace_root(cwd.display().to_string());
    }
    let _subprocess_span =
        subprocess_span("git rev-parse --show-toplevel", &span_context).entered();

    let (tx, rx) = mpsc::channel();

    let _git_thread = thread::spawn(move || {
        let result = std::process::Command::new("git")
            .args(["rev-parse", "--show-toplevel"])
            .output();
        let _ = tx.send(result);
    });

    match rx.recv_timeout(PROJECT_GIT_TIMEOUT) {
        Ok(Ok(output)) => {
            if output.status.success() {
                if let Ok(path_str) = std::str::from_utf8(&output.stdout) {
                    let path = PathBuf::from(path_str.trim());
                    if let Some(name) = path.file_name() {
                        return Some(name.to_string_lossy().into_owned());
                    }
                }
            }
            // Fallback: current directory basename
            std::env::current_dir()
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        }
        Ok(Err(_)) => {
            // Fallback: current directory basename
            std::env::current_dir()
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        }
        Err(_) => {
            // Timeout occurred
            tracing::debug!(
                "git rev-parse --show-toplevel timed out after {:?}",
                PROJECT_GIT_TIMEOUT
            );
            std::env::current_dir()
                .ok()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        }
    }
}

pub fn detect_git_context() -> GitContext {
    detect_git_context_from(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_project_returns_string_or_none() {
        let result = detect_project();
        if let Some(name) = result {
            assert!(!name.is_empty());
        }
    }

    #[test]
    fn test_detect_git_context_returns_struct() {
        let ctx = detect_git_context();
        assert!(ctx.branch.is_none() || !ctx.branch.as_deref().unwrap_or_default().is_empty());
        assert!(ctx.worktree.is_none() || !ctx.worktree.as_deref().unwrap_or_default().is_empty());
    }
}
