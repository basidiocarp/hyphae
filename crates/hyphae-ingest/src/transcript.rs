//! Parse Claude Code and Codex session transcripts (JSONL format) for ingestion into hyphae.

use anyhow::{Context, Result};
use std::fmt;
use std::path::Path;

use crate::session::{
    CodexLifecycleState, NormalizedSession, summarize_codex_lifecycle_state, truncate_snippet,
};

/// Runtime that produced a session transcript.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionRuntime {
    ClaudeCode,
    Codex,
}

impl fmt::Display for SessionRuntime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ClaudeCode => write!(f, "Claude Code"),
            Self::Codex => write!(f, "Codex"),
        }
    }
}

/// Summary of a parsed session transcript.
#[derive(Debug, Clone)]
pub struct TranscriptSummary {
    pub runtime: SessionRuntime,
    pub session_id: String,
    pub project: String,
    pub message_count: usize,
    pub files_modified: Vec<String>,
    pub commands_run: Vec<String>,
    pub errors: Vec<String>,
    pub highlights: Vec<String>,
    pub lifecycle_events: Vec<String>,
    pub lifecycle_state: Option<CodexLifecycleState>,
}

/// Parse a Claude Code or Codex JSONL transcript file into a summary.
pub fn parse_transcript(path: &Path) -> Result<TranscriptSummary> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;

    let mut normalized = NormalizedSession::new(SessionRuntime::ClaudeCode);
    let mut runtime = None;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let val: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let line_runtime = detect_runtime(&val);
        if runtime.is_none() {
            runtime = Some(line_runtime);
            normalized = NormalizedSession::new(line_runtime);
        }

        normalized.note_raw_excerpt_line(line);

        match runtime.unwrap_or(line_runtime) {
            SessionRuntime::Codex => {
                parse_codex_line(&val, &mut normalized);
            }
            SessionRuntime::ClaudeCode => {
                parse_claude_line(&val, &mut normalized);
            }
        }
    }

    let runtime = runtime.unwrap_or(SessionRuntime::ClaudeCode);

    Ok(TranscriptSummary::from_normalized(
        normalized,
        runtime,
        path,
        project_from_path(path).unwrap_or_default(),
    ))
}

fn detect_runtime(value: &serde_json::Value) -> SessionRuntime {
    let event_type = value.get("type").and_then(|t| t.as_str()).unwrap_or("");
    if matches!(
        event_type,
        "session_meta" | "turn_context" | "event_msg" | "response_item"
    ) || (value.get("session_id").and_then(|s| s.as_str()).is_some()
        && value.get("text").and_then(|t| t.as_str()).is_some()
        && value.get("message").is_none())
    {
        SessionRuntime::Codex
    } else {
        SessionRuntime::ClaudeCode
    }
}

fn parse_claude_line(value: &serde_json::Value, normalized: &mut NormalizedSession) {
    if let Some(uuid) = value.get("uuid").and_then(|u| u.as_str()) {
        normalized.note_session_id(uuid);
    }

    if let Some(cwd) = value.get("cwd").and_then(|c| c.as_str()) {
        normalized.note_project_from_cwd(cwd);
    }

    let msg_type = value.get("type").and_then(|t| t.as_str()).unwrap_or("");
    match msg_type {
        "user" | "assistant" => {
            normalized.note_message();
        }
        _ => {}
    }

    if let Some(message) = value.get("message") {
        if let Some(content) = message.get("content") {
            capture_text(content, normalized);
            capture_claude_tool_context(content, normalized);
        }
    }
}

fn capture_claude_tool_context(content: &serde_json::Value, normalized: &mut NormalizedSession) {
    let Some(content_arr) = content.as_array() else {
        return;
    };

    for item in content_arr {
        if item.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
            let name = item.get("name").and_then(|n| n.as_str()).unwrap_or("");
            if let Some(input) = item.get("input") {
                match name {
                    "Edit" | "Write" | "MultiEdit" => {
                        if let Some(fp) = input.get("file_path").and_then(|f| f.as_str()) {
                            normalized.note_file_modified(fp);
                        }
                    }
                    "Bash" => {
                        if let Some(cmd) = input.get("command").and_then(|c| c.as_str()) {
                            normalized.note_command(truncate_snippet(cmd, 100));
                        }
                    }
                    _ => {}
                }
            }
        }

        if item.get("type").and_then(|t| t.as_str()) == Some("tool_result")
            && item
                .get("is_error")
                .and_then(|e| e.as_bool())
                .unwrap_or(false)
        {
            if let Some(err_content) = item.get("content").and_then(|c| c.as_str()) {
                normalized.note_error(truncate_snippet(err_content, 200));
            }
        }
    }
}

fn parse_codex_line(value: &serde_json::Value, normalized: &mut NormalizedSession) {
    let event_type = value.get("type").and_then(|t| t.as_str()).unwrap_or("");

    if event_type.is_empty() {
        if let Some(id) = value.get("session_id").and_then(|s| s.as_str()) {
            normalized.note_session_id(id);
        }

        if let Some(cwd) = value.get("cwd").and_then(|c| c.as_str()) {
            normalized.note_project_from_cwd(cwd);
        }

        if let Some(text) = value.get("text").and_then(|t| t.as_str()) {
            normalized.note_message();
            normalized.note_highlight(text);
        }
        return;
    }

    let payload = value.get("payload");
    match event_type {
        "session_meta" => {
            normalized.note_codex_session_started();
            if normalized.session_id().is_none() {
                if let Some(id) = payload
                    .and_then(|p| p.get("id"))
                    .and_then(|id| id.as_str())
                    .or_else(|| value.get("session_id").and_then(|s| s.as_str()))
                {
                    normalized.note_session_id(id);
                }
            }

            if normalized.project().is_none() {
                if let Some(cwd) = payload
                    .and_then(|p| p.get("cwd"))
                    .and_then(|cwd| cwd.as_str())
                    .or_else(|| value.get("cwd").and_then(|cwd| cwd.as_str()))
                {
                    normalized.note_project_from_cwd(cwd);
                }
            }
        }
        "turn_context" => {
            if normalized.project().is_none() {
                if let Some(cwd) = payload
                    .and_then(|p| p.get("cwd"))
                    .and_then(|cwd| cwd.as_str())
                {
                    normalized.note_project_from_cwd(cwd);
                }
            }
        }
        "event_msg" => {
            let payload_type = payload
                .and_then(|p| p.get("type"))
                .and_then(|value| value.as_str())
                .unwrap_or("");

            if matches!(payload_type, "user_message" | "assistant_message") {
                normalized.note_message();
                if let Some(message) = payload.and_then(|p| p.get("message")) {
                    capture_text(message, normalized);
                }
                return;
            }

            if !payload_type.is_empty() && payload_type != "token_count" {
                normalized.note_raw_excerpt_line(format!("event_msg: {payload_type}"));
                normalized.note_highlight(payload_type);
                if let Some(payload) = payload {
                    capture_codex_lifecycle_payload(payload_type, payload, normalized);
                }
            }
        }
        "response_item" => {
            if payload
                .and_then(|p| p.get("type"))
                .and_then(|value| value.as_str())
                == Some("message")
            {
                normalized.note_message();
            }

            if let Some(content) = payload.and_then(|p| p.get("content")) {
                capture_text(content, normalized);
            }
        }
        _ => {}
    }
}

fn capture_codex_lifecycle_payload(
    event_type: &str,
    payload: &serde_json::Value,
    normalized: &mut NormalizedSession,
) {
    if let Some(cwd) = payload.get("cwd").and_then(|value| value.as_str()) {
        normalized.note_project_from_cwd(cwd);
    }

    let lifecycle_detail = summarize_codex_lifecycle_payload(payload);
    if !lifecycle_detail.is_empty() {
        normalized.record_codex_lifecycle_event(event_type, &lifecycle_detail);
    }

    for key in ["message", "reason", "status", "summary", "text", "content"] {
        if let Some(value) = payload.get(key) {
            capture_text(value, normalized);
        }
    }
}

fn summarize_codex_lifecycle_payload(payload: &serde_json::Value) -> String {
    let mut parts = Vec::new();
    for key in ["reason", "message", "status", "summary"] {
        if let Some(value) = payload.get(key) {
            let snippet = value_to_lifecycle_snippet(value);
            if !snippet.is_empty() {
                parts.push(snippet);
            }
        }
    }

    if parts.is_empty() {
        for key in ["text", "content"] {
            if let Some(value) = payload.get(key) {
                let snippet = value_to_lifecycle_snippet(value);
                if !snippet.is_empty() {
                    parts.push(snippet);
                }
            }
        }
    }

    parts.join(" · ")
}

fn project_from_path(path: &Path) -> Option<String> {
    let parent = path.parent()?;
    let name = parent.file_name()?.to_string_lossy().to_string();
    if name.starts_with('.') || name.is_empty() {
        None
    } else {
        Some(name)
    }
}

fn capture_text(value: &serde_json::Value, normalized: &mut NormalizedSession) {
    if let Some(text) = value.as_str() {
        normalized.note_highlight(text);
        return;
    }

    if let Some(obj) = value.as_object() {
        if let Some(text) = obj.get("text").and_then(|t| t.as_str()) {
            normalized.note_highlight(text);
        }
        if let Some(content) = obj.get("content") {
            capture_text(content, normalized);
        }
        return;
    }

    if let Some(items) = value.as_array() {
        for item in items {
            capture_text(item, normalized);
        }
    }
}

fn value_to_lifecycle_snippet(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => truncate_snippet(text, 140),
        serde_json::Value::Number(number) => number.to_string(),
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Null => String::new(),
        serde_json::Value::Array(items) => items
            .iter()
            .map(value_to_lifecycle_snippet)
            .filter(|snippet| !snippet.is_empty())
            .collect::<Vec<_>>()
            .join(" | "),
        serde_json::Value::Object(map) => {
            if let Some(text) = map.get("text").and_then(|value| value.as_str()) {
                return truncate_snippet(text, 140);
            }
            if let Some(text) = map.get("content").and_then(|value| value.as_str()) {
                return truncate_snippet(text, 140);
            }
            serde_json::to_string(value)
                .map(|s| truncate_snippet(&s, 140))
                .unwrap_or_else(|_| "<unserializable>".to_string())
        }
    }
}

impl TranscriptSummary {
    fn from_normalized(
        normalized: NormalizedSession,
        runtime: SessionRuntime,
        path: &Path,
        fallback_project: String,
    ) -> Self {
        let session_id = normalized
            .session_id()
            .map(str::to_string)
            .unwrap_or_else(|| {
                path.file_stem()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| "unknown".to_string())
            });

        let project = normalized
            .project()
            .map(str::to_string)
            .unwrap_or(fallback_project);

        Self {
            runtime,
            session_id,
            project,
            message_count: normalized.message_count(),
            files_modified: normalized.files_modified().iter().cloned().collect(),
            commands_run: normalized.commands_run().to_vec(),
            errors: normalized.errors().to_vec(),
            highlights: normalized.highlights().to_vec(),
            lifecycle_events: normalized.lifecycle_events().to_vec(),
            lifecycle_state: normalized.codex_lifecycle_state().cloned(),
        }
    }
}

/// Convert a transcript summary to searchable text for FTS indexing.
pub fn summary_to_text(summary: &TranscriptSummary) -> String {
    let mut parts = Vec::new();
    let project = if summary.project.is_empty() {
        "unknown".to_string()
    } else {
        summary.project.clone()
    };

    parts.push(format!(
        "{} session for {}: {} messages",
        summary.runtime, project, summary.message_count
    ));

    if !summary.highlights.is_empty() {
        parts.push(format!(
            "Key excerpts: {}",
            summary
                .highlights
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join(" | ")
        ));
    }

    if !summary.lifecycle_events.is_empty() {
        parts.push(format!(
            "Lifecycle events: {}",
            summary
                .lifecycle_events
                .iter()
                .take(4)
                .cloned()
                .collect::<Vec<_>>()
                .join(" | ")
        ));
    }

    if let Some(state) = &summary.lifecycle_state {
        let state_summary = summarize_codex_lifecycle_state(state);
        if !state_summary.is_empty() {
            parts.push(format!("Lifecycle state: {state_summary}"));
        }
    }

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
        assert_eq!(summary.runtime, SessionRuntime::ClaudeCode);
        assert_eq!(summary.message_count, 0);
        assert!(summary.files_modified.is_empty());
        assert!(summary.highlights.is_empty());
    }

    #[test]
    fn test_parse_claude_transcript_with_messages() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, r#"{{"type":"user","uuid":"abc123","cwd":"/Users/x/projects/myapp","message":{{"role":"user","content":"hello"}}}}"#).unwrap();
        writeln!(f, r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"text","text":"hi"}}]}}}}"#).unwrap();

        let summary = parse_transcript(&path).unwrap();
        assert_eq!(summary.runtime, SessionRuntime::ClaudeCode);
        assert_eq!(summary.message_count, 2);
        assert_eq!(summary.session_id, "abc123");
        assert_eq!(summary.project, "myapp");
        assert!(summary.highlights.iter().any(|s| s.contains("hello")));
        assert!(summary.highlights.iter().any(|s| s.contains("hi")));
    }

    #[test]
    fn test_parse_codex_transcript_with_text_entries() {
        let dir = tempfile::tempdir().unwrap();
        let codex_dir = dir.path().join(".codex");
        std::fs::create_dir(&codex_dir).unwrap();
        let path = codex_dir.join("history.jsonl");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"{{"session_id":"sess-42","ts":1774162605,"text":"//help"}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"session_id":"sess-42","ts":1774162687,"text":"Please review the repo"}}"#
        )
        .unwrap();

        let summary = parse_transcript(&path).unwrap();
        assert_eq!(summary.runtime, SessionRuntime::Codex);
        assert_eq!(summary.message_count, 2);
        assert_eq!(summary.session_id, "sess-42");
        assert!(summary.project.is_empty());
        assert!(summary.highlights.iter().any(|s| s.contains("//help")));
        assert!(
            summary
                .highlights
                .iter()
                .any(|s| s.contains("Please review"))
        );
    }

    #[test]
    fn test_parse_codex_session_transcript_with_event_payloads() {
        let dir = tempfile::tempdir().unwrap();
        let codex_dir = dir.path().join(".codex").join("sessions");
        std::fs::create_dir_all(&codex_dir).unwrap();
        let path = codex_dir.join("rollout-test.jsonl");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"{{"type":"session_meta","timestamp":"2026-03-23T11:00:00.000Z","payload":{{"id":"rollout-1","cwd":"/Users/test/demo-project","model_provider":"openai"}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"turn_context","timestamp":"2026-03-23T11:00:01.000Z","payload":{{"cwd":"/Users/test/demo-project","model":"gpt-5.4"}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"event_msg","timestamp":"2026-03-23T11:00:02.000Z","payload":{{"type":"user_message","message":"Please review the repo"}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"response_item","timestamp":"2026-03-23T11:00:03.000Z","payload":{{"type":"message","role":"assistant","content":[{{"type":"output_text","text":"Start with the tests."}}]}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"event_msg","timestamp":"2026-03-23T11:00:03.500Z","payload":{{"type":"approval_requested","reason":"needs approval before writing files","message":"Approve the file write?"}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"event_msg","timestamp":"2026-03-23T11:00:04.000Z","payload":{{"type":"token_count","info":{{"total_token_usage":{{"input_tokens":1200,"output_tokens":480,"cached_input_tokens":250}}}}}}}}"#
        )
        .unwrap();

        let summary = parse_transcript(&path).unwrap();
        assert_eq!(summary.runtime, SessionRuntime::Codex);
        assert_eq!(summary.session_id, "rollout-1");
        assert_eq!(summary.project, "demo-project");
        assert_eq!(summary.message_count, 2);
        assert!(
            summary
                .highlights
                .iter()
                .any(|snippet| snippet.contains("Please review the repo"))
        );
        assert!(
            summary
                .highlights
                .iter()
                .any(|snippet| snippet.contains("Start with the tests"))
        );
        assert!(
            summary
                .highlights
                .iter()
                .any(|snippet| snippet.contains("approval_requested"))
        );
        assert!(
            summary
                .highlights
                .iter()
                .any(|snippet| snippet.contains("needs approval before writing files"))
        );
        assert!(
            summary
                .lifecycle_events
                .iter()
                .any(|snippet| snippet.contains("approval-requested"))
        );
        assert_eq!(
            summary.lifecycle_state,
            Some(CodexLifecycleState {
                session_started: true,
                session_ended: false,
                turns_completed: 0,
                approvals_requested: 1,
                approvals_resolved: 0,
                pending_approvals: 1,
                last_event: Some("approval-requested".to_string()),
            })
        );
    }

    #[test]
    fn test_summary_to_text_mentions_runtime() {
        let summary = TranscriptSummary {
            runtime: SessionRuntime::Codex,
            session_id: "test".to_string(),
            project: "myapp".to_string(),
            message_count: 10,
            files_modified: vec!["src/main.rs".to_string()],
            commands_run: vec!["cargo test".to_string()],
            errors: vec![],
            highlights: vec!["turn summary".to_string()],
            lifecycle_events: vec!["approval-requested: needs approval".to_string()],
            lifecycle_state: Some(CodexLifecycleState {
                session_started: true,
                session_ended: false,
                turns_completed: 1,
                approvals_requested: 1,
                approvals_resolved: 0,
                pending_approvals: 1,
                last_event: Some("agent-turn-complete".to_string()),
            }),
        };
        let text = summary_to_text(&summary);
        assert!(text.contains("Codex session"));
        assert!(text.contains("turn summary"));
        assert!(text.contains("Lifecycle events"));
        assert!(text.contains("Lifecycle state"));
        assert!(text.contains("phase awaiting-approval"));
        assert!(text.contains("1 turn(s) completed"));
        assert!(text.contains("10 messages"));
        assert!(text.contains("src/main.rs"));
    }

    #[test]
    fn test_parse_codex_transcript_reconciles_state_transitions() {
        let dir = tempfile::tempdir().unwrap();
        let codex_dir = dir.path().join(".codex").join("sessions");
        std::fs::create_dir_all(&codex_dir).unwrap();
        let path = codex_dir.join("reconcile-test.jsonl");
        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(
            f,
            r#"{{"type":"session_meta","timestamp":"2026-03-23T11:00:00.000Z","payload":{{"id":"rollout-2","cwd":"/Users/test/demo-project","model_provider":"openai"}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"event_msg","timestamp":"2026-03-23T11:00:01.000Z","payload":{{"type":"approval_requested","reason":"needs approval before writing files"}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"event_msg","timestamp":"2026-03-23T11:00:02.000Z","payload":{{"type":"approval_approved","message":"approval granted"}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"event_msg","timestamp":"2026-03-23T11:00:03.000Z","payload":{{"type":"agent_turn_complete","summary":"turn wrapped"}}}}"#
        )
        .unwrap();
        writeln!(
            f,
            r#"{{"type":"event_msg","timestamp":"2026-03-23T11:00:04.000Z","payload":{{"type":"session_complete","status":"done"}}}}"#
        )
        .unwrap();

        let summary = parse_transcript(&path).unwrap();
        assert_eq!(
            summary.lifecycle_state,
            Some(CodexLifecycleState {
                session_started: true,
                session_ended: true,
                turns_completed: 1,
                approvals_requested: 1,
                approvals_resolved: 1,
                pending_approvals: 0,
                last_event: Some("session-complete".to_string()),
            })
        );
    }
}
