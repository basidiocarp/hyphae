use std::fs;

use hyphae_core::{Importance, Memory, MemoryStore};
use hyphae_mcp::tools::{call_tool, tool_definitions};
use hyphae_store::{SHARED_PROJECT, SqliteStore};
use serde_json::json;
use tempfile::TempDir;

fn test_store() -> SqliteStore {
    SqliteStore::in_memory().expect("in-memory store")
}

#[test]
fn tool_definitions_include_versioned_command_output_contract() {
    let defs = tool_definitions(false);
    let tools = defs["tools"].as_array().expect("tools array");
    let command_output = tools
        .iter()
        .find(|tool| tool["name"] == "hyphae_store_command_output")
        .expect("command output tool");
    let schema = &command_output["inputSchema"];

    assert_eq!(
        schema["properties"]["schema_version"]["type"].as_str(),
        Some("string")
    );
    assert!(
        schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("schema_version"))
    );
    assert!(
        schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("command"))
    );
    assert!(
        schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("output"))
    );
    assert!(schema["properties"]["project_root"].is_object());
    assert!(schema["properties"]["worktree_id"].is_object());
}

#[test]
fn search_all_rejects_partial_identity_pair() {
    let store = test_store();
    let result = call_tool(
        &store,
        None,
        "hyphae_search_all",
        &json!({
            "query": "identity scoped search",
            "project_root": "/repo/demo"
        }),
        false,
        Some("demo"),
        false,
    );

    assert!(result.is_error);
    assert!(
        result.content[0]
            .text
            .contains("project_root and worktree_id must be provided together")
    );
}

#[test]
fn search_all_keeps_identity_scoping_and_shared_results() {
    let store = test_store();

    let alpha = Memory::builder(
        "identity".into(),
        "Alpha memory search target".into(),
        Importance::Medium,
    )
    .project("demo".into())
    .worktree("/repo/demo/wt-alpha".into())
    .build();
    let beta = Memory::builder(
        "identity".into(),
        "Beta memory search target".into(),
        Importance::Medium,
    )
    .project("demo".into())
    .worktree("/repo/demo/wt-beta".into())
    .build();
    let shared = Memory::builder(
        "identity".into(),
        "Shared memory search target".into(),
        Importance::Medium,
    )
    .project(SHARED_PROJECT.into())
    .build();
    store.store(alpha).unwrap();
    store.store(beta).unwrap();
    store.store(shared).unwrap();

    let dir = TempDir::new().unwrap();
    let path = dir.path().join("identity.md");
    fs::write(&path, "Identity search target doc chunk for project demo.").unwrap();
    let ingest = call_tool(
        &store,
        None,
        "hyphae_ingest_file",
        &json!({"path": path.to_str().unwrap()}),
        false,
        Some("demo"),
        false,
    );
    assert!(!ingest.is_error);

    let result = call_tool(
        &store,
        None,
        "hyphae_search_all",
        &json!({
            "query": "search target",
            "project_root": "/repo/demo/wt-alpha",
            "worktree_id": "wt-alpha"
        }),
        false,
        Some("demo"),
        false,
    );

    assert!(
        !result.is_error,
        "search_all error: {}",
        result.content[0].text
    );
    let text = &result.content[0].text;
    assert!(text.contains("Alpha memory search target"));
    assert!(text.contains("Shared memory search target"));
    assert!(text.contains("Identity search target doc chunk"));
    assert!(!text.contains("Beta memory search target"));
}

#[test]
fn memory_store_blocks_secrets_when_enabled() {
    let store = test_store();
    let result = call_tool(
        &store,
        None,
        "hyphae_memory_store",
        &json!({
            "topic": "config",
            "content": "api_key = sk1234567890abcdefghij",
            "importance": "medium"
        }),
        false,
        None,
        true,
    );

    assert!(result.is_error);
    assert!(result.content[0].text.contains("Storing blocked"));
    assert!(result.content[0].text.contains("secrets detected"));
}

#[test]
fn memory_store_allows_normal_content_when_secret_rejection_enabled() {
    let store = test_store();
    let result = call_tool(
        &store,
        None,
        "hyphae_memory_store",
        &json!({
            "topic": "learning",
            "content": "How to debug memory issues in Rust",
            "importance": "medium"
        }),
        false,
        None,
        true,
    );

    assert!(!result.is_error);
}
