use std::path::PathBuf;

/// Detect a project name from the current environment.
/// Resolution order: git repo basename → cwd basename → None
pub fn detect_project() -> Option<String> {
    if let Ok(output) = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
    {
        if output.status.success() {
            if let Ok(path_str) = std::str::from_utf8(&output.stdout) {
                let path = PathBuf::from(path_str.trim());
                if let Some(name) = path.file_name() {
                    return Some(name.to_string_lossy().into_owned());
                }
            }
        }
    }
    // Fallback: current directory basename
    std::env::current_dir()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
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
}
