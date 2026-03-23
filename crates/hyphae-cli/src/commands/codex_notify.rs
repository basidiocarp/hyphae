use anyhow::{Context, Result};
use hyphae_core::{Importance, Memory, MemorySource, MemoryStore};
use hyphae_store::SqliteStore;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::Path;

#[derive(Debug, Deserialize)]
struct CodexNotification {
    #[serde(rename = "type")]
    event_type: String,
    #[serde(rename = "thread-id")]
    thread_id: Option<String>,
    #[serde(rename = "turn-id")]
    turn_id: Option<String>,
    cwd: Option<String>,
    #[serde(rename = "input-messages", default)]
    input_messages: Vec<serde_json::Value>,
    #[serde(rename = "last-assistant-message")]
    last_assistant_message: Option<String>,
}

#[derive(Debug, Clone)]
struct CodexTurnRecord {
    project: String,
    summary: String,
    raw_excerpt: String,
    keywords: Vec<String>,
    dedupe_hash: String,
    source: MemorySource,
}

pub fn run(
    store: &SqliteStore,
    notification_json: String,
    project_override: Option<&str>,
) -> Result<()> {
    let notification = parse_notification(&notification_json)?;
    if notification.event_type != "agent-turn-complete" {
        return Ok(());
    }

    let record = build_turn_record(&notification, project_override)?;
    if store.memory_exists_with_keyword(&record.dedupe_hash)? {
        return Ok(());
    }

    let mut keywords = record.keywords.clone();
    keywords.push(format!("hash:{}", record.dedupe_hash));

    let mut builder = Memory::builder(
        format!("session/{}", record.project),
        record.summary.clone(),
        Importance::Medium,
    )
    .keywords(keywords)
    .raw_excerpt(record.raw_excerpt.clone())
    .source(record.source.clone());

    if !record.project.is_empty() {
        builder = builder.project(record.project.clone());
    }

    let memory = builder.build();
    store
        .store(memory)
        .context("failed to store Codex turn summary")?;

    Ok(())
}

fn parse_notification(notification_json: &str) -> Result<CodexNotification> {
    serde_json::from_str(notification_json).with_context(|| "failed to parse Codex notification")
}

fn build_turn_record(
    notification: &CodexNotification,
    project_override: Option<&str>,
) -> Result<CodexTurnRecord> {
    let project = project_override
        .map(str::to_string)
        .or_else(|| project_from_cwd(notification.cwd.as_deref()))
        .unwrap_or_else(|| "unknown".to_string());

    let thread_id = notification.thread_id.clone();
    let turn_id = notification.turn_id.clone();

    let mut raw_lines = Vec::new();
    raw_lines.push(format!("type: {}", notification.event_type));
    if let Some(thread_id) = &thread_id {
        raw_lines.push(format!("thread-id: {thread_id}"));
    }
    if let Some(turn_id) = &turn_id {
        raw_lines.push(format!("turn-id: {turn_id}"));
    }
    if let Some(cwd) = &notification.cwd {
        raw_lines.push(format!("cwd: {cwd}"));
    }
    if !notification.input_messages.is_empty() {
        raw_lines.push("input-messages:".to_string());
        for message in &notification.input_messages {
            raw_lines.push(format!("  - {}", value_to_snippet(message)));
        }
    }
    if let Some(last_assistant_message) = &notification.last_assistant_message {
        raw_lines.push(format!(
            "last-assistant-message: {}",
            truncate_snippet(last_assistant_message, 200)
        ));
    }

    let assistant_snippet = notification
        .last_assistant_message
        .as_deref()
        .map(|s| truncate_snippet(s, 140))
        .unwrap_or_else(|| "turn complete".to_string());
    let turn_label = turn_id
        .as_deref()
        .or(thread_id.as_deref())
        .unwrap_or("unknown");

    let input_snippets = notification
        .input_messages
        .iter()
        .take(2)
        .map(value_to_snippet)
        .collect::<Vec<_>>()
        .join(" | ");

    let summary = if input_snippets.is_empty() {
        format!("Codex turn complete in {project} ({turn_label}): {assistant_snippet}")
    } else {
        format!(
            "Codex turn complete in {project} ({turn_label}): {input_snippets} -> {assistant_snippet}"
        )
    };

    let dedupe_source = {
        let mut parts = vec![notification.event_type.clone()];
        if let Some(thread_id) = &thread_id {
            parts.push(thread_id.clone());
        }
        if let Some(turn_id) = &turn_id {
            parts.push(turn_id.clone());
        }
        if let Some(cwd) = &notification.cwd {
            parts.push(cwd.clone());
        }
        parts.push(
            notification
                .last_assistant_message
                .clone()
                .unwrap_or_default(),
        );
        for message in &notification.input_messages {
            parts.push(serde_json::to_string(message).unwrap_or_default());
        }
        parts.join("\n")
    };
    let dedupe_hash = hash_prefix(&dedupe_source);

    let mut keywords = vec![
        "host:codex".to_string(),
        "event:agent-turn-complete".to_string(),
    ];
    if let Some(thread_id) = thread_id {
        keywords.push(format!("thread_id:{thread_id}"));
    }
    if let Some(turn_id) = turn_id {
        keywords.push(format!("turn_id:{turn_id}"));
    }
    if let Some(cwd) = &notification.cwd {
        keywords.push(format!("cwd:{cwd}"));
    }

    let source = notification
        .thread_id
        .clone()
        .map(|thread_id| MemorySource::Conversation { thread_id })
        .unwrap_or(MemorySource::Manual);

    Ok(CodexTurnRecord {
        project,
        summary,
        raw_excerpt: raw_lines.join("\n"),
        keywords,
        dedupe_hash,
        source,
    })
}

fn project_from_cwd(cwd: Option<&str>) -> Option<String> {
    let cwd = cwd?;
    Path::new(cwd)
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .filter(|name| !name.is_empty())
}

fn hash_prefix(text: &str) -> String {
    let hash = Sha256::digest(text.as_bytes());
    let hex = format!("{hash:x}");
    hex[..12].to_string()
}

fn truncate_snippet(text: &str, limit: usize) -> String {
    let trimmed = text.trim();
    let mut chars = trimmed.chars();
    let snippet: String = chars.by_ref().take(limit).collect();
    if chars.next().is_some() {
        format!("{snippet}...")
    } else {
        snippet
    }
}

fn value_to_snippet(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(text) => truncate_snippet(text, 120),
        serde_json::Value::Number(number) => number.to_string(),
        serde_json::Value::Bool(bool) => bool.to_string(),
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Array(items) => items
            .iter()
            .map(value_to_snippet)
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" | "),
        serde_json::Value::Object(map) => {
            if let Some(text) = map.get("text").and_then(|v| v.as_str()) {
                return truncate_snippet(text, 120);
            }
            if let Some(text) = map.get("content").and_then(|v| v.as_str()) {
                return truncate_snippet(text, 120);
            }
            serde_json::to_string(value)
                .map(|s| truncate_snippet(&s, 120))
                .unwrap_or_else(|_| "<unserializable>".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyphae_store::SqliteStore;

    #[test]
    fn test_parse_notification_and_store_turn_summary() {
        let store = SqliteStore::in_memory().unwrap();
        let notification = serde_json::json!({
            "type": "agent-turn-complete",
            "thread-id": "thread-7",
            "turn-id": "turn-9",
            "cwd": "/Users/williamnewton/projects/myapp",
            "input-messages": ["Can you summarize the repo?", {"text": "What should I do next?"}],
            "last-assistant-message": "You should release spore first."
        });
        let payload = serde_json::to_string(&notification).unwrap();

        let record = build_turn_record(&parse_notification(&payload).unwrap(), None).unwrap();
        let hash = record.dedupe_hash.clone();
        assert!(record.summary.contains("myapp"));
        assert!(record.summary.contains("turn-9"));
        assert!(record.raw_excerpt.contains("thread-id: thread-7"));

        run(&store, payload, None).unwrap();

        assert!(store.memory_exists_with_keyword(&hash).unwrap());
    }

    #[test]
    fn test_ignores_non_turn_complete_events() {
        let store = SqliteStore::in_memory().unwrap();
        let payload = serde_json::json!({
            "type": "approval-requested",
            "thread-id": "thread-7"
        });
        run(&store, serde_json::to_string(&payload).unwrap(), None).unwrap();
        assert!(!store.memory_exists_with_keyword("thread-7").unwrap());
    }
}
