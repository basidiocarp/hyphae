//! Session lifecycle MCP tools.
//!
//! Provides `hyphae_session_start`, `hyphae_session_end`, and `hyphae_session_context`
//! for tracking coding sessions across MCP clients.

use serde_json::{Value, json};

use hyphae_core::{Importance, Memory, MemoryStore};
use hyphae_store::SqliteStore;

use crate::protocol::ToolResult;

use super::{get_bounded_i64, get_str, validate_required_string};

/// `hyphae_session_start` — begin a new coding session.
pub(crate) fn tool_session_start(store: &SqliteStore, args: &Value) -> ToolResult {
    let project = match validate_required_string(args, "project") {
        Ok(p) => p,
        Err(e) => return e,
    };
    let task = get_str(args, "task");

    match store.session_start(project, task) {
        Ok((session_id, started_at)) => ToolResult::text(
            json!({
                "session_id": session_id,
                "started_at": started_at,
            })
            .to_string(),
        ),
        Err(e) => ToolResult::error(format!("failed to create session: {e}")),
    }
}

/// `hyphae_session_end` — end a coding session and store summary as memory.
pub(crate) fn tool_session_end(store: &SqliteStore, args: &Value) -> ToolResult {
    let session_id = match validate_required_string(args, "session_id") {
        Ok(s) => s,
        Err(e) => return e,
    };
    let summary = get_str(args, "summary");
    let files_modified = args
        .get("files_modified")
        .and_then(Value::as_array)
        .map(|arr| serde_json::to_string(arr).unwrap_or_default());
    let errors_encountered = args
        .get("errors_encountered")
        .and_then(Value::as_i64)
        .map(|n| n.to_string());

    match store.session_end(
        session_id,
        summary,
        files_modified.as_deref(),
        errors_encountered.as_deref(),
    ) {
        Ok((project, _started_at, task, _ended_at, duration_minutes)) => {
            // Store summary as a Memory for future recall
            if let Some(summary_text) = summary {
                let topic = format!("session/{project}");
                let content = if let Some(task_desc) = &task {
                    format!("Session completed: {task_desc}. {summary_text}")
                } else {
                    format!("Session completed. {summary_text}")
                };

                let memory = Memory::builder(topic, content, Importance::Medium)
                    .keywords(vec!["session".to_string(), project.clone()])
                    .build();

                if let Err(e) = store.store(memory) {
                    return ToolResult::error(format!(
                        "session ended but failed to store summary as memory: {e}"
                    ));
                }
            }

            ToolResult::text(
                json!({
                    "stored": true,
                    "duration_minutes": duration_minutes,
                })
                .to_string(),
            )
        }
        Err(e) => ToolResult::error(format!("{e}")),
    }
}

/// `hyphae_session_context` — retrieve recent session history for a project.
pub(crate) fn tool_session_context(store: &SqliteStore, args: &Value) -> ToolResult {
    let project = match validate_required_string(args, "project") {
        Ok(p) => p,
        Err(e) => return e,
    };
    let limit = get_bounded_i64(args, "limit", 5, 1, 50);

    match store.session_context(project, limit) {
        Ok(sessions) => {
            let session_values: Vec<Value> = sessions
                .iter()
                .map(|s| {
                    json!({
                        "session_id": s.id,
                        "task": s.task,
                        "started_at": s.started_at,
                        "ended_at": s.ended_at,
                        "summary": s.summary,
                        "files_modified": s.files_modified,
                        "errors": s.errors,
                        "status": s.status,
                    })
                })
                .collect();
            let count = session_values.len();

            ToolResult::text(
                json!({
                    "project": project,
                    "sessions": session_values,
                    "count": count,
                })
                .to_string(),
            )
        }
        Err(e) => ToolResult::error(format!("failed to query sessions: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_store() -> SqliteStore {
        SqliteStore::in_memory().expect("in-memory store")
    }

    #[test]
    fn test_session_start() {
        let store = test_store();
        let result = tool_session_start(
            &store,
            &json!({"project": "test-project", "task": "implement feature X"}),
        );
        assert!(!result.is_error, "session_start should succeed");
        let text = &result.content[0].text;
        let parsed: Value = serde_json::from_str(text).expect("valid JSON");
        assert!(parsed["session_id"].as_str().unwrap().starts_with("ses_"));
        assert!(parsed["started_at"].is_string());
    }

    #[test]
    fn test_session_start_missing_project() {
        let store = test_store();
        let result = tool_session_start(&store, &json!({}));
        assert!(result.is_error);
    }

    #[test]
    fn test_session_end() {
        let store = test_store();

        // Start a session
        let start_result = tool_session_start(&store, &json!({"project": "test-proj"}));
        assert!(!start_result.is_error);
        let parsed: Value = serde_json::from_str(&start_result.content[0].text).unwrap();
        let session_id = parsed["session_id"].as_str().unwrap();

        // End it
        let end_result = tool_session_end(
            &store,
            &json!({
                "session_id": session_id,
                "summary": "Implemented session tools",
                "files_modified": ["session.rs", "mod.rs"],
                "errors_encountered": 0,
            }),
        );
        assert!(
            !end_result.is_error,
            "session_end should succeed: {:?}",
            end_result
        );
        let end_parsed: Value = serde_json::from_str(&end_result.content[0].text).unwrap();
        assert!(end_parsed["stored"].as_bool().unwrap());
    }

    #[test]
    fn test_session_end_invalid_id() {
        let store = test_store();
        store.ensure_sessions_table().unwrap();

        let result = tool_session_end(&store, &json!({"session_id": "nonexistent"}));
        assert!(result.is_error);
    }

    #[test]
    fn test_session_context() {
        let store = test_store();

        // Start and end a session
        let start = tool_session_start(&store, &json!({"project": "ctx-proj", "task": "test"}));
        let parsed: Value = serde_json::from_str(&start.content[0].text).unwrap();
        let sid = parsed["session_id"].as_str().unwrap();
        let _ = tool_session_end(&store, &json!({"session_id": sid, "summary": "done"}));

        // Query context
        let ctx = tool_session_context(&store, &json!({"project": "ctx-proj"}));
        assert!(!ctx.is_error);
        let ctx_parsed: Value = serde_json::from_str(&ctx.content[0].text).unwrap();
        assert_eq!(ctx_parsed["count"].as_u64().unwrap(), 1);
        assert_eq!(
            ctx_parsed["sessions"][0]["status"].as_str().unwrap(),
            "completed"
        );
    }

    #[test]
    fn test_session_context_empty() {
        let store = test_store();
        let result = tool_session_context(&store, &json!({"project": "empty-proj"}));
        assert!(!result.is_error);
        let parsed: Value = serde_json::from_str(&result.content[0].text).unwrap();
        assert_eq!(parsed["count"].as_u64().unwrap(), 0);
    }
}
