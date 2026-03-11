use std::sync::OnceLock;

use serde_json::{Value, json};

use hyphae_core::{Embedder, Memoir, MemoirStore};
use hyphae_store::SqliteStore;

use crate::protocol::ToolResult;

mod memoir;
mod memory;
mod schema;

// ===========================================================================
// Tool schemas for tools/list
// ===========================================================================

static TOOL_DEFINITIONS: OnceLock<Vec<Value>> = OnceLock::new();

pub fn tool_definitions(has_embedder: bool) -> Value {
    let tools = TOOL_DEFINITIONS.get_or_init(|| schema::tool_definitions_json(has_embedder));
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
) -> ToolResult {
    match name {
        // Memory tools
        "hyphae_memory_store" => memory::tool_store(store, embedder, args, compact),
        "hyphae_memory_recall" => memory::tool_recall(store, embedder, args, compact),
        "hyphae_memory_forget" => memory::tool_forget(store, args),
        "hyphae_memory_update" => memory::tool_update(store, embedder, args),
        "hyphae_memory_consolidate" => memory::tool_consolidate(store, args),
        "hyphae_memory_list_topics" => memory::tool_list_topics(store),
        "hyphae_memory_stats" => memory::tool_stats(store),
        "hyphae_memory_health" => memory::tool_health(store, args),
        "hyphae_memory_embed_all" => memory::tool_embed_all(store, embedder, args),
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
        let result = call_tool(&store, None, "nonexistent_tool", &json!({}), false);
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
        );
        assert!(result.is_error);
        assert!(result.content[0].text.contains("content"));
    }

    #[test]
    fn test_recall_missing_query() {
        let store = test_store();
        let result = call_tool(&store, None, "hyphae_memory_recall", &json!({}), false);
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
        );
        assert!(!result.is_error);
        assert!(result.content[0].text.contains("No memories"));
    }

    #[test]
    fn test_forget_missing_id() {
        let store = test_store();
        let result = call_tool(&store, None, "hyphae_memory_forget", &json!({}), false);
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
        );
        assert!(!store_result.is_error);
        assert!(store_result.content[0].text.contains("Stored memory"));

        let recall_result = call_tool(
            &store,
            None,
            "hyphae_memory_recall",
            &json!({"query": "Rust SQLite"}),
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
        );
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_recall",
            &json!({"query": "Rust memory"}),
            true,
        );
        assert!(!result.is_error);
        assert!(result.content[0].text.contains("[proj]"));
    }

    #[test]
    fn test_stats_empty() {
        let store = test_store();
        let result = call_tool(&store, None, "hyphae_memory_stats", &json!({}), false);
        assert!(!result.is_error);
        assert!(result.content[0].text.contains("Memories: 0"));
    }

    #[test]
    fn test_list_topics_empty() {
        let store = test_store();
        let result = call_tool(&store, None, "hyphae_memory_list_topics", &json!({}), false);
        assert!(!result.is_error);
        assert!(result.content[0].text.contains("No topics"));
    }

    #[test]
    fn test_health_empty() {
        let store = test_store();
        let result = call_tool(&store, None, "hyphae_memory_health", &json!({}), false);
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
        );
        assert!(!result.is_error);
        let stats = call_tool(&store, None, "hyphae_memory_stats", &json!({}), false);
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
        );
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_recall",
            &json!({"query": "') OR 1=1 --"}),
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
        );
        assert!(!result.is_error);
        let recall = call_tool(
            &store,
            None,
            "hyphae_memory_recall",
            &json!({"query": "script alert"}),
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
        );
        assert!(!result.is_error);
        let list = call_tool(&store, None, "hyphae_memoir_list", &json!({}), false);
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
            );
            assert!(!result.is_error);
        }
        let stats = call_tool(&store, None, "hyphae_memory_stats", &json!({}), false);
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
            );
        }
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_recall",
            &json!({"query": "data", "topic": "beta"}),
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
            );
        }
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_consolidate",
            &json!({"topic": "consolidate-me", "summary": "All 10 details merged"}),
            false,
        );
        assert!(!result.is_error);
        let stats = call_tool(&store, None, "hyphae_memory_stats", &json!({}), false);
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
            );
            // Should either store safely (topic is just a string label) or reject
            // but must NOT crash or access filesystem
            assert!(!result.content.is_empty());
        }
        let stats = call_tool(&store, None, "hyphae_memory_stats", &json!({}), false);
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
        );
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_recall",
            &json!({"query": "normal\0injected"}),
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
            );
            assert!(!result.is_error, "Failed on unicode string: {:?}", s);
        }
        let stats = call_tool(&store, None, "hyphae_memory_stats", &json!({}), false);
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
        );
        // Should store normally, ignoring unknown fields
        assert!(!result.is_error);
        let recall = call_tool(
            &store,
            None,
            "hyphae_memory_recall",
            &json!({"query": "legit"}),
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
        );
        // Should store as a label, not access filesystem
        assert!(!result.content.is_empty());
        if !result.is_error {
            let list = call_tool(&store, None, "hyphae_memoir_list", &json!({}), false);
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
        );
        // A negative limit should be clamped to 1 (minimum), not panic or error
        let result = call_tool(
            &store,
            None,
            "hyphae_memory_recall",
            &json!({"query": "some data", "limit": -5}),
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
        );
        assert!(!result1.is_error);
        assert!(result1.content[0].text.contains("my-memoir"));

        let result2 = call_tool(
            &store,
            None,
            "hyphae_memoir_create",
            &json!({"name": "my-memoir", "description": "duplicate"}),
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
        );
        assert!(result.is_error);
        assert!(result.content[0].text.contains("not found"));
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
        );

        let result = call_tool(
            &store,
            None,
            "hyphae_memoir_inspect",
            &json!({"memoir": "inspect-memoir", "name": "Alpha", "depth": 1}),
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
        );

        let result = call_tool(
            &store,
            None,
            "hyphae_memoir_search",
            &json!({"memoir": "search-memoir", "query": "fungal"}),
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
        );
        call_tool(
            &store,
            None,
            "hyphae_memoir_create",
            &json!({"name": "memoir-two", "description": "second"}),
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
        );

        let result = call_tool(
            &store,
            None,
            "hyphae_memoir_search_all",
            &json!({"query": "rainforest"}),
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
        );
        assert!(result.is_error);
        assert!(result.content[0].text.contains("definition"));
        assert!(result.content[0].text.contains("32768"));
    }
}
