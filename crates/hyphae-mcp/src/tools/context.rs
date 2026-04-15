// ─────────────────────────────────────────────────────────────────────────────
// Context-gathering MCP tool
// ─────────────────────────────────────────────────────────────────────────────
//
// `hyphae_gather_context` — collects relevant memories, errors, sessions, and
// code symbols within a token budget, ranked by FTS relevance.

use serde::Serialize;
use serde_json::{Value, json};
use spore::logging::workflow_span;

use hyphae_core::{
    MemoirStore, MemoryStore, SCOPED_IDENTITY_SCHEMA_VERSION, ScopedIdentity, detect_secrets,
};
use hyphae_store::SqliteStore;

use crate::protocol::ToolResult;

use super::{
    ToolTraceContext, get_bounded_i64, get_str, normalize_identity, resolve_workspace_root,
    validate_required_string, workflow_span_context,
};

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

/// Rough estimate: 4 characters per token.
const CHARS_PER_TOKEN: usize = 4;

/// Default token budget when none is specified.
const DEFAULT_TOKEN_BUDGET: i64 = 2000;

/// Max results per source category.
const MAX_PER_SOURCE: usize = 5;
const REDACTED_VALUE: &str = "[REDACTED]";
const SENSITIVE_FIELD_FRAGMENTS: &[&str] = &[
    "api_key",
    "apikey",
    "access_key",
    "auth",
    "authorization",
    "bearer",
    "credential",
    "password",
    "private_key",
    "secret",
    "token",
];

// ─────────────────────────────────────────────────────────────────────────────
// Tool entry point
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) fn tool_gather_context(
    store: &SqliteStore,
    args: &Value,
    project: Option<&str>,
    trace: &ToolTraceContext,
) -> ToolResult {
    let task = match validate_required_string(args, "task") {
        Ok(t) => t,
        Err(e) => return e,
    };

    let project_arg = args.get("project").and_then(|v| v.as_str()).or(project);
    let (project_root, worktree_id) =
        normalize_identity(get_str(args, "project_root"), get_str(args, "worktree_id"));
    let scoped_worktree = super::scoped_worktree_root(project_root, worktree_id);
    let scope = get_str(args, "scope");
    let workflow_context = workflow_span_context(trace, resolve_workspace_root(args), None);
    let _workflow_span = workflow_span("gather_context", &workflow_context).entered();

    if project_arg.is_none() && project_root.is_some() && worktree_id.is_some() {
        return ToolResult::error(
            "project is required when project_root and worktree_id are provided".to_string(),
        );
    }

    let token_budget =
        get_bounded_i64(args, "token_budget", DEFAULT_TOKEN_BUDGET, 100, 50000) as usize;

    let include = parse_include(args);

    let char_budget = token_budget * CHARS_PER_TOKEN;
    let mut results: Vec<ContextItem> = Vec::new();
    let mut sources_queried: Vec<&str> = Vec::new();

    // ── Memories ─────────────────────────────────────────────────────────
    if include.memories {
        sources_queried.push("memories");
        let memories = if let Some(worktree) = scoped_worktree {
            store.search_fts_scoped(task, MAX_PER_SOURCE, 0, project_arg, Some(worktree))
        } else {
            store.search_fts(task, MAX_PER_SOURCE, 0, project_arg)
        };
        if let Ok(memories) = memories {
            for (idx, mem) in memories.iter().enumerate() {
                results.push(ContextItem {
                    source: "memory",
                    topic: Some(mem.topic.clone()),
                    symbol: None,
                    content: mem.summary.clone(),
                    relevance: relevance_score(idx),
                });
            }
        }
    }

    // ── Errors (topic: errors/*) ─────────────────────────────────────────
    if include.errors {
        sources_queried.push("errors");
        let error_query = task;
        let all_errors = if let Some(worktree) = scoped_worktree {
            store.search_fts_scoped(
                error_query,
                MAX_PER_SOURCE * 2,
                0,
                project_arg,
                Some(worktree),
            )
        } else {
            store.search_fts(error_query, MAX_PER_SOURCE * 2, 0, project_arg)
        };
        if let Ok(all_errors) = all_errors {
            let error_mems: Vec<_> = all_errors
                .iter()
                .filter(|m| m.topic.starts_with("errors/"))
                .take(MAX_PER_SOURCE)
                .collect();
            for (idx, mem) in error_mems.iter().enumerate() {
                results.push(ContextItem {
                    source: "error",
                    topic: Some(mem.topic.clone()),
                    symbol: None,
                    content: mem.summary.clone(),
                    relevance: relevance_score(idx),
                });
            }
        }
    }

    // ── Sessions (structured rows only) ──────────────────────────────────
    if include.sessions {
        sources_queried.push("sessions");
        let structured_rows = if let Some(proj) = project_arg {
            store.session_context_identity(proj, project_root, worktree_id, scope, 10_000)
        } else {
            store.session_context_all(10_000)
        };
        if let Ok(session_rows) = structured_rows {
            let mut structured_hits: Vec<(f64, String, String)> = session_rows
                .into_iter()
                .filter_map(|session| {
                    let relevance = session_query_score(
                        session.task.as_deref(),
                        session.summary.as_deref(),
                        task,
                    )?;
                    let content =
                        session_content(session.task.as_deref(), session.summary.as_deref())?;
                    Some((relevance, session.project, content))
                })
                .collect();

            structured_hits
                .sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

            for (idx, (relevance, session_project, content)) in
                structured_hits.into_iter().take(MAX_PER_SOURCE).enumerate()
            {
                results.push(ContextItem {
                    source: "session",
                    topic: Some(format!("session/{session_project}")),
                    symbol: None,
                    content,
                    relevance: relevance.max(relevance_score(idx)),
                });
            }
        }
    }

    // ── Code (from code:{project} memoir) ────────────────────────────────
    if include.code {
        sources_queried.push("code");
        if let Some(proj) = project_arg {
            let memoir_name = format!("code:{proj}");
            if let Ok(Some(memoir)) = store.get_memoir_by_name(&memoir_name) {
                if let Ok(concepts) = store.search_concepts_fts(&memoir.id, task, MAX_PER_SOURCE) {
                    for (idx, concept) in concepts.iter().enumerate() {
                        results.push(ContextItem {
                            source: "code",
                            topic: None,
                            symbol: Some(concept.name.clone()),
                            content: concept.definition.clone(),
                            relevance: relevance_score(idx),
                        });
                    }
                }
            }
        }
    }

    // ── Sort by relevance (highest first) ────────────────────────────────
    results.sort_by(|a, b| {
        b.relevance
            .partial_cmp(&a.relevance)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // ── Truncate to fit token budget ─────────────────────────────────────
    let mut chars_used: usize = 0;
    let mut truncated: Vec<Value> = Vec::new();

    for item in &results {
        let item_chars = item.content.len();
        if chars_used + item_chars > char_budget && !truncated.is_empty() {
            break;
        }
        chars_used += item_chars;

        let mut entry = json!({
            "source": item.source,
            "content": item.content,
            "relevance": (item.relevance * 100.0).round() / 100.0,
        });

        if let Some(ref topic) = item.topic {
            entry["topic"] = json!(topic);
        }
        if let Some(ref symbol) = item.symbol {
            entry["symbol"] = json!(symbol);
        }

        truncated.push(entry);
    }

    let tokens_used = chars_used / CHARS_PER_TOKEN;

    let response = json!({
        "schema_version": SCOPED_IDENTITY_SCHEMA_VERSION,
        "scoped_identity": ScopedIdentity::new(project_arg, project_root, worktree_id, scope, None),
        "context": truncated,
        "tokens_used": tokens_used,
        "tokens_budget": token_budget,
        "sources_queried": sources_queried,
    });

    ToolResult::text(response.to_string())
}

pub(crate) fn passive_context_resource_text(
    store: &SqliteStore,
    project: Option<&str>,
) -> Result<String, String> {
    let bundle = store
        .passive_context_bundle(project)
        .map_err(|e| format!("db error: {e}"))?;
    redacted_json_text(&bundle)
}

pub(crate) fn redacted_json_text<T: Serialize>(payload: &T) -> Result<String, String> {
    let value = serde_json::to_value(payload).map_err(|e| format!("serialize error: {e}"))?;
    let redacted = redact_boundary_value(None, value);
    serde_json::to_string_pretty(&redacted).map_err(|e| format!("serialize error: {e}"))
}

pub(crate) fn redact_boundary_text(text: &str) -> String {
    if detect_secrets(text).is_empty() {
        text.to_string()
    } else {
        REDACTED_VALUE.to_string()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal types
// ─────────────────────────────────────────────────────────────────────────────

struct ContextItem {
    source: &'static str,
    topic: Option<String>,
    symbol: Option<String>,
    content: String,
    relevance: f64,
}

struct IncludeFlags {
    memories: bool,
    errors: bool,
    sessions: bool,
    code: bool,
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Parse the `include` array from arguments. Defaults to all sources.
fn parse_include(args: &Value) -> IncludeFlags {
    let arr = args.get("include").and_then(|v| v.as_array());

    match arr {
        None => IncludeFlags {
            memories: true,
            errors: true,
            sessions: true,
            code: true,
        },
        Some(items) => {
            let strings: Vec<&str> = items.iter().filter_map(|v| v.as_str()).collect();
            IncludeFlags {
                memories: strings.contains(&"memories"),
                errors: strings.contains(&"errors"),
                sessions: strings.contains(&"sessions"),
                code: strings.contains(&"code"),
            }
        }
    }
}

/// Compute a relevance score based on FTS rank position (0-indexed).
/// First result gets 0.95, decreasing by 0.1 per position, minimum 0.1.
fn relevance_score(position: usize) -> f64 {
    (0.95 - (position as f64 * 0.1)).max(0.1)
}

fn session_content(task: Option<&str>, summary: Option<&str>) -> Option<String> {
    match (task, summary) {
        (Some(task), Some(summary)) => Some(format!("{task}\n{summary}")),
        (Some(task), None) => Some(task.to_string()),
        (None, Some(summary)) => Some(summary.to_string()),
        (None, None) => None,
    }
}

fn session_query_score(task: Option<&str>, summary: Option<&str>, query: &str) -> Option<f64> {
    let haystack = format!(
        "{} {}",
        task.unwrap_or_default().to_lowercase(),
        summary.unwrap_or_default().to_lowercase()
    );
    if haystack.trim().is_empty() {
        return None;
    }

    let query_terms: Vec<&str> = query
        .split_whitespace()
        .map(str::trim)
        .filter(|term| !term.is_empty())
        .collect();
    if query_terms.is_empty() {
        return Some(0.5);
    }

    let matches = query_terms
        .iter()
        .filter(|term| haystack.contains(&term.to_lowercase()))
        .count();
    if matches == 0 {
        return None;
    }

    Some((matches as f64 / query_terms.len() as f64).clamp(0.1, 1.0))
}

fn redact_boundary_value(field_name: Option<&str>, value: Value) -> Value {
    match value {
        Value::String(text) => {
            if field_name.is_some_and(is_sensitive_field_name) || !detect_secrets(&text).is_empty()
            {
                Value::String(REDACTED_VALUE.to_string())
            } else {
                Value::String(text)
            }
        }
        Value::Array(items) => Value::Array(
            items
                .into_iter()
                .map(|item| redact_boundary_value(field_name, item))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| {
                    let redacted = if is_sensitive_field_name(&key) {
                        Value::String(REDACTED_VALUE.to_string())
                    } else {
                        redact_boundary_value(Some(&key), value)
                    };
                    (key, redacted)
                })
                .collect(),
        ),
        other => other,
    }
}

fn is_sensitive_field_name(field_name: &str) -> bool {
    let normalized = field_name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect::<String>()
        .to_ascii_lowercase();

    SENSITIVE_FIELD_FRAGMENTS
        .iter()
        .any(|fragment| normalized.contains(fragment))
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use hyphae_core::{Importance, Memory, MemoryStore};
    use serde_json::json;

    fn test_store() -> SqliteStore {
        SqliteStore::in_memory().expect("in-memory store")
    }

    #[test]
    fn test_gather_context_empty_store() {
        let store = test_store();
        let result = tool_gather_context(
            &store,
            &json!({"task": "refactor auth"}),
            None,
            &ToolTraceContext::default(),
        );
        assert!(!result.is_error, "should succeed on empty store");
        let parsed: Value = serde_json::from_str(&result.content[0].text).unwrap();
        assert_eq!(parsed["schema_version"].as_str(), Some("1.0"));
        assert_eq!(parsed["tokens_budget"], 2000);
        assert!(parsed["scoped_identity"].is_object());
        assert!(parsed["context"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_gather_context_with_memories() {
        let store = test_store();

        let mem = Memory::builder(
            "architecture".to_string(),
            "Auth middleware uses JWT with RS256".to_string(),
            Importance::High,
        )
        .keywords(vec!["auth".to_string(), "jwt".to_string()])
        .build();
        store.store(mem).unwrap();

        let result = tool_gather_context(
            &store,
            &json!({"task": "auth middleware", "include": ["memories"]}),
            None,
            &ToolTraceContext::default(),
        );
        assert!(!result.is_error);
        let parsed: Value = serde_json::from_str(&result.content[0].text).unwrap();
        let ctx = parsed["context"].as_array().unwrap();
        assert_eq!(parsed["scoped_identity"]["project"].as_str(), None);
        assert!(!ctx.is_empty(), "should find the auth memory");
        assert_eq!(ctx[0]["source"], "memory");
    }

    #[test]
    fn test_gather_context_scopes_memories_to_worktree_identity() {
        let store = test_store();

        let alpha = Memory::builder(
            "architecture".to_string(),
            "Alpha worktree gather target".to_string(),
            Importance::High,
        )
        .project("demo".to_string())
        .worktree("/repo/demo/wt-alpha".to_string())
        .build();
        let beta = Memory::builder(
            "architecture".to_string(),
            "Beta worktree gather target".to_string(),
            Importance::High,
        )
        .project("demo".to_string())
        .worktree("/repo/demo/wt-beta".to_string())
        .build();
        store.store(alpha).unwrap();
        store.store(beta).unwrap();

        let result = tool_gather_context(
            &store,
            &json!({
                "task": "gather target",
                "project": "demo",
                "project_root": "/repo/demo/wt-alpha",
                "worktree_id": "wt-alpha",
                "include": ["memories"]
            }),
            None,
            &ToolTraceContext::default(),
        );
        assert!(!result.is_error);
        let parsed: Value = serde_json::from_str(&result.content[0].text).unwrap();
        let ctx = parsed["context"].as_array().unwrap();
        assert_eq!(parsed["scoped_identity"]["project"].as_str(), Some("demo"));
        assert_eq!(
            parsed["scoped_identity"]["project_root"].as_str(),
            Some("/repo/demo/wt-alpha")
        );
        assert_eq!(
            parsed["scoped_identity"]["worktree_id"].as_str(),
            Some("wt-alpha")
        );
        assert_eq!(ctx.len(), 1);
        let content = ctx[0]["content"].as_str().unwrap();
        assert!(content.contains("Alpha worktree gather target"));
        assert!(!content.contains("Beta worktree gather target"));
    }

    #[test]
    fn test_gather_context_respects_token_budget() {
        let store = test_store();

        // Store two memories with searchable content
        let large_content = format!("authentication {}", "details ".repeat(250));
        let mem1 = Memory::builder(
            "architecture".to_string(),
            large_content,
            Importance::Medium,
        )
        .build();
        store.store(mem1).unwrap();

        let mem2 = Memory::builder(
            "architecture".to_string(),
            "authentication uses JWT tokens for all API endpoints".to_string(),
            Importance::Medium,
        )
        .build();
        store.store(mem2).unwrap();

        let result = tool_gather_context(
            &store,
            &json!({"task": "authentication", "token_budget": 50, "include": ["memories"]}),
            None,
            &ToolTraceContext::default(),
        );
        assert!(!result.is_error);
        let parsed: Value = serde_json::from_str(&result.content[0].text).unwrap();
        // First item is always included; second may be truncated by budget
        let ctx = parsed["context"].as_array().unwrap();
        assert!(!ctx.is_empty());
        // Verify tokens_used is reported
        assert!(parsed["tokens_used"].as_u64().is_some());
    }

    #[test]
    fn test_gather_context_missing_task() {
        let store = test_store();
        let result = tool_gather_context(&store, &json!({}), None, &ToolTraceContext::default());
        assert!(result.is_error);
    }

    #[test]
    fn test_gather_context_with_project() {
        let store = test_store();
        let (session_id, _) = store.session_start("myapp", Some("login flow")).unwrap();
        store
            .session_end(&session_id, Some("Implemented login flow"), None, Some("0"))
            .unwrap();

        let result = tool_gather_context(
            &store,
            &json!({
                "task": "login",
                "project": "myapp",
                "include": ["sessions"]
            }),
            None,
            &ToolTraceContext::default(),
        );
        assert!(!result.is_error);
        let parsed: Value = serde_json::from_str(&result.content[0].text).unwrap();
        assert!(
            parsed["sources_queried"]
                .as_array()
                .unwrap()
                .contains(&json!("sessions"))
        );
        assert_eq!(parsed["context"][0]["source"], "session");
    }

    #[test]
    fn test_gather_context_with_project_ignores_legacy_session_memories() {
        let store = test_store();

        let mem = Memory::builder(
            "session/myapp".to_string(),
            "Implemented login flow".to_string(),
            Importance::Medium,
        )
        .project("myapp".to_string())
        .build();
        store.store(mem).unwrap();

        let result = tool_gather_context(
            &store,
            &json!({
                "task": "login",
                "project": "myapp",
                "include": ["sessions"]
            }),
            None,
            &ToolTraceContext::default(),
        );

        assert!(!result.is_error);
        let parsed: Value = serde_json::from_str(&result.content[0].text).unwrap();
        assert!(parsed["context"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_gather_context_without_project_uses_structured_sessions() {
        let store = test_store();
        let (session_id, _) = store
            .session_start("shared-app", Some("login flow"))
            .unwrap();
        store
            .session_end(&session_id, Some("Implemented login flow"), None, Some("0"))
            .unwrap();

        let result = tool_gather_context(
            &store,
            &json!({
                "task": "login",
                "include": ["sessions"]
            }),
            None,
            &ToolTraceContext::default(),
        );

        assert!(!result.is_error);
        let parsed: Value = serde_json::from_str(&result.content[0].text).unwrap();
        assert_eq!(parsed["context"][0]["source"], "session");
        assert_eq!(parsed["context"][0]["topic"], "session/shared-app");
    }

    #[test]
    fn test_gather_context_requires_project_with_full_identity() {
        let store = test_store();

        let result = tool_gather_context(
            &store,
            &json!({
                "task": "login",
                "project_root": "/repo/demo",
                "worktree_id": "wt-alpha",
                "include": ["sessions"]
            }),
            None,
            &ToolTraceContext::default(),
        );

        assert!(result.is_error);
        assert!(
            result.content[0]
                .text
                .contains("project is required when project_root and worktree_id are provided")
        );
    }

    #[test]
    fn test_gather_context_filters_sessions_by_identity_v1() {
        let store = test_store();

        let (alpha_id, _) = store
            .session_start_identity(
                "demo",
                Some("login flow"),
                Some("/repo/demo"),
                Some("wt-alpha"),
                None,
            )
            .unwrap();
        store
            .session_end(
                &alpha_id,
                Some("Alpha login implementation"),
                None,
                Some("0"),
            )
            .unwrap();

        let (beta_id, _) = store
            .session_start_identity(
                "demo",
                Some("login flow"),
                Some("/repo/demo"),
                Some("wt-beta"),
                None,
            )
            .unwrap();
        store
            .session_end(&beta_id, Some("Beta login implementation"), None, Some("0"))
            .unwrap();

        let result = tool_gather_context(
            &store,
            &json!({
                "task": "login",
                "project": "demo",
                "project_root": "/repo/demo",
                "worktree_id": "wt-alpha",
                "include": ["sessions"]
            }),
            None,
            &ToolTraceContext::default(),
        );

        assert!(!result.is_error);
        let parsed: Value = serde_json::from_str(&result.content[0].text).unwrap();
        let context = parsed["context"].as_array().unwrap();
        assert_eq!(context.len(), 1);
        assert_eq!(context[0]["topic"], "session/demo");
        assert!(
            context[0]["content"]
                .as_str()
                .unwrap()
                .contains("Alpha login implementation")
        );
    }

    #[test]
    fn test_gather_context_filters_parallel_workers_by_scope() {
        let store = test_store();

        let (worker_a_id, _) = store
            .session_start_identity(
                "demo",
                Some("login flow"),
                Some("/repo/demo"),
                Some("wt-alpha"),
                Some("worker-a"),
            )
            .unwrap();
        store
            .session_end(
                &worker_a_id,
                Some("Worker A login implementation"),
                None,
                Some("0"),
            )
            .unwrap();

        let (worker_b_id, _) = store
            .session_start_identity(
                "demo",
                Some("login flow"),
                Some("/repo/demo"),
                Some("wt-alpha"),
                Some("worker-b"),
            )
            .unwrap();
        store
            .session_end(
                &worker_b_id,
                Some("Worker B login implementation"),
                None,
                Some("0"),
            )
            .unwrap();

        let result = tool_gather_context(
            &store,
            &json!({
                "task": "login",
                "project": "demo",
                "project_root": "/repo/demo",
                "worktree_id": "wt-alpha",
                "scope": "worker-a",
                "include": ["sessions"]
            }),
            None,
            &ToolTraceContext::default(),
        );

        assert!(!result.is_error);
        let parsed: Value = serde_json::from_str(&result.content[0].text).unwrap();
        let context = parsed["context"].as_array().unwrap();
        assert_eq!(context.len(), 1);
        assert!(
            context[0]["content"]
                .as_str()
                .unwrap()
                .contains("Worker A login implementation")
        );
        assert!(
            !context[0]["content"]
                .as_str()
                .unwrap()
                .contains("Worker B login implementation")
        );
    }

    #[test]
    fn test_gather_context_partial_identity_uses_project_sessions() {
        let store = test_store();

        let (alpha_id, _) = store
            .session_start_identity(
                "demo",
                Some("login flow"),
                Some("/repo/demo"),
                Some("wt-alpha"),
                None,
            )
            .unwrap();
        store
            .session_end(
                &alpha_id,
                Some("Alpha login implementation"),
                None,
                Some("0"),
            )
            .unwrap();

        let (beta_id, _) = store
            .session_start_identity(
                "demo",
                Some("login flow"),
                Some("/repo/demo"),
                Some("wt-beta"),
                None,
            )
            .unwrap();
        store
            .session_end(&beta_id, Some("Beta login implementation"), None, Some("0"))
            .unwrap();

        let result = tool_gather_context(
            &store,
            &json!({
                "task": "login",
                "project": "demo",
                "project_root": "/repo/demo",
                "include": ["sessions"]
            }),
            None,
            &ToolTraceContext::default(),
        );

        assert!(!result.is_error);
        let parsed: Value = serde_json::from_str(&result.content[0].text).unwrap();
        let context = parsed["context"].as_array().unwrap();
        assert_eq!(context.len(), 2);

        let contents: Vec<&str> = context
            .iter()
            .map(|entry| entry["content"].as_str().unwrap())
            .collect();
        assert!(
            contents
                .iter()
                .any(|content| content.contains("Alpha login implementation"))
        );
        assert!(
            contents
                .iter()
                .any(|content| content.contains("Beta login implementation"))
        );
    }

    #[test]
    fn test_gather_context_identity_miss_does_not_fall_back_to_legacy_session_memories() {
        let store = test_store();

        let legacy = Memory::builder(
            "session/demo".to_string(),
            "Legacy beta worktree summary".to_string(),
            Importance::Medium,
        )
        .project("demo".to_string())
        .build();
        store.store(legacy).unwrap();

        let result = tool_gather_context(
            &store,
            &json!({
                "task": "beta",
                "project": "demo",
                "project_root": "/repo/demo",
                "worktree_id": "wt-alpha",
                "include": ["sessions"]
            }),
            None,
            &ToolTraceContext::default(),
        );

        assert!(!result.is_error);
        let parsed: Value = serde_json::from_str(&result.content[0].text).unwrap();
        assert!(parsed["context"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_parse_include_defaults() {
        let flags = parse_include(&json!({}));
        assert!(flags.memories);
        assert!(flags.errors);
        assert!(flags.sessions);
        assert!(flags.code);
    }

    #[test]
    fn test_parse_include_selective() {
        let flags = parse_include(&json!({"include": ["memories", "code"]}));
        assert!(flags.memories);
        assert!(!flags.errors);
        assert!(!flags.sessions);
        assert!(flags.code);
    }

    #[test]
    fn test_relevance_score() {
        assert!((relevance_score(0) - 0.95).abs() < 0.001);
        assert!((relevance_score(1) - 0.85).abs() < 0.001);
        assert!((relevance_score(10) - 0.1).abs() < 0.001);
    }

    #[test]
    fn test_redacted_json_text_masks_secret_strings() {
        let payload = json!({
            "summary": "normal note",
            "raw_excerpt": "api_key: sk1234567890abcdefghij",
            "nested": {
                "token": "Bearer super-secret-token-value"
            }
        });

        let text = redacted_json_text(&payload).unwrap();
        assert!(text.contains("[REDACTED]"));
        assert!(!text.contains("sk1234567890abcdefghij"));
        assert!(!text.contains("super-secret-token-value"));
    }

    #[test]
    fn test_redact_boundary_text_redacts_secrets() {
        let redacted = redact_boundary_text("api_key: sk1234567890abcdefghij");
        assert_eq!(redacted, REDACTED_VALUE);
    }

    #[test]
    fn test_redact_boundary_text_keeps_safe_text() {
        let redacted = redact_boundary_text("Implemented compact passive context resource");
        assert_eq!(redacted, "Implemented compact passive context resource");
    }
}
