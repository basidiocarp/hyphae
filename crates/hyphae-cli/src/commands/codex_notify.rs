use anyhow::{Context, Result};
use hyphae_core::{Importance, Memory, MemorySource, MemoryStore};
use hyphae_ingest::session::{NormalizedSession, normalize_codex_event_type, truncate_snippet};
use hyphae_store::SqliteStore;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

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
    #[serde(flatten, default)]
    extra_fields: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone)]
struct CodexMemoryRecord {
    project: String,
    summary: String,
    raw_excerpt: String,
    keywords: Vec<String>,
    dedupe_hash: String,
    source: MemorySource,
    importance: Importance,
    weight: f32,
    topic: String,
}

pub fn run(
    store: &SqliteStore,
    notification_json: String,
    project_override: Option<&str>,
) -> Result<()> {
    let notification = parse_notification(&notification_json)?;

    let Some(record) = build_record(&notification, project_override)? else {
        return Ok(());
    };

    if store.memory_exists_with_keyword(&record.dedupe_hash)? {
        return Ok(());
    }

    let mut keywords = record.keywords.clone();
    keywords.push(format!("hash:{}", record.dedupe_hash));

    let mut builder = Memory::builder(
        record.topic.clone(),
        record.summary.clone(),
        record.importance,
    )
    .keywords(keywords)
    .weight(record.weight)
    .raw_excerpt(record.raw_excerpt.clone())
    .source(record.source.clone());

    if !record.project.is_empty() {
        builder = builder.project(record.project.clone());
    }

    let memory = builder.build();
    store
        .store(memory)
        .context("failed to store Codex lifecycle event")?;

    Ok(())
}

fn parse_notification(notification_json: &str) -> Result<CodexNotification> {
    serde_json::from_str(notification_json).with_context(|| "failed to parse Codex notification")
}

fn build_record(
    notification: &CodexNotification,
    project_override: Option<&str>,
) -> Result<Option<CodexMemoryRecord>> {
    let session = build_normalized_session(notification, project_override);
    let project = session.project().unwrap_or("unknown").to_string();
    let normalized_event_type = normalize_codex_event_type(&notification.event_type);
    let turn_label = notification
        .turn_id
        .as_deref()
        .or(notification.thread_id.as_deref())
        .unwrap_or("unknown");
    if normalized_event_type.is_empty() {
        return Ok(None);
    }

    if normalized_event_type == "agent-turn-complete" {
        return Ok(Some(build_turn_record(
            notification,
            &session,
            project,
            turn_label,
        )));
    }

    Ok(Some(build_lifecycle_record(
        notification,
        &session,
        project,
        turn_label,
        &normalized_event_type,
    )))
}

fn build_turn_record(
    notification: &CodexNotification,
    session: &NormalizedSession,
    project: String,
    turn_label: &str,
) -> CodexMemoryRecord {
    let assistant_snippet = notification
        .last_assistant_message
        .as_deref()
        .map(|s| truncate_snippet(s, 140))
        .unwrap_or_else(|| "turn complete".to_string());

    let input_snippets = notification
        .input_messages
        .iter()
        .take(2)
        .map(value_to_snippet)
        .collect::<Vec<_>>()
        .join(" | ");

    let state_suffix = session
        .codex_lifecycle_state_summary()
        .map(|summary| format!(" [{summary}]"))
        .unwrap_or_default();

    let summary = if input_snippets.is_empty() {
        format!("Codex turn complete in {project} ({turn_label}): {assistant_snippet}")
    } else {
        format!(
            "Codex turn complete in {project} ({turn_label}): {input_snippets} -> {assistant_snippet}"
        )
    } + &state_suffix;

    let dedupe_hash = hash_prefix(&dedupe_source(notification));

    let mut keywords = vec![
        "host:codex".to_string(),
        "event:agent-turn-complete".to_string(),
        "event_kind:summary".to_string(),
    ];
    if let Some(keyword) = session.codex_lifecycle_state_keyword() {
        keywords.push(keyword);
    }
    if let Some(thread_id) = notification.thread_id.as_deref() {
        keywords.push(format!("session_id:{thread_id}"));
        keywords.push(format!("thread_id:{thread_id}"));
    }
    if let Some(turn_id) = notification.turn_id.as_deref() {
        keywords.push(format!("turn_id:{turn_id}"));
    }
    if let Some(cwd) = notification.cwd.as_deref() {
        keywords.push(format!("cwd:{cwd}"));
    }

    let source = notification
        .thread_id
        .clone()
        .map(|thread_id| {
            MemorySource::agent_session(hyphae_core::SessionHost::Codex, thread_id, None)
        })
        .unwrap_or(MemorySource::Manual);
    let topic = format!("session/{project}");

    CodexMemoryRecord {
        project,
        summary,
        raw_excerpt: session.raw_excerpt().join("\n"),
        keywords,
        dedupe_hash,
        source,
        importance: Importance::Medium,
        weight: 0.75,
        topic,
    }
}

fn build_lifecycle_record(
    notification: &CodexNotification,
    session: &NormalizedSession,
    project: String,
    turn_label: &str,
    normalized_event_type: &str,
) -> CodexMemoryRecord {
    let lifecycle_snippet = lifecycle_snippet(notification);
    let state_suffix = session
        .codex_lifecycle_state_summary()
        .map(|summary| format!(" [state: {summary}]"))
        .unwrap_or_default();

    let summary = if lifecycle_snippet.is_empty() {
        format!(
            "Codex lifecycle event {} in {project} ({turn_label}){}",
            normalized_event_type, state_suffix
        )
    } else {
        format!(
            "Codex lifecycle event {} in {project} ({turn_label}): {lifecycle_snippet}{}",
            normalized_event_type, state_suffix
        )
    };

    let dedupe_hash = hash_prefix(&dedupe_source(notification));

    let mut keywords = vec![
        "host:codex".to_string(),
        format!("event:{normalized_event_type}"),
        "event_kind:lifecycle".to_string(),
    ];
    if let Some(keyword) = session.codex_lifecycle_state_keyword() {
        keywords.push(keyword);
    }
    if let Some(thread_id) = notification.thread_id.as_deref() {
        keywords.push(format!("session_id:{thread_id}"));
        keywords.push(format!("thread_id:{thread_id}"));
    }
    if let Some(turn_id) = notification.turn_id.as_deref() {
        keywords.push(format!("turn_id:{turn_id}"));
    }
    if let Some(cwd) = notification.cwd.as_deref() {
        keywords.push(format!("cwd:{cwd}"));
    }

    let source = notification
        .thread_id
        .clone()
        .map(|thread_id| {
            MemorySource::agent_session(hyphae_core::SessionHost::Codex, thread_id, None)
        })
        .unwrap_or(MemorySource::Manual);
    let topic = format!("session/{project}/codex-lifecycle");

    CodexMemoryRecord {
        project,
        summary,
        raw_excerpt: session.raw_excerpt().join("\n"),
        keywords,
        dedupe_hash,
        source,
        importance: Importance::Low,
        weight: 0.35,
        topic,
    }
}

fn build_normalized_session(
    notification: &CodexNotification,
    project_override: Option<&str>,
) -> NormalizedSession {
    let mut session = NormalizedSession::new(hyphae_ingest::transcript::SessionRuntime::Codex);
    session.note_raw_excerpt_line(format!("type: {}", notification.event_type));
    if let Some(thread_id) = notification.thread_id.as_deref() {
        session.note_raw_excerpt_line(format!("thread-id: {thread_id}"));
        session.note_session_id(thread_id);
    }
    if let Some(turn_id) = notification.turn_id.as_deref() {
        session.note_raw_excerpt_line(format!("turn-id: {turn_id}"));
    }
    if let Some(project) = project_override {
        session.note_project(project.to_string());
    }
    if let Some(cwd) = notification.cwd.as_deref() {
        session.note_raw_excerpt_line(format!("cwd: {cwd}"));
        session.note_project_from_cwd(cwd);
    }
    if !notification.event_type.is_empty() {
        let lifecycle_detail = lifecycle_snippet(notification);
        if let Some(recorded) =
            session.record_codex_lifecycle_event(&notification.event_type, &lifecycle_detail)
        {
            session.note_raw_excerpt_line(format!("lifecycle: {}", recorded.note));
        }
    }
    if !notification.input_messages.is_empty() {
        session.note_raw_excerpt_line("input-messages:");
        for message in &notification.input_messages {
            let snippet = value_to_snippet(message);
            session.note_raw_excerpt_line(format!("  - {snippet}"));
            session.note_highlight(&snippet);
            session.note_message();
        }
    }
    if let Some(last_assistant_message) = notification.last_assistant_message.as_deref() {
        session.note_highlight(last_assistant_message);
        session.note_raw_excerpt_line(format!(
            "last-assistant-message: {}",
            truncate_snippet(last_assistant_message, 200)
        ));
        session.note_message();
    }
    if !notification.extra_fields.is_empty() {
        session.note_raw_excerpt_line("extra-fields:");
        for (key, value) in notification.extra_fields.iter().take(5) {
            let snippet = value_to_snippet(value);
            session.note_raw_excerpt_line(format!("  - {key}: {snippet}"));
            session.note_highlight(&format!("{key}: {snippet}"));
        }
    }
    session
}

fn hash_prefix(text: &str) -> String {
    let hash = Sha256::digest(text.as_bytes());
    let hex = format!("{hash:x}");
    hex[..12].to_string()
}

fn dedupe_source(notification: &CodexNotification) -> String {
    let mut parts = vec![normalize_codex_event_type(&notification.event_type)];
    if let Some(thread_id) = &notification.thread_id {
        parts.push(thread_id.clone());
    }
    if let Some(turn_id) = &notification.turn_id {
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
    for (key, value) in &notification.extra_fields {
        parts.push(format!(
            "{key}={}",
            serde_json::to_string(value).unwrap_or_default()
        ));
    }
    parts.join("\n")
}

fn lifecycle_snippet(notification: &CodexNotification) -> String {
    let mut parts = Vec::new();
    if let Some(thread_id) = notification.thread_id.as_deref() {
        parts.push(format!("thread {thread_id}"));
    }
    if let Some(turn_id) = notification.turn_id.as_deref() {
        parts.push(format!("turn {turn_id}"));
    }
    if let Some(cwd) = notification.cwd.as_deref() {
        parts.push(format!("cwd {cwd}"));
    }
    if !notification.input_messages.is_empty() {
        let input = notification
            .input_messages
            .iter()
            .take(2)
            .map(value_to_snippet)
            .filter(|snippet| !snippet.is_empty())
            .collect::<Vec<_>>()
            .join(" | ");
        if !input.is_empty() {
            parts.push(input);
        }
    }
    if let Some(last_assistant_message) = notification.last_assistant_message.as_deref() {
        parts.push(truncate_snippet(last_assistant_message, 120));
    }
    for (key, value) in notification.extra_fields.iter().take(3) {
        let snippet = value_to_snippet(value);
        if !snippet.is_empty() {
            parts.push(format!("{key}: {snippet}"));
        }
    }
    parts.join(" · ")
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

        let record = build_record(&parse_notification(&payload).unwrap(), None)
            .unwrap()
            .unwrap();
        let hash = record.dedupe_hash.clone();
        assert!(record.summary.contains("myapp"));
        assert!(record.summary.contains("turn-9"));
        assert!(record.summary.contains("phase turn-complete"));
        assert!(record.raw_excerpt.contains("thread-id: thread-7"));
        assert!(
            record
                .keywords
                .iter()
                .any(|keyword| keyword == "state:turn-complete")
        );

        run(&store, payload, None).unwrap();

        assert!(store.memory_exists_with_keyword(&hash).unwrap());
    }

    #[test]
    fn test_stores_lifecycle_breadcrumbs_for_non_turn_complete_events() {
        let store = SqliteStore::in_memory().unwrap();
        let payload = serde_json::json!({
            "type": "approval_requested",
            "thread-id": "thread-7",
            "cwd": "/Users/williamnewton/projects/myapp",
            "reason": "needs approval before writing files"
        });
        let serialized = serde_json::to_string(&payload).unwrap();
        let record = build_record(&parse_notification(&serialized).unwrap(), None)
            .unwrap()
            .unwrap();
        assert!(
            record
                .keywords
                .iter()
                .any(|k| k == "event:approval-requested")
        );
        assert!(record.summary.contains("approval-requested"));
        assert!(record.summary.contains("phase awaiting-approval"));
        assert!(record.raw_excerpt.contains("lifecycle: approval-requested"));
        assert!(
            record
                .keywords
                .iter()
                .any(|keyword| keyword == "state:awaiting-approval")
        );

        run(&store, serialized, None).unwrap();

        let results = store
            .search_by_keywords(&["approval-requested"], 10, 0, None)
            .unwrap();
        assert_eq!(results.len(), 1);
        let memory = &results[0];
        assert_eq!(memory.importance, Importance::Low);
        assert!(memory.summary.contains("approval-requested"));
        assert!(memory.summary.contains("needs approval"));
        assert!(
            memory
                .raw_excerpt
                .as_deref()
                .unwrap_or_default()
                .contains("reason: needs approval before writing files")
        );
    }

    #[test]
    fn test_normalized_turn_complete_variants_store_turn_summary() {
        let payload = serde_json::json!({
            "type": "agent_turn_complete",
            "thread-id": "thread-8",
            "turn-id": "turn-4",
            "cwd": "/Users/williamnewton/projects/myapp",
            "last-assistant-message": "Wrapped up the turn."
        });

        let record = build_record(
            &parse_notification(&serde_json::to_string(&payload).unwrap()).unwrap(),
            None,
        )
        .unwrap()
        .unwrap();

        assert_eq!(record.importance, Importance::Medium);
        assert!(
            record
                .keywords
                .iter()
                .any(|keyword| keyword == "event:agent-turn-complete")
        );
        assert!(
            record
                .keywords
                .iter()
                .any(|keyword| keyword == "state:turn-complete")
        );
        assert!(record.summary.contains("Codex turn complete"));
        assert!(!record.summary.contains("Codex lifecycle event"));
        assert!(
            record
                .keywords
                .iter()
                .any(|keyword| keyword == "session_id:thread-8")
        );
    }
}
