//! Parse Claude Code session transcripts (JSONL format) for ingestion into hyphae.

use anyhow::{Context, Result};
use std::collections::HashSet;
use std::path::Path;

/// Summary of a parsed session transcript.
#[derive(Debug, Clone)]
pub struct TranscriptSummary {
    pub session_id: String,
    pub project: String,
    pub message_count: usize,
    pub files_modified: Vec<String>,
    pub commands_run: Vec<String>,
    pub errors: Vec<String>,
}

/// Parse a Claude Code JSONL transcript file into a summary.
pub fn parse_transcript(path: &Path) -> Result<TranscriptSummary> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;

    let mut message_count = 0usize;
    let mut files: HashSet<String> = HashSet::new();
    let mut commands: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    let mut session_id = String::new();
    let mut project = String::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let val: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        // Extract session ID from first message UUID
        if session_id.is_empty() {
            if let Some(uuid) = val.get("uuid").and_then(|u| u.as_str()) {
                session_id = uuid.to_string();
            }
        }

        // Extract project from cwd
        if project.is_empty() {
            if let Some(cwd) = val.get("cwd").and_then(|c| c.as_str()) {
                project = Path::new(cwd)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
            }
        }

        let msg_type = val.get("type").and_then(|t| t.as_str()).unwrap_or("");

        match msg_type {
            "user" | "assistant" => {
                message_count += 1;
            }
            _ => {}
        }

        // Extract files from Edit/Write tool uses
        if let Some(message) = val.get("message") {
            if let Some(content_arr) = message.get("content").and_then(|c| c.as_array()) {
                for item in content_arr {
                    if item.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                        let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("");
                        if let Some(input) = item.get("input") {
                            match name {
                                "Edit" | "Write" | "MultiEdit" => {
                                    if let Some(fp) =
                                        input.get("file_path").and_then(|f| f.as_str())
                                    {
                                        files.insert(fp.to_string());
                                    }
                                }
                                "Bash" => {
                                    if let Some(cmd) = input.get("command").and_then(|c| c.as_str())
                                    {
                                        if commands.len() < 50 {
                                            let short = if cmd.len() > 100 {
                                                format!("{}...", &cmd[..100])
                                            } else {
                                                cmd.to_string()
                                            };
                                            commands.push(short);
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        }

        // Extract errors from tool results
        if let Some(message) = val.get("message") {
            if let Some(content_arr) = message.get("content").and_then(|c| c.as_array()) {
                for item in content_arr {
                    if item.get("type").and_then(|t| t.as_str()) == Some("tool_result")
                        && item
                            .get("is_error")
                            .and_then(|e| e.as_bool())
                            .unwrap_or(false)
                    {
                        if let Some(err_content) = item.get("content").and_then(|c| c.as_str()) {
                            let short = if err_content.len() > 200 {
                                format!("{}...", &err_content[..200])
                            } else {
                                err_content.to_string()
                            };
                            if errors.len() < 20 {
                                errors.push(short);
                            }
                        }
                    }
                }
            }
        }
    }

    // Fallback session ID from filename
    if session_id.is_empty() {
        session_id = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());
    }

    Ok(TranscriptSummary {
        session_id,
        project,
        message_count,
        files_modified: files.into_iter().collect(),
        commands_run: commands,
        errors,
    })
}

/// Convert a transcript summary to searchable text for FTS indexing.
pub fn summary_to_text(summary: &TranscriptSummary) -> String {
    let mut parts = Vec::new();

    parts.push(format!(
        "Session for {}: {} messages",
        summary.project, summary.message_count
    ));

    if !summary.files_modified.is_empty() {
        parts.push(format!(
            "Modified {} files: {}",
            summary.files_modified.len(),
            summary.files_modified.join(", ")
        ));
    }

    if !summary.commands_run.is_empty() {
        let cmd_count = summary.commands_run.len();
        let sample: Vec<&str> = summary
            .commands_run
            .iter()
            .take(10)
            .map(String::as_str)
            .collect();
        parts.push(format!("{cmd_count} commands: {}", sample.join(", ")));
    }

    if !summary.errors.is_empty() {
        parts.push(format!("{} errors encountered", summary.errors.len()));
        for err in summary.errors.iter().take(3) {
            parts.push(format!("  Error: {err}"));
        }
    }

    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_parse_empty_transcript() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.jsonl");
        std::fs::write(&path, "").unwrap();

        let summary = parse_transcript(&path).unwrap();
        assert_eq!(summary.message_count, 0);
        assert!(summary.files_modified.is_empty());
    }

    #[test]
    fn test_parse_transcript_with_messages() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, r#"{{"type":"user","uuid":"abc123","cwd":"/Users/x/projects/myapp","message":{{"role":"user","content":"hello"}}}}"#).unwrap();
        writeln!(f, r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"text","text":"hi"}}]}}}}"#).unwrap();

        let summary = parse_transcript(&path).unwrap();
        assert_eq!(summary.message_count, 2);
        assert_eq!(summary.session_id, "abc123");
        assert_eq!(summary.project, "myapp");
    }

    #[test]
    fn test_summary_to_text() {
        let summary = TranscriptSummary {
            session_id: "test".to_string(),
            project: "myapp".to_string(),
            message_count: 10,
            files_modified: vec!["src/main.rs".to_string()],
            commands_run: vec!["cargo test".to_string()],
            errors: vec![],
        };
        let text = summary_to_text(&summary);
        assert!(text.contains("myapp"));
        assert!(text.contains("10 messages"));
        assert!(text.contains("src/main.rs"));
    }
}
