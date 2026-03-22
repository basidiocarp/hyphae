use serde_json::{Value, json};

use hyphae_core::{Embedder, Memoir, MemoirStore};
use hyphae_store::SqliteStore;

use crate::protocol::ToolResult;

mod context;
mod ingest;
mod memoir;
mod memory;
mod schema;
mod session;

// ===========================================================================
// Tool schemas for tools/list
// ===========================================================================

pub fn tool_definitions(has_embedder: bool) -> Value {
    // ─────────────────────────────────────────────────────────────────────
    // Compute tool definitions fresh each call, keyed by has_embedder
    // (Schema generation is cheap; caching with wrong has_embedder value
    // causes vector search tools to be missing/present incorrectly)
    // ─────────────────────────────────────────────────────────────────────
    let tools = schema::tool_definitions_json(has_embedder);
    json!({ "tools": tools })
}

// ===========================================================================
// Tool dispatch
// ===========================================================================

pub fn call_tool(
    store: &SqliteStore,
    embedder: Option<&dyn Embedder>,
    name: &str,
    args: &Value,
    compact: bool,
    project: Option<&str>,
    reject_secrets: bool,
) -> ToolResult {
    match name {
        // Memory tools
        "hyphae_memory_store" => {
            memory::tool_store(store, embedder, args, compact, project, reject_secrets)
        }
        "hyphae_memory_recall" => memory::tool_recall(store, embedder, args, compact, project),
        "hyphae_memory_forget" => memory::tool_forget(store, args),
        "hyphae_memory_update" => memory::tool_update(store, embedder, args),
        "hyphae_memory_consolidate" => memory::tool_consolidate(store, args),
        "hyphae_memory_list_topics" => memory::tool_list_topics(store, project),
        "hyphae_memory_stats" => memory::tool_stats(store, project),
        "hyphae_memory_health" => memory::tool_health(store, args, project),
        "hyphae_memory_embed_all" => memory::tool_embed_all(store, embedder, args, project),
        "hyphae_extract_lessons" => memory::tool_extract_lessons(store, args, project),
        "hyphae_evaluate" => memory::tool_evaluate(store, args, project),
        // Cross-project tools
        "hyphae_recall_global" => memory::tool_recall_global(store, args, compact),
        "hyphae_promote_to_memoir" => memory::tool_promote_to_memoir(store, args, project),
        // Memoir tools
        "hyphae_memoir_create" => memoir::tool_memoir_create(store, args),
        "hyphae_memoir_list" => memoir::tool_memoir_list(store),
        "hyphae_memoir_show" => memoir::tool_memoir_show(store, args),
        "hyphae_memoir_add_concept" => memoir::tool_memoir_add_concept(store, args),
        "hyphae_memoir_refine" => memoir::tool_memoir_refine(store, args),
        "hyphae_memoir_search" => memoir::tool_memoir_search(store, args),
        "hyphae_memoir_search_all" => memoir::tool_memoir_search_all(store, args),
        "hyphae_memoir_link" => memoir::tool_memoir_link(store, args),
        "hyphae_memoir_inspect" => memoir::tool_memoir_inspect(store, args),
        "hyphae_import_code_graph" => memoir::tool_import_code_graph(store, args, compact, project),
        "hyphae_code_query" => memoir::tool_code_query(store, args, compact, project),
        // RAG tools
        "hyphae_ingest_file" => ingest::tool_ingest_file(store, embedder, args, compact, project),
        "hyphae_search_docs" => ingest::tool_search_docs(store, embedder, args, compact, project),
        "hyphae_list_sources" => ingest::tool_list_sources(store, project),
        "hyphae_forget_source" => ingest::tool_forget_source(store, args, project),
        "hyphae_search_all" => ingest::tool_search_all(store, embedder, args, compact, project),
        // Command output tools
        "hyphae_store_command_output" => {
            ingest::tool_store_command_output(store, args, compact, project)
        }
        "hyphae_get_command_chunks" => ingest::tool_get_command_chunks(store, args),
        // Context gathering
        "hyphae_gather_context" => context::tool_gather_context(store, args, project),
        // Session lifecycle tools
        "hyphae_session_start" => session::tool_session_start(store, args),
        "hyphae_session_end" => session::tool_session_end(store, args),
        "hyphae_session_context" => session::tool_session_context(store, args),
        // Onboarding
        "hyphae_onboard" => tool_onboard(store, project),
        _ => ToolResult::error(format!("unknown tool: {name}")),
    }
}

// ===========================================================================
// Helpers (pub(super) so memory.rs and memoir.rs can use them)
// ===========================================================================

pub(super) fn get_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str())
}

pub(super) fn get_bounded_i64(args: &Value, key: &str, default: i64, min: i64, max: i64) -> i64 {
    args.get(key)
        .and_then(|v| v.as_i64())
        .unwrap_or(default)
        .clamp(min, max)
}

pub(super) fn validate_required_string<'a>(
    args: &'a Value,
    key: &str,
) -> Result<&'a str, ToolResult> {
    match get_str(args, key) {
        None => Err(ToolResult::error(format!("missing required field: {key}"))),
        Some(s) if s.trim().is_empty() => {
            Err(ToolResult::error(format!("field must not be empty: {key}")))
        }
        Some(s) => Ok(s),
    }
}

pub(super) fn validate_max_length(
    value: &str,
    field_name: &str,
    max_len: usize,
) -> Result<(), ToolResult> {
    if value.len() > max_len {
        Err(ToolResult::error(format!(
            "field '{field_name}' exceeds maximum length of {max_len} bytes"
        )))
    } else {
        Ok(())
    }
}

pub(super) fn resolve_memoir(store: &SqliteStore, name: &str) -> Result<Memoir, ToolResult> {
    store
        .get_memoir_by_name(name)
        .map_err(|e| ToolResult::error(format!("db error: {e}")))?
        .ok_or_else(|| ToolResult::error(format!("memoir not found: {name}")))
}

// ===========================================================================
// Onboarding tool
// ===========================================================================

use hyphae_core::MemoryStore;

fn tool_onboard(store: &SqliteStore, project: Option<&str>) -> ToolResult {
    let stats = match store.stats(project) {
        Ok(s) => s,
        Err(e) => return ToolResult::error(format!("failed to get stats: {e}")),
    };

    let memoirs = store.list_memoirs().unwrap_or_default();
    let topics: Vec<String> = match store.list_topics(project) {
        Ok(t) => t.into_iter().map(|(name, _count)| name).collect(),
        Err(_) => Vec::new(),
    };

    let tools_available = vec![
        "hyphae_memory_store",
        "hyphae_memory_recall",
        "hyphae_memory_forget",
        "hyphae_memory_update",
        "hyphae_memory_consolidate",
        "hyphae_memory_list_topics",
        "hyphae_memory_stats",
        "hyphae_memory_health",
        "hyphae_memoir_create",
        "hyphae_memoir_list",
        "hyphae_memoir_show",
        "hyphae_memoir_add_concept",
        "hyphae_memoir_refine",
        "hyphae_memoir_search",
        "hyphae_memoir_search_all",
        "hyphae_memoir_link",
        "hyphae_memoir_inspect",
        "hyphae_import_code_graph",
        "hyphae_code_query",
        "hyphae_ingest_file",
        "hyphae_search_docs",
        "hyphae_list_sources",
        "hyphae_forget_source",
        "hyphae_search_all",
        "hyphae_store_command_output",
        "hyphae_get_command_chunks",
        "hyphae_gather_context",
        "hyphae_session_start",
        "hyphae_session_end",
        "hyphae_session_context",
        "hyphae_onboard",
    ];

    let quick_start = if stats.total_memories == 0 {
        "No memories yet. Start by storing important project context with hyphae_memory_store, \
         then use hyphae_memory_recall to search later. Use hyphae_import_code_graph to index \
         your codebase for semantic code queries."
    } else {
        "Your memory system is active. Use hyphae_memory_recall to search past context, \
         hyphae_memory_health for maintenance, and hyphae_memoir_search for knowledge graphs."
    };

    let result = json!({
        "total_memories": stats.total_memories,
        "total_memoirs": memoirs.len(),
        "topics": topics,
        "tools_available": tools_available,
        "quick_start": quick_start,
    });

    ToolResult::text(result.to_string())
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> SqliteStore {
        SqliteStore::in_memory().unwrap()
    }

    #[test]
    fn test_unknown_tool_returns_error() {
        let store = test_store();
        let result = call_tool(
            &store,
            None,
            "nonexistent_tool",
            &json!({}),
            false,
            None,
            false,
        );
        assert!(result.is_error);
        assert!(result.content[0].text.contains("unknown tool"));
    }

    #[test]
    fn test_store_missing_topic() {
        let store = test_store();
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_store",
            &json!({"content": "hello"}),
            false,
            None,
            false,
        );
        assert!(result.is_error);
        assert!(result.content[0].text.contains("topic"));
    }

    #[test]
    fn test_store_missing_content() {
        let store = test_store();
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_store",
            &json!({"topic": "test"}),
            false,
            None,
            false,
        );
        assert!(result.is_error);
        assert!(result.content[0].text.contains("content"));
    }

    #[test]
    fn test_recall_missing_query() {
        let store = test_store();
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_recall",
            &json!({}),
            false,
            None,
            false,
        );
        assert!(result.is_error);
        assert!(result.content[0].text.contains("query"));
    }

    #[test]
    fn test_recall_empty_store() {
        let store = test_store();
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_recall",
            &json!({"query": "anything"}),
            false,
            None,
            false,
        );
        assert!(!result.is_error);
        assert!(result.content[0].text.contains("No memories"));
    }

    #[test]
    fn test_forget_missing_id() {
        let store = test_store();
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_forget",
            &json!({}),
            false,
            None,
            false,
        );
        assert!(result.is_error);
        assert!(result.content[0].text.contains("id"));
    }

    #[test]
    fn test_forget_nonexistent_id() {
        let store = test_store();
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_forget",
            &json!({"id": "does-not-exist"}),
            false,
            None,
            false,
        );
        assert!(result.is_error);
    }

    #[test]
    fn test_store_and_recall_roundtrip() {
        let store = test_store();
        let store_result = call_tool(
            &store,
            None,
            "hyphae_memory_store",
            &json!({"topic": "test-project", "content": "Uses Rust and SQLite"}),
            false,
            None,
            false,
        );
        assert!(!store_result.is_error);
        assert!(store_result.content[0].text.contains("Stored memory"));

        let recall_result = call_tool(
            &store,
            None,
            "hyphae_memory_recall",
            &json!({"query": "Rust SQLite"}),
            false,
            None,
            false,
        );
        assert!(!recall_result.is_error);
        assert!(recall_result.content[0].text.contains("Rust"));
    }

    #[test]
    fn test_compact_store_output() {
        let store = test_store();
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_store",
            &json!({"topic": "t", "content": "c"}),
            true,
            None,
            false,
        );
        assert!(!result.is_error);
        assert!(result.content[0].text.starts_with("ok:"));
    }

    #[test]
    fn test_compact_recall_output() {
        let store = test_store();
        call_tool(
            &store,
            None,
            "hyphae_memory_store",
            &json!({"topic": "proj", "content": "Rust memory system"}),
            false,
            None,
            false,
        );
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_recall",
            &json!({"query": "Rust memory"}),
            true,
            None,
            false,
        );
        assert!(!result.is_error);
        assert!(result.content[0].text.contains("[proj]"));
    }

    #[test]
    fn test_stats_empty() {
        let store = test_store();
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_stats",
            &json!({}),
            false,
            None,
            false,
        );
        assert!(!result.is_error);
        assert!(result.content[0].text.contains("Memories: 0"));
    }

    #[test]
    fn test_list_topics_empty() {
        let store = test_store();
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_list_topics",
            &json!({}),
            false,
            None,
            false,
        );
        assert!(!result.is_error);
        assert!(result.content[0].text.contains("No topics"));
    }

    #[test]
    fn test_health_empty() {
        let store = test_store();
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_health",
            &json!({}),
            false,
            None,
            false,
        );
        assert!(!result.is_error);
        assert!(result.content[0].text.contains("No topics"));
    }

    #[test]
    fn test_update_missing_fields() {
        let store = test_store();
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_update",
            &json!({"id": "x"}),
            false,
            None,
            false,
        );
        assert!(result.is_error);
        assert!(result.content[0].text.contains("content"));
    }

    #[test]
    fn test_update_nonexistent() {
        let store = test_store();
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_update",
            &json!({"id": "fake", "content": "new"}),
            false,
            None,
            false,
        );
        assert!(result.is_error);
        assert!(result.content[0].text.contains("not found"));
    }

    #[test]
    fn test_store_sql_injection_topic() {
        let store = test_store();
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_store",
            &json!({"topic": "'; DROP TABLE memories;--", "content": "pwned"}),
            false,
            None,
            false,
        );
        assert!(!result.is_error);
        let stats = call_tool(
            &store,
            None,
            "hyphae_memory_stats",
            &json!({}),
            false,
            None,
            false,
        );
        assert!(stats.content[0].text.contains("Memories: 1"));
    }

    #[test]
    fn test_recall_injection_query() {
        let store = test_store();
        call_tool(
            &store,
            None,
            "hyphae_memory_store",
            &json!({"topic": "safe", "content": "normal data"}),
            false,
            None,
            false,
        );
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_recall",
            &json!({"query": "') OR 1=1 --"}),
            false,
            None,
            false,
        );
        assert!(!result.is_error);
    }

    #[test]
    fn test_store_xss_in_content() {
        let store = test_store();
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_store",
            &json!({
                "topic": "xss",
                "content": "<script>alert('xss')</script>"
            }),
            false,
            None,
            false,
        );
        assert!(!result.is_error);
        let recall = call_tool(
            &store,
            None,
            "hyphae_memory_recall",
            &json!({"query": "script alert"}),
            false,
            None,
            false,
        );
        assert!(recall.content[0].text.contains("<script>"));
    }

    #[test]
    fn test_store_very_large_content() {
        let store = test_store();
        // Content within the 32KB limit should succeed
        let within_limit = "x".repeat(32768);
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_store",
            &json!({"topic": "big", "content": within_limit}),
            false,
            None,
            false,
        );
        assert!(!result.is_error);
        // Content exceeding 32KB should be rejected
        let over_limit = "x".repeat(32769);
        let result2 = call_tool(
            &store,
            None,
            "hyphae_memory_store",
            &json!({"topic": "big", "content": over_limit}),
            false,
            None,
            false,
        );
        assert!(result2.is_error);
        assert!(result2.content[0].text.contains("content"));
    }

    #[test]
    fn test_memoir_create_injection() {
        let store = test_store();
        let result = call_tool(
            &store,
            None,
            "hyphae_memoir_create",
            &json!({"name": "'; DROP TABLE memoirs;--", "description": "test"}),
            false,
            None,
            false,
        );
        assert!(!result.is_error);
        let list = call_tool(
            &store,
            None,
            "hyphae_memoir_list",
            &json!({}),
            false,
            None,
            false,
        );
        assert!(!list.is_error);
        assert!(list.content[0].text.contains("DROP TABLE"));
    }

    #[test]
    fn test_store_many_via_mcp() {
        let store = test_store();
        for i in 0..50 {
            let result = call_tool(
                &store,
                None,
                "hyphae_memory_store",
                &json!({"topic": "perf", "content": format!("item {i}")}),
                true,
                None,
                false,
            );
            assert!(!result.is_error);
        }
        let stats = call_tool(
            &store,
            None,
            "hyphae_memory_stats",
            &json!({}),
            false,
            None,
            false,
        );
        assert!(stats.content[0].text.contains("Memories: 50"));
    }

    #[test]
    fn test_recall_with_topic_filter() {
        let store = test_store();
        for topic in &["alpha", "beta", "gamma"] {
            call_tool(
                &store,
                None,
                "hyphae_memory_store",
                &json!({"topic": topic, "content": format!("data for {topic}")}),
                false,
                None,
                false,
            );
        }
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_recall",
            &json!({"query": "data", "topic": "beta"}),
            false,
            None,
            false,
        );
        assert!(!result.is_error);
        assert!(result.content[0].text.contains("beta"));
        assert!(!result.content[0].text.contains("alpha"));
    }

    #[test]
    fn test_consolidate_via_mcp() {
        let store = test_store();
        for i in 0..10 {
            call_tool(
                &store,
                None,
                "hyphae_memory_store",
                &json!({"topic": "consolidate-me", "content": format!("detail {i}")}),
                false,
                None,
                false,
            );
        }
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_consolidate",
            &json!({"topic": "consolidate-me", "summary": "All 10 details merged"}),
            false,
            None,
            false,
        );
        assert!(!result.is_error);
        let stats = call_tool(
            &store,
            None,
            "hyphae_memory_stats",
            &json!({}),
            false,
            None,
            false,
        );
        assert!(stats.content[0].text.contains("Memories: 1"));
    }

    // === Security tests ===

    #[test]
    fn test_path_traversal_in_topic() {
        let store = test_store();
        let malicious_topics = [
            "../../../etc/passwd",
            "..\\..\\windows\\system32",
            "/etc/shadow",
            "topic/../../secret",
            "....//....//etc/passwd",
        ];
        for topic in &malicious_topics {
            let result = call_tool(
                &store,
                None,
                "hyphae_memory_store",
                &json!({"topic": topic, "content": "path traversal attempt"}),
                false,
                None,
                false,
            );
            // Should either store safely (topic is just a string label) or reject
            // but must NOT crash or access filesystem
            assert!(!result.content.is_empty());
        }
        let stats = call_tool(
            &store,
            None,
            "hyphae_memory_stats",
            &json!({}),
            false,
            None,
            false,
        );
        assert!(!stats.is_error);
    }

    #[test]
    fn test_extremely_long_content_over_1mb() {
        let store = test_store();
        let huge_content = "A".repeat(1_100_000); // ~1.1MB
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_store",
            &json!({"topic": "huge", "content": huge_content}),
            false,
            None,
            false,
        );
        // Should either store or reject gracefully, never panic
        assert!(!result.content.is_empty());
    }

    #[test]
    fn test_null_bytes_in_topic() {
        let store = test_store();
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_store",
            &json!({"topic": "before\0after", "content": "null byte topic"}),
            false,
            None,
            false,
        );
        assert!(!result.content.is_empty());
    }

    #[test]
    fn test_null_bytes_in_content() {
        let store = test_store();
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_store",
            &json!({"topic": "test", "content": "start\0middle\0end"}),
            false,
            None,
            false,
        );
        assert!(!result.content.is_empty());
    }

    #[test]
    fn test_null_bytes_in_query() {
        let store = test_store();
        call_tool(
            &store,
            None,
            "hyphae_memory_store",
            &json!({"topic": "safe", "content": "normal data"}),
            false,
            None,
            false,
        );
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_recall",
            &json!({"query": "normal\0injected"}),
            false,
            None,
            false,
        );
        assert!(!result.content.is_empty());
    }

    #[test]
    fn test_unicode_rtl_and_zero_width_chars() {
        let store = test_store();
        // Right-to-left override, zero-width joiners, bidi markers
        let tricky_strings = [
            "\u{202E}reversed\u{202C}",                   // RTL override
            "normal\u{200B}zero\u{200B}width",            // zero-width space
            "\u{FEFF}bom_prefix",                         // BOM
            "a\u{0300}\u{0301}\u{0302}\u{0303}combining", // stacked combining marks
            "\u{200D}\u{200D}\u{200D}",                   // zero-width joiners only
        ];
        for s in &tricky_strings {
            let result = call_tool(
                &store,
                None,
                "hyphae_memory_store",
                &json!({"topic": s, "content": format!("content with {s}")}),
                false,
                None,
                false,
            );
            assert!(!result.is_error, "Failed on unicode string: {:?}", s);
        }
        let stats = call_tool(
            &store,
            None,
            "hyphae_memory_stats",
            &json!({}),
            false,
            None,
            false,
        );
        assert!(!stats.is_error);
    }

    #[test]
    fn test_json_injection_in_params() {
        let store = test_store();
        // Attempt to inject extra JSON fields
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_store",
            &json!({
                "topic": "test",
                "content": "legit",
                "__proto__": {"admin": true},
                "constructor": {"prototype": {"isAdmin": true}},
                "extra_unknown_field": "should be ignored"
            }),
            false,
            None,
            false,
        );
        // Should store normally, ignoring unknown fields
        assert!(!result.is_error);
        let recall = call_tool(
            &store,
            None,
            "hyphae_memory_recall",
            &json!({"query": "legit"}),
            false,
            None,
            false,
        );
        assert!(!recall.is_error);
        assert!(recall.content[0].text.contains("legit"));
    }

    #[test]
    fn test_empty_topic_field() {
        let store = test_store();
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_store",
            &json!({"topic": "", "content": "empty topic"}),
            false,
            None,
            false,
        );
        // Should either reject or store; must not panic
        assert!(!result.content.is_empty());
    }

    #[test]
    fn test_whitespace_only_fields() {
        let store = test_store();
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_store",
            &json!({"topic": "   \t\n  ", "content": "   \n\t  "}),
            false,
            None,
            false,
        );
        // Should either reject or store; must not panic
        assert!(!result.content.is_empty());
    }

    #[test]
    fn test_whitespace_only_recall_query() {
        let store = test_store();
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_recall",
            &json!({"query": "   \t\n  "}),
            false,
            None,
            false,
        );
        // Should return empty or error, not crash
        assert!(!result.content.is_empty());
    }

    #[test]
    fn test_memoir_create_path_traversal_name() {
        let store = test_store();
        let result = call_tool(
            &store,
            None,
            "hyphae_memoir_create",
            &json!({"name": "../../../etc/passwd", "description": "traversal"}),
            false,
            None,
            false,
        );
        // Should store as a label, not access filesystem
        assert!(!result.content.is_empty());
        if !result.is_error {
            let list = call_tool(
                &store,
                None,
                "hyphae_memoir_list",
                &json!({}),
                false,
                None,
                false,
            );
            assert!(!list.is_error);
        }
    }

    // === Bounds checking and validation tests ===

    #[test]
    fn test_oversized_content_returns_error() {
        let store = test_store();
        let oversized = "A".repeat(32769);
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_store",
            &json!({"topic": "test", "content": oversized}),
            false,
            None,
            false,
        );
        assert!(result.is_error);
        assert!(result.content[0].text.contains("content"));
        assert!(result.content[0].text.contains("32768"));
    }

    #[test]
    fn test_negative_limit_gets_clamped() {
        let store = test_store();
        // Store a memory first so recall can return results
        call_tool(
            &store,
            None,
            "hyphae_memory_store",
            &json!({"topic": "clamp-test", "content": "some data"}),
            false,
            None,
            false,
        );
        // A negative limit should be clamped to 1 (minimum), not panic or error
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_recall",
            &json!({"query": "some data", "limit": -5}),
            false,
            None,
            false,
        );
        assert!(!result.is_error);
    }

    #[test]
    fn test_whitespace_only_topic_returns_error() {
        let store = test_store();
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_store",
            &json!({"topic": "   \t  ", "content": "some content"}),
            false,
            None,
            false,
        );
        assert!(result.is_error);
        assert!(result.content[0].text.contains("topic"));
    }

    // === Memoir edge-case tests ===

    #[test]
    fn test_memoir_create_duplicate_name_returns_error() {
        let store = test_store();
        let result1 = call_tool(
            &store,
            None,
            "hyphae_memoir_create",
            &json!({"name": "my-memoir", "description": "first"}),
            false,
            None,
            false,
        );
        assert!(!result1.is_error);
        assert!(result1.content[0].text.contains("my-memoir"));

        let result2 = call_tool(
            &store,
            None,
            "hyphae_memoir_create",
            &json!({"name": "my-memoir", "description": "duplicate"}),
            false,
            None,
            false,
        );
        assert!(result2.is_error);
    }

    #[test]
    fn test_memoir_add_concept_nonexistent_memoir_returns_error() {
        let store = test_store();
        let result = call_tool(
            &store,
            None,
            "hyphae_memoir_add_concept",
            &json!({
                "memoir": "does-not-exist",
                "name": "SomeConcept",
                "definition": "A concept in a nonexistent memoir"
            }),
            false,
            None,
            false,
        );
        assert!(result.is_error);
        assert!(result.content[0].text.contains("not found"));
    }

    #[test]
    fn test_memoir_refine_updates_definition_and_increments_revision() {
        let store = test_store();
        call_tool(
            &store,
            None,
            "hyphae_memoir_create",
            &json!({"name": "refine-memoir", "description": "for refine test"}),
            false,
            None,
            false,
        );
        call_tool(
            &store,
            None,
            "hyphae_memoir_add_concept",
            &json!({
                "memoir": "refine-memoir",
                "name": "MyConcept",
                "definition": "original definition"
            }),
            false,
            None,
            false,
        );

        let refine_result = call_tool(
            &store,
            None,
            "hyphae_memoir_refine",
            &json!({
                "memoir": "refine-memoir",
                "name": "MyConcept",
                "definition": "updated definition"
            }),
            false,
            None,
            false,
        );
        assert!(!refine_result.is_error);
        // Revision should have incremented; initial is 1, after refine should be 2
        assert!(refine_result.content[0].text.contains("r2"));
    }

    #[test]
    fn test_memoir_link_nonexistent_concepts_returns_error() {
        let store = test_store();
        call_tool(
            &store,
            None,
            "hyphae_memoir_create",
            &json!({"name": "link-memoir", "description": "for link test"}),
            false,
            None,
            false,
        );

        let result = call_tool(
            &store,
            None,
            "hyphae_memoir_link",
            &json!({
                "memoir": "link-memoir",
                "from": "GhostA",
                "to": "GhostB",
                "relation": "related_to"
            }),
            false,
            None,
            false,
        );
        assert!(result.is_error);
        assert!(result.content[0].text.contains("not found"));
    }

    #[test]
    fn test_memoir_link_self_referential_returns_error() {
        let store = test_store();
        call_tool(
            &store,
            None,
            "hyphae_memoir_create",
            &json!({"name": "self-link-memoir", "description": "for self-link test"}),
            false,
            None,
            false,
        );
        call_tool(
            &store,
            None,
            "hyphae_memoir_add_concept",
            &json!({
                "memoir": "self-link-memoir",
                "name": "ConceptA",
                "definition": "a concept"
            }),
            false,
            None,
            false,
        );

        let result = call_tool(
            &store,
            None,
            "hyphae_memoir_link",
            &json!({
                "memoir": "self-link-memoir",
                "from": "ConceptA",
                "to": "ConceptA",
                "relation": "related_to"
            }),
            false,
            None,
            false,
        );
        assert!(result.is_error);
        assert!(result.content[0].text.contains("itself"));
    }

    #[test]
    fn test_memoir_inspect_returns_graph_with_correct_depth() {
        let store = test_store();
        call_tool(
            &store,
            None,
            "hyphae_memoir_create",
            &json!({"name": "inspect-memoir", "description": "for inspect test"}),
            false,
            None,
            false,
        );
        call_tool(
            &store,
            None,
            "hyphae_memoir_add_concept",
            &json!({
                "memoir": "inspect-memoir",
                "name": "Alpha",
                "definition": "First concept"
            }),
            false,
            None,
            false,
        );
        call_tool(
            &store,
            None,
            "hyphae_memoir_add_concept",
            &json!({
                "memoir": "inspect-memoir",
                "name": "Beta",
                "definition": "Second concept"
            }),
            false,
            None,
            false,
        );
        call_tool(
            &store,
            None,
            "hyphae_memoir_link",
            &json!({
                "memoir": "inspect-memoir",
                "from": "Alpha",
                "to": "Beta",
                "relation": "related_to"
            }),
            false,
            None,
            false,
        );

        let result = call_tool(
            &store,
            None,
            "hyphae_memoir_inspect",
            &json!({"memoir": "inspect-memoir", "name": "Alpha", "depth": 1}),
            false,
            None,
            false,
        );
        assert!(!result.is_error);
        let text = &result.content[0].text;
        assert!(text.contains("Alpha"));
        assert!(text.contains("Graph (depth=1)"));
        assert!(text.contains("Beta"));
    }

    #[test]
    fn test_memoir_search_returns_matching_results() {
        let store = test_store();
        call_tool(
            &store,
            None,
            "hyphae_memoir_create",
            &json!({"name": "search-memoir", "description": "for search test"}),
            false,
            None,
            false,
        );
        call_tool(
            &store,
            None,
            "hyphae_memoir_add_concept",
            &json!({
                "memoir": "search-memoir",
                "name": "Hyphae",
                "definition": "Fungal threads that form the mycelium network"
            }),
            false,
            None,
            false,
        );
        call_tool(
            &store,
            None,
            "hyphae_memoir_add_concept",
            &json!({
                "memoir": "search-memoir",
                "name": "Spore",
                "definition": "Reproductive unit of fungi"
            }),
            false,
            None,
            false,
        );

        let result = call_tool(
            &store,
            None,
            "hyphae_memoir_search",
            &json!({"memoir": "search-memoir", "query": "fungal"}),
            false,
            None,
            false,
        );
        assert!(!result.is_error);
        let text = &result.content[0].text;
        assert!(text.contains("Hyphae"));
        assert!(!text.contains("Spore"));
    }

    #[test]
    fn test_memoir_search_all_searches_across_multiple_memoirs() {
        let store = test_store();
        call_tool(
            &store,
            None,
            "hyphae_memoir_create",
            &json!({"name": "memoir-one", "description": "first"}),
            false,
            None,
            false,
        );
        call_tool(
            &store,
            None,
            "hyphae_memoir_create",
            &json!({"name": "memoir-two", "description": "second"}),
            false,
            None,
            false,
        );
        call_tool(
            &store,
            None,
            "hyphae_memoir_add_concept",
            &json!({
                "memoir": "memoir-one",
                "name": "ConceptA",
                "definition": "A rare orchid found in the rainforest"
            }),
            false,
            None,
            false,
        );
        call_tool(
            &store,
            None,
            "hyphae_memoir_add_concept",
            &json!({
                "memoir": "memoir-two",
                "name": "ConceptB",
                "definition": "A rare beetle found in the rainforest"
            }),
            false,
            None,
            false,
        );

        let result = call_tool(
            &store,
            None,
            "hyphae_memoir_search_all",
            &json!({"query": "rainforest"}),
            false,
            None,
            false,
        );
        assert!(!result.is_error);
        let text = &result.content[0].text;
        assert!(text.contains("ConceptA"));
        assert!(text.contains("ConceptB"));
        assert!(text.contains("memoir-one"));
        assert!(text.contains("memoir-two"));
    }

    #[test]
    fn test_memoir_input_validation_empty_memoir_name() {
        let store = test_store();
        let result = call_tool(
            &store,
            None,
            "hyphae_memoir_create",
            &json!({"name": "", "description": "empty name"}),
            false,
            None,
            false,
        );
        assert!(result.is_error);
        assert!(result.content[0].text.contains("name"));
    }

    #[test]
    fn test_memoir_input_validation_empty_concept_name() {
        let store = test_store();
        call_tool(
            &store,
            None,
            "hyphae_memoir_create",
            &json!({"name": "val-memoir", "description": "for validation"}),
            false,
            None,
            false,
        );
        let result = call_tool(
            &store,
            None,
            "hyphae_memoir_add_concept",
            &json!({
                "memoir": "val-memoir",
                "name": "",
                "definition": "valid definition"
            }),
            false,
            None,
            false,
        );
        assert!(result.is_error);
        assert!(result.content[0].text.contains("name"));
    }

    #[test]
    fn test_memoir_input_validation_oversized_definition() {
        let store = test_store();
        call_tool(
            &store,
            None,
            "hyphae_memoir_create",
            &json!({"name": "oversize-memoir", "description": "for size test"}),
            false,
            None,
            false,
        );
        let oversized_def = "D".repeat(32769);
        let result = call_tool(
            &store,
            None,
            "hyphae_memoir_add_concept",
            &json!({
                "memoir": "oversize-memoir",
                "name": "BigConcept",
                "definition": oversized_def
            }),
            false,
            None,
            false,
        );
        assert!(result.is_error);
        assert!(result.content[0].text.contains("definition"));
        assert!(result.content[0].text.contains("32768"));
    }

    // --- RAG tool tests ---

    #[test]
    fn test_tool_ingest_file() {
        use std::fs;
        use tempfile::TempDir;

        let store = test_store();
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test_doc.md");
        fs::write(&path, "# Hello\n\nThis is a test document with some content.\n\n## Section\n\nMore text here.").unwrap();

        let result = call_tool(
            &store,
            None,
            "hyphae_ingest_file",
            &json!({"path": path.to_str().unwrap()}),
            false,
            None,
            false,
        );
        assert!(
            !result.is_error,
            "unexpected error: {}",
            result.content[0].text
        );
        assert!(result.content[0].text.contains("Ingested"));
        assert!(result.content[0].text.contains("document(s)"));
        assert!(result.content[0].text.contains("chunk(s)"));
    }

    #[test]
    fn test_tool_search_docs() {
        use std::fs;
        use tempfile::TempDir;

        let store = test_store();
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("searchable.md");
        fs::write(
            &path,
            "# Mycelium\n\nMycelium is the vegetative part of a fungus.",
        )
        .unwrap();

        // Ingest first
        call_tool(
            &store,
            None,
            "hyphae_ingest_file",
            &json!({"path": path.to_str().unwrap()}),
            false,
            None,
            false,
        );

        let result = call_tool(
            &store,
            None,
            "hyphae_search_docs",
            &json!({"query": "mycelium fungus"}),
            false,
            None,
            false,
        );
        assert!(
            !result.is_error,
            "unexpected error: {}",
            result.content[0].text
        );
        assert!(
            result.content[0].text.to_lowercase().contains("mycelium")
                || result.content[0].text.to_lowercase().contains("fungus")
        );
    }

    #[test]
    fn test_tool_list_sources() {
        use std::fs;
        use tempfile::TempDir;

        let store = test_store();
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("listed.txt");
        fs::write(&path, "Some content to list.").unwrap();

        call_tool(
            &store,
            None,
            "hyphae_ingest_file",
            &json!({"path": path.to_str().unwrap()}),
            false,
            None,
            false,
        );

        let result = call_tool(
            &store,
            None,
            "hyphae_list_sources",
            &json!({}),
            false,
            None,
            false,
        );
        assert!(
            !result.is_error,
            "unexpected error: {}",
            result.content[0].text
        );
        assert!(
            result.content[0].text.contains("listed.txt")
                || result.content[0].text.contains("listed")
        );
    }

    #[test]
    fn test_tool_forget_source() {
        use std::fs;
        use tempfile::TempDir;

        let store = test_store();
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("to_forget.txt");
        fs::write(&path, "Content that will be forgotten.").unwrap();

        let path_str = path.to_str().unwrap();

        call_tool(
            &store,
            None,
            "hyphae_ingest_file",
            &json!({"path": path_str}),
            false,
            None,
            false,
        );

        let result = call_tool(
            &store,
            None,
            "hyphae_forget_source",
            &json!({"path": path_str}),
            false,
            None,
            false,
        );
        assert!(
            !result.is_error,
            "unexpected error: {}",
            result.content[0].text
        );
        assert!(result.content[0].text.contains("Deleted"));

        // Verify it's gone
        let list_result = call_tool(
            &store,
            None,
            "hyphae_list_sources",
            &json!({}),
            false,
            None,
            false,
        );
        assert!(!list_result.content[0].text.contains("to_forget.txt"));
    }

    #[test]
    fn test_tool_search_docs_no_results() {
        let store = test_store();
        let result = call_tool(
            &store,
            None,
            "hyphae_search_docs",
            &json!({"query": "nonexistent unicorn content"}),
            false,
            None,
            false,
        );
        assert!(!result.is_error);
        assert!(result.content[0].text.contains("No results"));
    }

    #[test]
    fn test_tool_forget_source_not_found() {
        let store = test_store();
        let result = call_tool(
            &store,
            None,
            "hyphae_forget_source",
            &json!({"path": "/nonexistent/path.txt"}),
            false,
            None,
            false,
        );
        assert!(result.is_error);
        assert!(
            result.content[0].text.contains("not found")
                || result.content[0].text.contains("Source not found")
        );
    }

    #[test]
    fn test_tool_search_all_empty() {
        let store = test_store();
        let result = call_tool(
            &store,
            None,
            "hyphae_search_all",
            &json!({"query": "anything"}),
            false,
            None,
            false,
        );
        assert!(!result.is_error);
        assert!(result.content[0].text.contains("No results"));
    }

    #[test]
    fn test_tool_search_all_memories_and_docs() {
        use std::fs;
        use tempfile::TempDir;

        let store = test_store();

        // Store a memory
        call_tool(
            &store,
            None,
            "hyphae_memory_store",
            &json!({"topic": "architecture", "content": "The system uses PostgreSQL for data storage"}),
            false,
            None,
            false,
        );

        // Ingest a document
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("guide.md");
        fs::write(
            &path,
            "# Storage Guide\n\nPostgreSQL is the primary database for production workloads.",
        )
        .unwrap();
        call_tool(
            &store,
            None,
            "hyphae_ingest_file",
            &json!({"path": path.to_str().unwrap()}),
            false,
            None,
            false,
        );

        // Search across both
        let result = call_tool(
            &store,
            None,
            "hyphae_search_all",
            &json!({"query": "PostgreSQL database"}),
            false,
            None,
            false,
        );
        assert!(
            !result.is_error,
            "search_all error: {}",
            result.content[0].text
        );
        let text = &result.content[0].text;
        assert!(
            text.contains("[memory]") || text.contains("[doc:"),
            "should contain tagged results"
        );
    }

    #[test]
    fn test_tool_search_all_missing_query() {
        let store = test_store();
        let result = call_tool(
            &store,
            None,
            "hyphae_search_all",
            &json!({}),
            false,
            None,
            false,
        );
        assert!(result.is_error);
        assert!(result.content[0].text.contains("query"));
    }

    #[test]
    fn test_tool_search_all_include_docs_false() {
        use std::fs;
        use tempfile::TempDir;

        let store = test_store();

        call_tool(
            &store,
            None,
            "hyphae_memory_store",
            &json!({"topic": "test", "content": "Kubernetes cluster management"}),
            false,
            None,
            false,
        );

        let dir = TempDir::new().unwrap();
        let path = dir.path().join("k8s.md");
        fs::write(&path, "Kubernetes pod scheduling and orchestration.").unwrap();
        call_tool(
            &store,
            None,
            "hyphae_ingest_file",
            &json!({"path": path.to_str().unwrap()}),
            false,
            None,
            false,
        );

        let result = call_tool(
            &store,
            None,
            "hyphae_search_all",
            &json!({"query": "Kubernetes", "include_docs": false}),
            false,
            None,
            false,
        );
        assert!(!result.is_error);
        let text = &result.content[0].text;
        // Should contain memory result but no doc results
        assert!(
            !text.contains("[doc:"),
            "should not include doc results when include_docs=false"
        );
    }

    #[test]
    fn test_is_session_query_detects_keywords() {
        assert!(memory::is_session_query("what did I do last session"));
        assert!(memory::is_session_query("last time I worked on auth"));
        assert!(memory::is_session_query("what happened yesterday"));
        assert!(memory::is_session_query("earlier today I fixed a bug"));
        assert!(memory::is_session_query("show me previous changes"));
        assert!(memory::is_session_query("SESSION summary"));
    }

    #[test]
    fn test_is_session_query_rejects_non_session() {
        assert!(!memory::is_session_query("how to parse JSON"));
        assert!(!memory::is_session_query("authentication flow"));
        assert!(!memory::is_session_query("database schema design"));
    }

    // ─────────────────────────────────────────────────────────────────────────────
    // Secrets Rejection Tests
    // ─────────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_store_with_reject_secrets_true_blocks_api_key() {
        let store = test_store();
        let args = json!({
            "topic": "config",
            "content": "api_key = sk1234567890abcdefghij",
            "importance": "medium"
        });
        let result = memory::tool_store(&store, None, &args, false, None, true);
        assert!(result.is_error);
        assert!(result.content[0].text.contains("Storing blocked"));
        assert!(result.content[0].text.contains("secrets detected"));
    }

    #[test]
    fn test_store_with_reject_secrets_true_blocks_github_token() {
        let store = test_store();
        let args = json!({
            "topic": "credentials",
            "content": "ghp_1234567890abcdefghijklmnopqrstuvwxyz",
            "importance": "high"
        });
        let result = memory::tool_store(&store, None, &args, false, None, true);
        assert!(result.is_error);
        assert!(result.content[0].text.contains("Storing blocked"));
    }

    #[test]
    fn test_store_with_reject_secrets_false_allows_api_key() {
        let store = test_store();
        let args = json!({
            "topic": "config",
            "content": "api_key = sk1234567890abcdefghij",
            "importance": "medium"
        });
        let result = memory::tool_store(&store, None, &args, false, None, false);
        // Should store successfully (though it warns about secrets)
        assert!(!result.is_error);
    }

    #[test]
    fn test_store_with_reject_secrets_allows_normal_content() {
        let store = test_store();
        let args = json!({
            "topic": "learning",
            "content": "How to debug memory issues in Rust",
            "importance": "medium"
        });
        let result = memory::tool_store(&store, None, &args, false, None, true);
        assert!(!result.is_error);
    }

    #[test]
    fn test_store_with_reject_secrets_blocks_private_key() {
        let store = test_store();
        let args = json!({
            "topic": "security",
            "content": "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA...",
            "importance": "critical"
        });
        let result = memory::tool_store(&store, None, &args, false, None, true);
        assert!(result.is_error);
        assert!(result.content[0].text.contains("Storing blocked"));
        assert!(result.content[0].text.contains("private key"));
    }

    #[test]
    fn test_store_with_reject_secrets_blocks_aws_key() {
        let store = test_store();
        let args = json!({
            "topic": "credentials",
            "content": "AWS_ACCESS_KEY_ID = AKIAIOSFODNN7EXAMPLE",
            "importance": "high"
        });
        let result = memory::tool_store(&store, None, &args, false, None, true);
        assert!(result.is_error);
        assert!(result.content[0].text.contains("Storing blocked"));
        assert!(result.content[0].text.contains("AWS"));
    }
}
