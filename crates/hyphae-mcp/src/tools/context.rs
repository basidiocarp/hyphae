// ─────────────────────────────────────────────────────────────────────────────
// Context-gathering MCP tool
// ─────────────────────────────────────────────────────────────────────────────
//
// `hyphae_gather_context` — collects relevant memories, errors, sessions, and
// code symbols within a token budget, ranked by FTS relevance.

use serde_json::{Value, json};

use hyphae_core::{MemoirStore, MemoryStore};
use hyphae_store::SqliteStore;

use crate::protocol::ToolResult;

use super::{get_bounded_i64, validate_required_string};

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

/// Rough estimate: 4 characters per token.
const CHARS_PER_TOKEN: usize = 4;

/// Default token budget when none is specified.
const DEFAULT_TOKEN_BUDGET: i64 = 2000;

/// Max results per source category.
const MAX_PER_SOURCE: usize = 5;

// ─────────────────────────────────────────────────────────────────────────────
// Tool entry point
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) fn tool_gather_context(
    store: &SqliteStore,
    args: &Value,
    project: Option<&str>,
) -> ToolResult {
    let task = match validate_required_string(args, "task") {
        Ok(t) => t,
        Err(e) => return e,
    };

    let project_arg = args.get("project").and_then(|v| v.as_str()).or(project);

    let token_budget =
        get_bounded_i64(args, "token_budget", DEFAULT_TOKEN_BUDGET, 100, 50000) as usize;

    let include = parse_include(args);

    let char_budget = token_budget * CHARS_PER_TOKEN;
    let mut results: Vec<ContextItem> = Vec::new();
    let mut sources_queried: Vec<&str> = Vec::new();

    // ── Memories ─────────────────────────────────────────────────────────
    if include.memories {
        sources_queried.push("memories");
        if let Ok(memories) = store.search_fts(task, MAX_PER_SOURCE, 0, project_arg) {
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

    // ── Errors (topic: errors/resolved) ──────────────────────────────────
    if include.errors {
        sources_queried.push("errors");
        // Search within the errors/resolved topic
        let error_query = task;
        if let Ok(all_errors) = store.search_fts(error_query, MAX_PER_SOURCE * 2, 0, project_arg) {
            let error_mems: Vec<_> = all_errors
                .iter()
                .filter(|m| m.topic.starts_with("errors") || m.topic.starts_with("resolved"))
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

    // ── Sessions (topic: session/*) ──────────────────────────────────────
    if include.sessions {
        sources_queried.push("sessions");
        if let Ok(session_hits) = store.search_fts(task, MAX_PER_SOURCE * 2, 0, project_arg) {
            let session_mems: Vec<_> = session_hits
                .iter()
                .filter(|m| m.topic.starts_with("session/"))
                .take(MAX_PER_SOURCE)
                .collect();
            for (idx, mem) in session_mems.iter().enumerate() {
                results.push(ContextItem {
                    source: "session",
                    topic: Some(mem.topic.clone()),
                    symbol: None,
                    content: mem.summary.clone(),
                    relevance: relevance_score(idx),
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
        "context": truncated,
        "tokens_used": tokens_used,
        "tokens_budget": token_budget,
        "sources_queried": sources_queried,
    });

    ToolResult::text(response.to_string())
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
        let result = tool_gather_context(&store, &json!({"task": "refactor auth"}), None);
        assert!(!result.is_error, "should succeed on empty store");
        let parsed: Value = serde_json::from_str(&result.content[0].text).unwrap();
        assert_eq!(parsed["tokens_budget"], 2000);
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
        );
        assert!(!result.is_error);
        let parsed: Value = serde_json::from_str(&result.content[0].text).unwrap();
        let ctx = parsed["context"].as_array().unwrap();
        assert!(!ctx.is_empty(), "should find the auth memory");
        assert_eq!(ctx[0]["source"], "memory");
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
        );
        assert!(!result.is_error);
        let parsed: Value = serde_json::from_str(&result.content[0].text).unwrap();
        // First item is always included; second may be truncated by budget
        let ctx = parsed["context"].as_array().unwrap();
        assert!(ctx.len() >= 1);
        // Verify tokens_used is reported
        assert!(parsed["tokens_used"].as_u64().is_some());
    }

    #[test]
    fn test_gather_context_missing_task() {
        let store = test_store();
        let result = tool_gather_context(&store, &json!({}), None);
        assert!(result.is_error);
    }

    #[test]
    fn test_gather_context_with_project() {
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
        );
        assert!(!result.is_error);
        let parsed: Value = serde_json::from_str(&result.content[0].text).unwrap();
        assert!(
            parsed["sources_queried"]
                .as_array()
                .unwrap()
                .contains(&json!("sessions"))
        );
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
}
