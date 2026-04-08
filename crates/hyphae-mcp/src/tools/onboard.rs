// ─────────────────────────────────────────────────────────────────────────────
// Onboarding Tool
// ─────────────────────────────────────────────────────────────────────────────

use serde_json::json;
use spore::logging::workflow_span;

use hyphae_core::{MemoirStore, MemoryStore};
use hyphae_store::SqliteStore;

use crate::protocol::ToolResult;

use super::{ToolTraceContext, workflow_span_context};

pub fn tool_onboard(
    store: &SqliteStore,
    project: Option<&str>,
    trace: &ToolTraceContext,
) -> ToolResult {
    let workflow_context = workflow_span_context(trace, None, None);
    let _workflow_span = workflow_span("onboard", &workflow_context).entered();
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
