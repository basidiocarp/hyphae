//! Session lifecycle MCP tools.
//!
//! Provides `hyphae_session_start`, `hyphae_session_end`, and `hyphae_session_context`
//! for tracking coding sessions across MCP clients.

use serde_json::{Value, json};

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
    let (project_root, worktree_id) =
        normalize_identity(get_str(args, "project_root"), get_str(args, "worktree_id"));
    let scope = get_str(args, "scope");
    let runtime_session_id = get_str(args, "runtime_session_id");

    match store.session_start_identity_with_runtime(
        project,
        task,
        project_root,
        worktree_id,
        scope,
        runtime_session_id,
    ) {
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

/// `hyphae_session_end` — end a coding session.
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
        Ok((project, _started_at, task, _ended_at, duration_minutes)) => ToolResult::text(
            json!({
                "stored": true,
                "project": project,
                "task": task,
                "duration_minutes": duration_minutes,
            })
            .to_string(),
        ),
        Err(e) => ToolResult::error(format!("{e}")),
    }
}

/// `hyphae_session_context` — retrieve recent session history for a project.
pub(crate) fn tool_session_context(store: &SqliteStore, args: &Value) -> ToolResult {
    let project = match validate_required_string(args, "project") {
        Ok(p) => p,
        Err(e) => return e,
    };
    let (project_root, worktree_id) =
        normalize_identity(get_str(args, "project_root"), get_str(args, "worktree_id"));
    let scope = get_str(args, "scope");
    let limit = get_bounded_i64(args, "limit", 5, 1, 50);

    match store.session_context_identity(project, project_root, worktree_id, scope, limit) {
        Ok(sessions) => {
            let session_values: Vec<Value> = sessions
                .iter()
                .map(|s| {
                    json!({
                        "session_id": s.id,
                        "project_root": s.project_root,
                        "worktree_id": s.worktree_id,
                        "scope": s.scope,
                        "runtime_session_id": s.runtime_session_id,
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
                    "project_root": project_root,
                    "worktree_id": worktree_id,
                    "scope": scope,
                    "sessions": session_values,
                    "count": count,
                })
                .to_string(),
            )
        }
        Err(e) => ToolResult::error(format!("failed to query sessions: {e}")),
    }
}

fn normalize_identity<'a>(
    project_root: Option<&'a str>,
    worktree_id: Option<&'a str>,
) -> (Option<&'a str>, Option<&'a str>) {
    match (project_root, worktree_id) {
        (Some(project_root), Some(worktree_id)) => (Some(project_root), Some(worktree_id)),
        _ => (None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyphae_core::MemoryStore;
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
    fn test_session_start_with_scope() {
        let store = test_store();
        let first = tool_session_start(
            &store,
            &json!({"project": "test-project", "task": "worker a", "scope": "worker-a"}),
        );
        let second = tool_session_start(
            &store,
            &json!({"project": "test-project", "task": "worker b", "scope": "worker-b"}),
        );

        let first_parsed: Value = serde_json::from_str(&first.content[0].text).unwrap();
        let second_parsed: Value = serde_json::from_str(&second.content[0].text).unwrap();
        assert_ne!(first_parsed["session_id"], second_parsed["session_id"]);
    }

    #[test]
    fn test_session_start_accepts_identity_v1_fields() {
        let store = test_store();
        let result = tool_session_start(
            &store,
            &json!({
                "project": "test-project",
                "task": "worker a",
                "project_root": "/repo/test-project",
                "worktree_id": "wt-alpha",
                "scope": "worker-a",
                "runtime_session_id": "claude-session-1"
            }),
        );
        assert!(!result.is_error);

        let parsed: Value = serde_json::from_str(&result.content[0].text).unwrap();
        let session_id = parsed["session_id"].as_str().unwrap();
        let session = store.session_status(session_id).unwrap().unwrap();
        assert_eq!(session.project_root.as_deref(), Some("/repo/test-project"));
        assert_eq!(session.worktree_id.as_deref(), Some("wt-alpha"));
        assert_eq!(
            session.runtime_session_id.as_deref(),
            Some("claude-session-1")
        );
    }

    #[test]
    fn test_session_start_partial_identity_normalizes_to_legacy_behavior() {
        let store = test_store();
        let first = tool_session_start(
            &store,
            &json!({
                "project": "test-project",
                "task": "worker a",
                "project_root": "/repo/test-project",
                "scope": "worker-a"
            }),
        );
        assert!(!first.is_error);

        let second = tool_session_start(
            &store,
            &json!({
                "project": "test-project",
                "task": "worker b",
                "scope": "worker-a"
            }),
        );
        assert!(!second.is_error);

        let first_parsed: Value = serde_json::from_str(&first.content[0].text).unwrap();
        let second_parsed: Value = serde_json::from_str(&second.content[0].text).unwrap();
        assert_eq!(first_parsed["session_id"], second_parsed["session_id"]);

        let session_id = first_parsed["session_id"].as_str().unwrap();
        let session = store.session_status(session_id).unwrap().unwrap();
        assert!(session.project_root.is_none());
        assert!(session.worktree_id.is_none());

        let ctx = tool_session_context(
            &store,
            &json!({
                "project": "test-project",
                "project_root": "/repo/test-project",
                "scope": "worker-a"
            }),
        );
        assert!(!ctx.is_error);
        let ctx_parsed: Value = serde_json::from_str(&ctx.content[0].text).unwrap();
        assert!(ctx_parsed["project_root"].is_null());
        assert!(ctx_parsed["worktree_id"].is_null());
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
        assert_eq!(end_parsed["project"].as_str(), Some("test-proj"));
        assert_eq!(end_parsed["task"].as_str(), None);

        let session_memories = store
            .get_by_topic("session/test-proj", Some("test-proj"))
            .unwrap();
        assert!(session_memories.is_empty());
    }

    #[test]
    fn test_session_end_invalid_id() {
        let store = test_store();

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
        assert!(ctx_parsed["sessions"][0]["scope"].is_null());
    }

    #[test]
    fn test_session_context_with_scope_filter() {
        let store = test_store();

        let worker_a = tool_session_start(
            &store,
            &json!({"project": "ctx-proj", "task": "worker a", "scope": "worker-a"}),
        );
        let worker_b = tool_session_start(
            &store,
            &json!({"project": "ctx-proj", "task": "worker b", "scope": "worker-b"}),
        );
        assert!(!worker_a.is_error);
        assert!(!worker_b.is_error);

        let ctx =
            tool_session_context(&store, &json!({"project": "ctx-proj", "scope": "worker-a"}));
        assert!(!ctx.is_error);
        let ctx_parsed: Value = serde_json::from_str(&ctx.content[0].text).unwrap();
        assert_eq!(ctx_parsed["count"].as_u64().unwrap(), 1);
        assert_eq!(
            ctx_parsed["sessions"][0]["scope"].as_str().unwrap(),
            "worker-a"
        );
    }

    #[test]
    fn test_session_context_returns_identity_v1_fields() {
        let store = test_store();

        let start = tool_session_start(
            &store,
            &json!({
                "project": "ctx-proj",
                "task": "worker a",
                "project_root": "/repo/ctx-proj",
                "worktree_id": "wt-alpha",
                "scope": "worker-a"
            }),
        );
        assert!(!start.is_error);

        let ctx = tool_session_context(
            &store,
            &json!({
                "project": "ctx-proj",
                "project_root": "/repo/ctx-proj",
                "worktree_id": "wt-alpha",
                "scope": "worker-a"
            }),
        );
        assert!(!ctx.is_error);

        let parsed: Value = serde_json::from_str(&ctx.content[0].text).unwrap();
        assert_eq!(parsed["project_root"].as_str(), Some("/repo/ctx-proj"));
        assert_eq!(parsed["worktree_id"].as_str(), Some("wt-alpha"));
        assert_eq!(
            parsed["sessions"][0]["project_root"].as_str(),
            Some("/repo/ctx-proj")
        );
        assert_eq!(
            parsed["sessions"][0]["worktree_id"].as_str(),
            Some("wt-alpha")
        );
    }

    #[test]
    fn test_session_context_identity_respects_scope() {
        let store = test_store();

        let worker_a = tool_session_start(
            &store,
            &json!({
                "project": "ctx-proj",
                "task": "worker a",
                "project_root": "/repo/ctx-proj",
                "worktree_id": "wt-alpha",
                "scope": "worker-a"
            }),
        );
        let worker_b = tool_session_start(
            &store,
            &json!({
                "project": "ctx-proj",
                "task": "worker b",
                "project_root": "/repo/ctx-proj",
                "worktree_id": "wt-alpha",
                "scope": "worker-b"
            }),
        );
        assert!(!worker_a.is_error);
        assert!(!worker_b.is_error);

        let ctx = tool_session_context(
            &store,
            &json!({
                "project": "ctx-proj",
                "project_root": "/repo/ctx-proj",
                "worktree_id": "wt-alpha",
                "scope": "worker-a"
            }),
        );
        assert!(!ctx.is_error);

        let parsed: Value = serde_json::from_str(&ctx.content[0].text).unwrap();
        assert_eq!(parsed["count"].as_u64().unwrap(), 1);
        assert_eq!(parsed["sessions"][0]["scope"].as_str(), Some("worker-a"));
        assert_eq!(parsed["sessions"][0]["task"].as_str(), Some("worker a"));
    }

    #[test]
    fn test_session_context_identity_does_not_return_legacy_scope_rows() {
        let store = test_store();

        let legacy = tool_session_start(
            &store,
            &json!({"project": "ctx-proj", "task": "worker a", "scope": "worker-a"}),
        );
        assert!(!legacy.is_error);

        let ctx = tool_session_context(
            &store,
            &json!({
                "project": "ctx-proj",
                "project_root": "/repo/ctx-proj",
                "worktree_id": "wt-alpha",
                "scope": "worker-a"
            }),
        );
        assert!(!ctx.is_error);

        let parsed: Value = serde_json::from_str(&ctx.content[0].text).unwrap();
        assert_eq!(parsed["count"].as_u64(), Some(0));
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
