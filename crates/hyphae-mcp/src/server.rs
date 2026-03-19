use std::io::{self, BufRead, Write};

use serde_json::{Value, json};
use tracing::{debug, error};

use hyphae_core::Embedder;
use hyphae_store::SqliteStore;

use crate::protocol::{JsonRpcMessage, JsonRpcResponse};
use crate::tools;

const SERVER_NAME: &str = "hyphae";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
const PROTOCOL_VERSION: &str = "2024-11-05";

/// Number of non-store tool calls before we nudge the agent to store.
const STORE_NUDGE_THRESHOLD: u32 = 10;

/// Build an initial context string with recent memories for the project.
fn initial_context(store: &SqliteStore, project: Option<&str>) -> String {
    use hyphae_core::MemoryStore;

    let proj = project.unwrap_or("default");

    // Get recent session context
    let session_ctx = store.session_context(proj, 3).unwrap_or_default();

    // Get top memories for the project context topic
    let project_topic = format!("context-{proj}");
    let recent_memories = store
        .get_by_topic(&project_topic, project)
        .unwrap_or_default();

    // Get decision memories
    let decisions_topic = format!("decisions-{proj}");
    let decisions = store
        .get_by_topic(&decisions_topic, project)
        .unwrap_or_default();

    if session_ctx.is_empty() && recent_memories.is_empty() && decisions.is_empty() {
        return String::new();
    }

    let mut ctx = String::from("\n\n[Hyphae Auto-Recall for this session]\n");

    if !session_ctx.is_empty() {
        ctx.push_str("Recent sessions:\n");
        for s in &session_ctx {
            if let Some(summary) = &s.summary {
                ctx.push_str(&format!("- {summary}\n"));
            }
        }
    }

    let truncate = |s: &str| -> String {
        if s.len() > 200 {
            format!("{}...", &s[..200])
        } else {
            s.to_string()
        }
    };

    if !decisions.is_empty() {
        ctx.push_str("Key decisions:\n");
        for m in decisions.iter().take(3) {
            ctx.push_str(&format!("- {}\n", truncate(&m.summary)));
        }
    }

    if !recent_memories.is_empty() {
        ctx.push_str("Project context:\n");
        for m in recent_memories.iter().take(5) {
            ctx.push_str(&format!("- {}\n", truncate(&m.summary)));
        }
    }

    ctx
}

/// Run the MCP server on stdio. Blocks until stdin is closed.
pub fn run_server(
    store: &SqliteStore,
    embedder: Option<&dyn Embedder>,
    compact: bool,
    project: Option<String>,
) -> anyhow::Result<()> {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    let mut calls_since_store: u32 = 0;

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                error!("stdin read error: {e}");
                break;
            }
        };

        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let msg: JsonRpcMessage = match serde_json::from_str(line) {
            Ok(m) => m,
            Err(e) => {
                error!("invalid JSON-RPC: {e}");
                // Send parse error if we can
                let resp = JsonRpcResponse::err(Value::Null, -32700, format!("parse error: {e}"));
                write_response(&mut stdout, &resp)?;
                continue;
            }
        };

        let method = msg.method.as_deref().unwrap_or("");
        debug!("MCP request: {method}");

        // Notifications have no id — don't respond
        let id = match msg.id {
            Some(id) => id,
            None => {
                debug!("received notification: {}", method);
                continue;
            }
        };

        let response = match method {
            "initialize" => handle_initialize(id, store, project.as_deref()),
            "ping" => JsonRpcResponse::ok(id, json!({})),
            "tools/list" => handle_tools_list(id, embedder.is_some()),
            "tools/call" => handle_tools_call(
                id,
                &msg.params,
                store,
                embedder,
                compact,
                project.as_deref(),
                &mut calls_since_store,
            ),
            other => JsonRpcResponse::method_not_found(id, other),
        };

        write_response(&mut stdout, &response)?;
    }

    Ok(())
}

fn write_response(stdout: &mut io::Stdout, resp: &JsonRpcResponse) -> anyhow::Result<()> {
    let mut lock = stdout.lock();
    serde_json::to_writer(&mut lock, resp)?;
    lock.write_all(b"\n")?;
    lock.flush()?;
    Ok(())
}

fn handle_initialize(id: Value, store: &SqliteStore, project: Option<&str>) -> JsonRpcResponse {
    let ctx = initial_context(store, project);
    let instructions = if ctx.is_empty() {
        HYPHAE_INSTRUCTIONS.to_string()
    } else {
        format!("{HYPHAE_INSTRUCTIONS}{ctx}")
    };

    JsonRpcResponse::ok(
        id,
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": SERVER_NAME,
                "version": SERVER_VERSION
            },
            "instructions": instructions
        }),
    )
}

const HYPHAE_INSTRUCTIONS: &str = "\
Use Hyphae (Infinite Context Memory) proactively to maintain long-term memory across sessions.\n\
\n\
RECALL (hyphae_memory_recall): At the start of a task, search for relevant past context — decisions, \
resolved errors, user preferences. Search only what is relevant, do not dump everything.\n\
\n\
STORE (hyphae_memory_store): Automatically store important information:\n\
- Architecture decisions → topic: \"decisions-{project}\"\n\
- Resolved errors with solutions → topic: \"errors-resolved\"\n\
- User preferences discovered in session → topic: \"preferences\"\n\
- Project context after significant work → topic: \"context-{project}\"\n\
\n\
Do NOT store: trivial details, information already in CLAUDE.md, ephemeral state.\n\
\n\
Importance levels: critical (never forgotten), high (slow decay), medium (normal), low (fast decay).\n\
\n\
MEMOIR (hyphae_memoir_create, _add_concept, _link, _refine): Build knowledge graphs for structural \
understanding that outlasts individual memories. Create memoirs when you discover:\n\
- System architecture (services, their roles, and how they connect)\n\
- Domain models (key entities, their relationships, business rules)\n\
- Recurring patterns (error patterns, design patterns, team conventions)\n\
Workflow: create memoir → add concepts with definitions → link related concepts → refine as understanding deepens.\n\
Use hyphae_import_code_graph after significant code changes to keep code structure memoirs current.\n\
\n\
CROSS-PROJECT: Store universal patterns with project: \"_shared\" so they are visible across all projects. \
Use hyphae_recall_global when local recall returns no results or when working on cross-project integration.\n\
\n\
CONSOLIDATE (hyphae_memory_consolidate): When a topic accumulates 15+ memories, consolidate to merge \
redundant entries and improve recall quality.";

fn handle_tools_list(id: Value, has_embedder: bool) -> JsonRpcResponse {
    JsonRpcResponse::ok(id, tools::tool_definitions(has_embedder))
}

fn handle_tools_call(
    id: Value,
    params: &Option<Value>,
    store: &SqliteStore,
    embedder: Option<&dyn Embedder>,
    compact: bool,
    project: Option<&str>,
    calls_since_store: &mut u32,
) -> JsonRpcResponse {
    let params = match params {
        Some(p) => p,
        None => {
            return JsonRpcResponse::err(id, -32602, "missing params".into());
        }
    };

    let tool_name = match params.get("name").and_then(|v| v.as_str()) {
        Some(n) => n,
        None => {
            return JsonRpcResponse::err(id, -32602, "missing tool name".into());
        }
    };

    let args = params.get("arguments").cloned().unwrap_or(json!({}));

    // Track store calls to nudge the agent
    if tool_name == "hyphae_memory_store" {
        *calls_since_store = 0;
    } else {
        *calls_since_store += 1;
    }

    let mut result = tools::call_tool(store, embedder, tool_name, &args, compact, project);

    // Nudge: append a store reminder if too many calls without storing
    if *calls_since_store >= STORE_NUDGE_THRESHOLD && tool_name != "hyphae_memory_store" {
        result = result.with_hint(&format!(
            "\n[Hyphae: {} tool calls since last store. \
             Consider saving important context with hyphae_memory_store before it is lost.]",
            calls_since_store
        ));
    }

    let result_value = match serde_json::to_value(result) {
        Ok(v) => v,
        Err(e) => {
            error!("serialization error: {e}");
            return JsonRpcResponse::err(id, -32603, format!("internal serialization error: {e}"));
        }
    };

    JsonRpcResponse::ok(id, result_value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_store() -> SqliteStore {
        SqliteStore::in_memory().unwrap()
    }

    #[test]
    fn test_handle_initialize_returns_capabilities() {
        let store = SqliteStore::in_memory().unwrap();
        let resp = handle_initialize(json!(1), &store, None);
        let result = resp.result.unwrap();
        assert_eq!(result["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(result["serverInfo"]["name"], SERVER_NAME);
        assert!(result["capabilities"]["tools"].is_object());
        assert!(result["instructions"].as_str().unwrap().contains("Hyphae"));
    }

    #[test]
    fn test_handle_tools_list_returns_tools() {
        let resp = handle_tools_list(json!(2), false);
        let result = resp.result.unwrap();
        let tools = result["tools"].as_array().unwrap();
        assert!(!tools.is_empty());
        // All tools should have name and inputSchema
        for tool in tools {
            assert!(tool["name"].as_str().is_some());
            assert!(tool["inputSchema"].is_object());
        }
    }

    #[test]
    fn test_handle_tools_call_missing_params() {
        let store = test_store();
        let mut counter = 0;
        let resp = handle_tools_call(json!(3), &None, &store, None, false, None, &mut counter);
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32602);
    }

    #[test]
    fn test_handle_tools_call_missing_tool_name() {
        let store = test_store();
        let mut counter = 0;
        let params = Some(json!({"arguments": {}}));
        let resp = handle_tools_call(json!(4), &params, &store, None, false, None, &mut counter);
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32602);
    }

    #[test]
    fn test_handle_tools_call_unknown_tool() {
        let store = test_store();
        let mut counter = 0;
        let params = Some(json!({"name": "nonexistent", "arguments": {}}));
        let resp = handle_tools_call(json!(5), &params, &store, None, false, None, &mut counter);
        // Unknown tool returns a result with is_error, not a JSON-RPC error
        let result = resp.result.unwrap();
        assert_eq!(result["isError"], true);
    }

    #[test]
    fn test_handle_tools_call_valid_store() {
        let store = test_store();
        let mut counter = 5;
        let params = Some(json!({
            "name": "hyphae_memory_store",
            "arguments": {
                "topic": "test",
                "content": "hello world",
                "importance": "medium"
            }
        }));
        let resp = handle_tools_call(json!(6), &params, &store, None, false, None, &mut counter);
        assert!(resp.error.is_none());
        // Store call resets counter
        assert_eq!(counter, 0);
    }

    #[test]
    fn test_store_nudge_after_threshold() {
        let store = test_store();
        let mut counter = STORE_NUDGE_THRESHOLD;
        let params = Some(json!({
            "name": "hyphae_memory_recall",
            "arguments": {"query": "test"}
        }));
        let resp = handle_tools_call(json!(7), &params, &store, None, false, None, &mut counter);
        let result = resp.result.unwrap();
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("tool calls since last store"));
    }

    #[test]
    fn test_store_nudge_not_before_threshold() {
        let store = test_store();
        let mut counter = 0;
        let params = Some(json!({
            "name": "hyphae_memory_recall",
            "arguments": {"query": "test"}
        }));
        let resp = handle_tools_call(json!(8), &params, &store, None, false, None, &mut counter);
        let result = resp.result.unwrap();
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(!text.contains("tool calls since last store"));
    }

    #[test]
    fn test_calls_since_store_increments() {
        let store = test_store();
        let mut counter = 0;
        let params = Some(json!({
            "name": "hyphae_memory_recall",
            "arguments": {"query": "test"}
        }));
        handle_tools_call(json!(9), &params, &store, None, false, None, &mut counter);
        assert_eq!(counter, 1);
        handle_tools_call(json!(10), &params, &store, None, false, None, &mut counter);
        assert_eq!(counter, 2);
    }
}
