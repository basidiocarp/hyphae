//! Session lifecycle MCP tools.
//!
//! Provides `hyphae_session_start`, `hyphae_session_end`, and `hyphae_session_context`
//! for tracking coding sessions across MCP clients.

use serde_json::{Value, json};
use spore::logging::workflow_span;

use hyphae_core::{
    Embedder, Importance, MemoirStore, Memory, MemoryStore, SCOPED_IDENTITY_SCHEMA_VERSION,
    ScopedIdentity, WmOp, WmSection, WorkingMemory,
};
use hyphae_store::SqliteStore;

use crate::protocol::ToolResult;

use super::{
    ToolTraceContext, get_bounded_i64, get_str, normalize_identity, resolve_workspace_root,
    validate_required_string, workflow_span_context,
};

/// `hyphae_session_start` — begin a new coding session.
pub(crate) fn tool_session_start(
    store: &SqliteStore,
    embedder: Option<&dyn Embedder>,
    args: &Value,
    trace: &ToolTraceContext,
) -> ToolResult {
    let project = match validate_required_string(args, "project") {
        Ok(p) => p,
        Err(e) => return e,
    };
    let task = get_str(args, "task");
    let (project_root, worktree_id) =
        normalize_identity(get_str(args, "project_root"), get_str(args, "worktree_id"));
    let scope = get_str(args, "scope");
    let runtime_session_id = get_str(args, "runtime_session_id");
    let context_signals = args.get("context_signals");
    let workflow_context =
        workflow_span_context(trace, resolve_workspace_root(args), runtime_session_id);
    let _workflow_span = workflow_span("session_start", &workflow_context).entered();

    match store.session_start_identity_with_runtime_and_context_signals(
        project,
        task,
        project_root,
        worktree_id,
        scope,
        runtime_session_id,
        context_signals,
        embedder,
    ) {
        Ok((session_id, started_at, recalled_context)) => {
            let session_context = build_session_context(store, &session_id, project);
            ToolResult::text(
                json!({
                    "schema_version": SCOPED_IDENTITY_SCHEMA_VERSION,
                    "session_id": session_id,
                    "started_at": started_at,
                    "scoped_identity": ScopedIdentity::new(
                        Some(project),
                        project_root,
                        worktree_id,
                        scope,
                        runtime_session_id,
                    ),
                    "recalled_context": recalled_context
                        .iter()
                        .map(|(memory, score)| json!({
                            "content": memory.summary.clone(),
                            "topic": memory.topic.clone(),
                            "score": score,
                        }))
                        .collect::<Vec<_>>(),
                    "session_context": session_context,
                })
                .to_string(),
            )
        }
        Err(e) => ToolResult::error(format!("failed to create session: {e}")),
    }
}

/// Build a `SessionContext` bundle for a project by running pre-baked multi-queries.
///
/// Failures in any sub-query are silently ignored so session start cannot fail due to
/// an empty store or a search error.
fn build_session_context(store: &SqliteStore, session_id: &str, project: &str) -> Value {
    // Recent episodic memories for this project (FTS search scoped to project).
    let recent_work: Vec<Value> = store
        .search_fts_scoped(project, 5, 0, Some(project), None)
        .unwrap_or_default()
        .into_iter()
        .map(|m| {
            json!({
                "topic": m.topic,
                "summary": m.summary,
            })
        })
        .collect();

    // Log recall event for the memories retrieved via FTS search
    let memory_ids: Vec<String> = store
        .search_fts_scoped(project, 5, 0, Some(project), None)
        .unwrap_or_default()
        .into_iter()
        .map(|m| m.id.to_string())
        .collect();
    if !memory_ids.is_empty() {
        if let Err(e) = store.log_recall_event(
            Some(session_id),
            "session_start_context",
            &memory_ids,
            Some(project),
        ) {
            tracing::warn!("failed to log session_start recall event: {e}");
        }
    }

    // Lessons from past corrections and resolved errors for this project.
    let mut lesson_memories = Vec::new();
    for topic in ["corrections", "errors/resolved", "tests/resolved"] {
        if let Ok(memories) = store.get_by_topic(topic, Some(project)) {
            lesson_memories.extend(memories);
        }
    }
    let known_patterns: Vec<Value> = lesson_memories
        .into_iter()
        .take(10)
        .map(|m| {
            json!({
                "topic": m.topic,
                "summary": m.summary,
            })
        })
        .collect();

    // Permanent memoirs (knowledge graphs) — global, not project-scoped.
    let established_facts: Vec<Value> = store
        .list_memoirs()
        .unwrap_or_default()
        .into_iter()
        .take(10)
        .map(|memoir| {
            json!({
                "name": memoir.name,
                "description": memoir.description,
            })
        })
        .collect();

    json!({
        "recent_work": recent_work,
        "known_patterns": known_patterns,
        "established_facts": established_facts,
        "open_items": [],
        "environment": null,
        "env_artifact_stale": false,
    })
}

fn build_working_memory(
    store: &SqliteStore,
    session_id: &str,
    task: Option<String>,
    project: &str,
    summary: Option<&str>,
    files_modified: Option<&str>,
) -> WorkingMemory {
    let session_title = task.as_deref().unwrap_or(session_id).to_string();

    let current_state = summary.unwrap_or("Session ended.").to_string();

    let task_and_goals = session_title.clone();

    // Populate key facts and decisions from topic-scoped lookup.
    // Decision memories are stored under "decisions/{project}" by convention.
    let decisions_topic = format!("decisions/{project}");
    let key_facts = match store.get_by_topic(&decisions_topic, Some(project)) {
        Ok(memories) if !memories.is_empty() => memories
            .iter()
            .take(10)
            .map(|m| format!("- {}", m.summary.lines().next().unwrap_or("")))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => "None identified.".to_string(),
    };

    // Extract files from files_modified JSON array
    let files_context = match files_modified {
        Some(json_str) => match serde_json::from_str::<Vec<String>>(json_str) {
            Ok(files) => {
                if files.is_empty() {
                    "None modified.".to_string()
                } else {
                    files
                        .iter()
                        .map(|f| format!("- {}", f))
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            }
            Err(_) => "None recorded.".to_string(),
        },
        None => "None recorded.".to_string(),
    };

    // Populate errors and corrections from topic-scoped lookup
    let errors = match store.get_by_topic("errors/resolved", Some(project)) {
        Ok(memories) if !memories.is_empty() => memories
            .iter()
            .take(5)
            .map(|m| format!("- {}", m.summary.lines().next().unwrap_or("")))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => "None encountered.".to_string(),
    };

    WorkingMemory {
        schema_version: "1".to_string(),
        session_id: session_id.to_string(),
        session_title: WmSection {
            op: WmOp::Update,
            content: session_title,
        },
        current_state: WmSection {
            op: WmOp::Update,
            content: current_state,
        },
        task_and_goals: WmSection {
            op: WmOp::Update,
            content: task_and_goals,
        },
        key_facts_and_decisions: WmSection {
            op: WmOp::Update,
            content: key_facts,
        },
        files_and_context: WmSection {
            op: WmOp::Update,
            content: files_context,
        },
        errors_and_corrections: WmSection {
            op: WmOp::Update,
            content: errors,
        },
        open_issues: WmSection {
            op: WmOp::Update,
            content: "None identified.".to_string(),
        },
    }
}

/// `hyphae_session_end` — end a coding session.
pub(crate) fn tool_session_end(
    store: &SqliteStore,
    args: &Value,
    trace: &ToolTraceContext,
) -> ToolResult {
    let session_id = match validate_required_string(args, "session_id") {
        Ok(s) => s,
        Err(e) => return e,
    };
    let workflow_context =
        workflow_span_context(trace, resolve_workspace_root(args), Some(session_id));
    let _workflow_span = workflow_span("session_end", &workflow_context).entered();
    let summary = get_str(args, "summary");
    let files_modified = args
        .get("files_modified")
        .and_then(Value::as_array)
        .map(|arr| serde_json::to_string(arr).unwrap_or_default());
    let errors_encountered = args
        .get("errors_encountered")
        .and_then(Value::as_i64)
        .map(|n| n.to_string());
    let scoped_identity = match store.session_status(session_id) {
        Ok(Some(session)) => Some(session.scoped_identity()),
        Ok(None) => None,
        Err(e) => return ToolResult::error(format!("failed to query session: {e}")),
    };

    match store.session_end(
        session_id,
        summary,
        files_modified.as_deref(),
        errors_encountered.as_deref(),
    ) {
        Ok((project, _started_at, task, _ended_at, duration_minutes)) => {
            // Build working memory document (best-effort, errors silently ignored)
            let wm = build_working_memory(
                store,
                session_id,
                task.clone(),
                &project,
                summary,
                files_modified.as_deref(),
            );
            let working_memory_json = match serde_json::to_value(&wm) {
                Ok(v) => {
                    if let Ok(json_str) = serde_json::to_string(&wm) {
                        let memory = Memory::builder(
                            "sessions/working-memory".into(),
                            json_str,
                            Importance::High,
                        )
                        .project(project.clone())
                        .build();
                        if let Err(e) = store.store(memory) {
                            tracing::warn!("failed to store working memory: {e}");
                        }
                    }
                    v
                }
                Err(e) => {
                    tracing::warn!("failed to serialize working memory: {e}");
                    json!({"schema_version": "1", "session_id": session_id, "error": "serialization failed"})
                }
            };

            ToolResult::text(
                json!({
                    "schema_version": SCOPED_IDENTITY_SCHEMA_VERSION,
                    "stored": true,
                    "project": project,
                    "scoped_identity": scoped_identity,
                    "task": task,
                    "duration_minutes": duration_minutes,
                    "working_memory": working_memory_json,
                })
                .to_string(),
            )
        }
        Err(e) => ToolResult::error(format!("{e}")),
    }
}

/// `hyphae_session_context` — retrieve recent session history for a project.
pub(crate) fn tool_session_context(
    store: &SqliteStore,
    args: &Value,
    trace: &ToolTraceContext,
) -> ToolResult {
    let project = match validate_required_string(args, "project") {
        Ok(p) => p,
        Err(e) => return e,
    };
    let (project_root, worktree_id) =
        normalize_identity(get_str(args, "project_root"), get_str(args, "worktree_id"));
    let scope = get_str(args, "scope");
    let limit = get_bounded_i64(args, "limit", 5, 1, 50);
    let workflow_context = workflow_span_context(trace, resolve_workspace_root(args), None);
    let _workflow_span = workflow_span("session_context", &workflow_context).entered();

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
                    "schema_version": SCOPED_IDENTITY_SCHEMA_VERSION,
                    "project": project,
                    "project_root": project_root,
                    "worktree_id": worktree_id,
                    "scope": scope,
                    "scoped_identity": ScopedIdentity::new(
                        Some(project),
                        project_root,
                        worktree_id,
                        scope,
                        None,
                    ),
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
    use hyphae_core::{Importance, Memory, MemoryStore};
    use serde_json::json;

    fn test_store() -> SqliteStore {
        SqliteStore::in_memory().expect("in-memory store")
    }

    #[test]
    fn test_session_start() {
        let store = test_store();
        let result = tool_session_start(
            &store,
            None,
            &json!({"project": "test-project", "task": "implement feature X"}),
            &ToolTraceContext::default(),
        );
        assert!(!result.is_error, "session_start should succeed");
        let text = &result.content[0].text;
        let parsed: Value = serde_json::from_str(text).expect("valid JSON");
        assert_eq!(parsed["schema_version"].as_str(), Some("1.0"));
        assert!(parsed["session_id"].as_str().unwrap().starts_with("ses_"));
        assert!(parsed["started_at"].is_string());
        assert_eq!(
            parsed["scoped_identity"]["project"].as_str(),
            Some("test-project")
        );
        assert!(parsed["recalled_context"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_session_start_with_context_signals_returns_recalled_context() {
        let store = test_store();
        store
            .store(
                Memory::builder(
                    "session_scope".into(),
                    "session_scope context aware recall feat context aware recall build failed"
                        .into(),
                    Importance::Medium,
                )
                .project("test-project".into())
                .worktree("/tmp/demo-project".into())
                .build(),
            )
            .unwrap();

        let result = tool_session_start(
            &store,
            None,
            &json!({
                "project": "test-project",
                "task": "implement feature X",
                "project_root": "/tmp/demo-project",
                "worktree_id": "git:demo",
                "scope": "worker-a",
                "context_signals": {
                    "recent_files": ["/repo/demo/src/session_scope.rs"],
                    "active_errors": ["context aware recall"],
                    "git_branch": "feat/context-aware-recall"
                }
            }),
            &ToolTraceContext::default(),
        );
        assert!(!result.is_error);

        let text = &result.content[0].text;
        let parsed: Value = serde_json::from_str(text).expect("valid JSON");
        assert_eq!(parsed["schema_version"].as_str(), Some("1.0"));
        let recalled = parsed["recalled_context"].as_array().unwrap();
        assert!(!recalled.is_empty());
        assert_eq!(recalled[0]["topic"].as_str(), Some("session_scope"));
    }

    #[test]
    fn test_session_start_with_scope() {
        let store = test_store();
        let first = tool_session_start(
            &store,
            None,
            &json!({"project": "test-project", "task": "worker a", "scope": "worker-a"}),
            &ToolTraceContext::default(),
        );
        let second = tool_session_start(
            &store,
            None,
            &json!({"project": "test-project", "task": "worker b", "scope": "worker-b"}),
            &ToolTraceContext::default(),
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
            None,
            &json!({
                "project": "test-project",
                "task": "worker a",
                "project_root": "/repo/test-project",
                "worktree_id": "wt-alpha",
                "scope": "worker-a",
                "runtime_session_id": "claude-session-1"
            }),
            &ToolTraceContext::default(),
        );
        assert!(!result.is_error);

        let parsed: Value = serde_json::from_str(&result.content[0].text).unwrap();
        assert_eq!(
            parsed["scoped_identity"]["project_root"].as_str(),
            Some("/repo/test-project")
        );
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
            None,
            &json!({
                "project": "test-project",
                "task": "worker a",
                "project_root": "/repo/test-project",
                "scope": "worker-a"
            }),
            &ToolTraceContext::default(),
        );
        assert!(!first.is_error);

        let second = tool_session_start(
            &store,
            None,
            &json!({
                "project": "test-project",
                "task": "worker b",
                "scope": "worker-a"
            }),
            &ToolTraceContext::default(),
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
            &ToolTraceContext::default(),
        );
        assert!(!ctx.is_error);
        let ctx_parsed: Value = serde_json::from_str(&ctx.content[0].text).unwrap();
        assert!(ctx_parsed["project_root"].is_null());
        assert!(ctx_parsed["worktree_id"].is_null());
    }

    #[test]
    fn test_session_start_missing_project() {
        let store = test_store();
        let result = tool_session_start(&store, None, &json!({}), &ToolTraceContext::default());
        assert!(result.is_error);
    }

    #[test]
    fn test_session_end() {
        let store = test_store();

        // Start a session
        let start_result = tool_session_start(
            &store,
            None,
            &json!({"project": "test-proj"}),
            &ToolTraceContext::default(),
        );
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
            &ToolTraceContext::default(),
        );
        assert!(
            !end_result.is_error,
            "session_end should succeed: {:?}",
            end_result
        );
        let end_parsed: Value = serde_json::from_str(&end_result.content[0].text).unwrap();
        assert!(end_parsed["stored"].as_bool().unwrap());
        assert_eq!(end_parsed["schema_version"].as_str(), Some("1.0"));
        assert_eq!(end_parsed["project"].as_str(), Some("test-proj"));
        assert_eq!(
            end_parsed["scoped_identity"]["project"].as_str(),
            Some("test-proj")
        );
        assert_eq!(end_parsed["task"].as_str(), None);
        assert!(
            !end_parsed["working_memory"].is_null(),
            "working_memory must be present and non-null"
        );
        assert_eq!(
            end_parsed["working_memory"]["schema_version"].as_str(),
            Some("1")
        );
        assert!(
            end_parsed["working_memory"]["session_id"].is_string(),
            "working_memory must include session_id"
        );
        assert!(
            end_parsed["working_memory"]["current_state"]["content"]
                .as_str()
                .is_some(),
            "working_memory current_state must have content"
        );

        let session_memories = store
            .get_by_topic("session/test-proj", Some("test-proj"))
            .unwrap();
        assert!(session_memories.is_empty());
    }

    #[test]
    fn test_session_end_working_memory_includes_seeded_decisions() {
        let store = test_store();

        // Seed a decision memory under "decisions/test-proj"
        store
            .store(
                Memory::builder(
                    "decisions/test-proj".into(),
                    "Chose SQLite over Postgres for local-first storage".into(),
                    Importance::High,
                )
                .project("test-proj".into())
                .build(),
            )
            .unwrap();

        let start_result = tool_session_start(
            &store,
            None,
            &json!({"project": "test-proj"}),
            &ToolTraceContext::default(),
        );
        let parsed: Value = serde_json::from_str(&start_result.content[0].text).unwrap();
        let session_id = parsed["session_id"].as_str().unwrap();

        let end_result = tool_session_end(
            &store,
            &json!({"session_id": session_id, "summary": "Done"}),
            &ToolTraceContext::default(),
        );
        assert!(!end_result.is_error);
        let end_parsed: Value = serde_json::from_str(&end_result.content[0].text).unwrap();
        let key_facts = end_parsed["working_memory"]["key_facts_and_decisions"]["content"]
            .as_str()
            .unwrap_or("");
        assert!(
            key_facts.contains("SQLite"),
            "key_facts_and_decisions must include the seeded decision memory, got: {key_facts}"
        );
    }

    #[test]
    fn test_session_end_invalid_id() {
        let store = test_store();

        let result = tool_session_end(
            &store,
            &json!({"session_id": "nonexistent"}),
            &ToolTraceContext::default(),
        );
        assert!(result.is_error);
    }

    #[test]
    fn test_session_context() {
        let store = test_store();

        // Start and end a session
        let start = tool_session_start(
            &store,
            None,
            &json!({"project": "ctx-proj", "task": "test"}),
            &ToolTraceContext::default(),
        );
        let parsed: Value = serde_json::from_str(&start.content[0].text).unwrap();
        let sid = parsed["session_id"].as_str().unwrap();
        let _ = tool_session_end(
            &store,
            &json!({"session_id": sid, "summary": "done"}),
            &ToolTraceContext::default(),
        );

        // Query context
        let ctx = tool_session_context(
            &store,
            &json!({"project": "ctx-proj"}),
            &ToolTraceContext::default(),
        );
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
            None,
            &json!({"project": "ctx-proj", "task": "worker a", "scope": "worker-a"}),
            &ToolTraceContext::default(),
        );
        let worker_b = tool_session_start(
            &store,
            None,
            &json!({"project": "ctx-proj", "task": "worker b", "scope": "worker-b"}),
            &ToolTraceContext::default(),
        );
        assert!(!worker_a.is_error);
        assert!(!worker_b.is_error);

        let ctx = tool_session_context(
            &store,
            &json!({"project": "ctx-proj", "scope": "worker-a"}),
            &ToolTraceContext::default(),
        );
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
            None,
            &json!({
                "project": "ctx-proj",
                "task": "worker a",
                "project_root": "/repo/ctx-proj",
                "worktree_id": "wt-alpha",
                "scope": "worker-a"
            }),
            &ToolTraceContext::default(),
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
            &ToolTraceContext::default(),
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
            None,
            &json!({
                "project": "ctx-proj",
                "task": "worker a",
                "project_root": "/repo/ctx-proj",
                "worktree_id": "wt-alpha",
                "scope": "worker-a"
            }),
            &ToolTraceContext::default(),
        );
        let worker_b = tool_session_start(
            &store,
            None,
            &json!({
                "project": "ctx-proj",
                "task": "worker b",
                "project_root": "/repo/ctx-proj",
                "worktree_id": "wt-alpha",
                "scope": "worker-b"
            }),
            &ToolTraceContext::default(),
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
            &ToolTraceContext::default(),
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
            None,
            &json!({"project": "ctx-proj", "task": "worker a", "scope": "worker-a"}),
            &ToolTraceContext::default(),
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
            &ToolTraceContext::default(),
        );
        assert!(!ctx.is_error);

        let parsed: Value = serde_json::from_str(&ctx.content[0].text).unwrap();
        assert_eq!(parsed["count"].as_u64(), Some(0));
    }

    #[test]
    fn test_session_context_empty() {
        let store = test_store();
        let result = tool_session_context(
            &store,
            &json!({"project": "empty-proj"}),
            &ToolTraceContext::default(),
        );
        assert!(!result.is_error);
        let parsed: Value = serde_json::from_str(&result.content[0].text).unwrap();
        assert_eq!(parsed["count"].as_u64().unwrap(), 0);
    }

    #[test]
    fn test_session_start_empty_store_returns_empty_recalled_context() {
        let store = test_store();

        // Session-start with no memories in the store should return success with empty context
        let result = tool_session_start(
            &store,
            None,
            &json!({
                "project": "empty-memory-proj",
                "task": "test task",
                "project_root": "/tmp/empty",
                "worktree_id": "git:empty",
                "scope": "test-scope",
                "context_signals": {
                    "recent_files": ["/tmp/empty/src/main.rs"],
                    "active_errors": ["test error"],
                    "git_branch": "feat/empty-store-test"
                }
            }),
            &ToolTraceContext::default(),
        );

        // Should not return error even with empty memory store
        assert!(
            !result.is_error,
            "session_start should succeed with empty store: {:?}",
            result
        );

        let text = &result.content[0].text;
        let parsed: Value = serde_json::from_str(text).expect("valid JSON");

        // Verify session was created successfully
        assert_eq!(parsed["schema_version"].as_str(), Some("1.0"));
        assert!(parsed["session_id"].as_str().unwrap().starts_with("ses_"));

        // Verify recalled_context is an empty array (valid empty result, not an error)
        let recalled = parsed["recalled_context"].as_array().unwrap();
        assert!(
            recalled.is_empty(),
            "Empty memory store should return empty recalled_context, not an error"
        );
    }

    #[test]
    fn test_session_start_with_context_signals_returns_gracefully_when_store_has_memories() {
        let store = test_store();

        // Store a memory with topic that matches recall test pattern
        store
            .store(
                Memory::builder(
                    "malform_analysis".into(),
                    "malform analysis malform analysis graceful graceful handling error recovery"
                        .into(),
                    Importance::Medium,
                )
                .project("malform-proj".into())
                .worktree("/tmp/malform-project".into())
                .build(),
            )
            .unwrap();

        // Session-start with context signals that trigger recall.
        // This verifies that the recall path completes successfully and doesn't panic
        // even if some internal search operations encounter edge cases.
        let result = tool_session_start(
            &store,
            None,
            &json!({
                "project": "malform-proj",
                "task": "test graceful handling",
                "project_root": "/tmp/malform-project",
                "worktree_id": "git:malform",
                "scope": "test-scope",
                "context_signals": {
                    "recent_files": ["/repo/demo/src/malform_analysis.rs"],
                    "active_errors": ["graceful handling"],
                    "git_branch": "feat/error-recovery"
                }
            }),
            &ToolTraceContext::default(),
        );

        // Session should be created successfully without panicking on any search or recall issues
        assert!(
            !result.is_error,
            "session_start should not panic or error: {:?}",
            result
        );

        let text = &result.content[0].text;
        let parsed: Value = serde_json::from_str(text).expect("valid JSON");

        // Verify a valid session was created
        assert_eq!(parsed["schema_version"].as_str(), Some("1.0"));
        assert!(parsed["session_id"].as_str().unwrap().starts_with("ses_"));

        // Verify recalled_context is an array (may be empty or populated, both are valid)
        let recalled = parsed["recalled_context"].as_array().unwrap();
        // The key assertion: we get a valid response without panicking,
        // regardless of whether the search found matches or not
        let _ = recalled;
    }

    #[test]
    fn test_session_start_returns_session_context_bundle() {
        let store = test_store();

        let result = tool_session_start(
            &store,
            None,
            &json!({"project": "bundle-proj", "task": "test session context"}),
            &ToolTraceContext::default(),
        );
        assert!(!result.is_error);

        let text = &result.content[0].text;
        let parsed: Value = serde_json::from_str(text).expect("valid JSON");

        let ctx = &parsed["session_context"];
        assert!(ctx.is_object(), "session_context must be an object");
        assert!(
            ctx["recent_work"].is_array(),
            "recent_work must be an array"
        );
        assert!(
            ctx["known_patterns"].is_array(),
            "known_patterns must be an array"
        );
        assert!(
            ctx["established_facts"].is_array(),
            "established_facts must be an array"
        );
        assert!(ctx["open_items"].is_array(), "open_items must be an array");
        assert!(
            ctx["environment"].is_null(),
            "environment must be null until H-05"
        );
        assert_eq!(
            ctx["env_artifact_stale"].as_bool(),
            Some(false),
            "env_artifact_stale defaults false"
        );
    }

    #[test]
    fn test_session_start_logs_recall_event_when_memories_retrieved() {
        let store = test_store();

        // Store some memories in the project with content that matches the project name
        // This ensures the FTS search with "test-proj" as the query will find them
        store
            .store(
                Memory::builder(
                    "session_context".into(),
                    "test-proj session context test memory important content".into(),
                    Importance::High,
                )
                .project("test-proj".into())
                .build(),
            )
            .unwrap();

        // Start a session which will retrieve memories and should log a recall event
        let result = tool_session_start(
            &store,
            None,
            &json!({"project": "test-proj", "task": "test recall logging"}),
            &ToolTraceContext::default(),
        );
        assert!(!result.is_error, "session_start should succeed");

        let text = &result.content[0].text;
        let parsed: Value = serde_json::from_str(text).expect("valid JSON");
        let session_id = parsed["session_id"].as_str().unwrap();

        // Verify that a recall event was logged
        let count = store
            .count_recall_events(Some(session_id), Some("test-proj"), None)
            .expect("count_recall_events should succeed");

        assert!(
            count > 0,
            "A recall event should have been logged for session_start_context"
        );
    }
}
