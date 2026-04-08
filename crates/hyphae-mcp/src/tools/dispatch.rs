// ─────────────────────────────────────────────────────────────────────────────
// Tool Dispatch
// ─────────────────────────────────────────────────────────────────────────────

use serde_json::Value;

use hyphae_core::{ConsolidationConfig, Embedder};
use hyphae_store::SqliteStore;

use crate::protocol::ToolResult;

use super::{context, ingest, memoir, memory, session};

pub fn call_tool(
    store: &SqliteStore,
    embedder: Option<&dyn Embedder>,
    name: &str,
    args: &Value,
    compact: bool,
    project: Option<&str>,
    reject_secrets: bool,
) -> ToolResult {
    call_tool_with_consolidation(
        store,
        embedder,
        &ConsolidationConfig::default(),
        name,
        args,
        compact,
        project,
        reject_secrets,
        &super::ToolTraceContext::default(),
    )
}

pub fn call_tool_with_consolidation(
    store: &SqliteStore,
    embedder: Option<&dyn Embedder>,
    consolidation: &ConsolidationConfig,
    name: &str,
    args: &Value,
    compact: bool,
    project: Option<&str>,
    reject_secrets: bool,
    trace: &super::ToolTraceContext,
) -> ToolResult {
    match name {
        // Memory tools
        "hyphae_memory_store" => memory::tool_store(
            store,
            embedder,
            consolidation,
            args,
            compact,
            project,
            reject_secrets,
            trace,
        ),
        "hyphae_memory_recall" => {
            memory::tool_recall(store, embedder, args, compact, project, trace)
        }
        "hyphae_memory_forget" => memory::tool_forget(store, args, trace),
        "hyphae_memory_invalidate" => memory::tool_invalidate(store, args, trace),
        "hyphae_memory_list_invalidated" => {
            memory::tool_list_invalidated(store, args, project, trace)
        }
        "hyphae_memory_update" => memory::tool_update(store, embedder, args, trace),
        "hyphae_memory_consolidate" => memory::tool_consolidate(store, args, trace),
        "hyphae_memory_list_topics" => memory::tool_list_topics(store, project, trace),
        "hyphae_memory_stats" => memory::tool_stats(store, project, trace),
        "hyphae_memory_health" => {
            memory::tool_health_with_rules(store, consolidation, args, project, trace)
        }
        "hyphae_memory_embed_all" => memory::tool_embed_all(store, embedder, args, project, trace),
        "hyphae_extract_lessons" => memory::tool_extract_lessons(store, args, project, trace),
        "hyphae_evaluate" => memory::tool_evaluate(store, args, project, trace),
        // Cross-project tools
        "hyphae_recall_global" => memory::tool_recall_global(store, args, compact, trace),
        "hyphae_promote_to_memoir" => memory::tool_promote_to_memoir(store, args, project, trace),
        // Memoir tools
        "hyphae_memoir_create" => memoir::tool_memoir_create(store, args, trace),
        "hyphae_memoir_list" => memoir::tool_memoir_list(store, trace),
        "hyphae_memoir_show" => memoir::tool_memoir_show(store, args, trace),
        "hyphae_memoir_add_concept" => memoir::tool_memoir_add_concept(store, args, trace),
        "hyphae_memoir_refine" => memoir::tool_memoir_refine(store, args, trace),
        "hyphae_memoir_search" => memoir::tool_memoir_search(store, args, trace),
        "hyphae_memoir_search_all" => memoir::tool_memoir_search_all(store, args, trace),
        "hyphae_memoir_link" => memoir::tool_memoir_link(store, args, trace),
        "hyphae_memoir_inspect" => memoir::tool_memoir_inspect(store, args, trace),
        "hyphae_import_code_graph" => {
            memoir::tool_import_code_graph(store, args, compact, project, trace)
        }
        "hyphae_code_query" => memoir::tool_code_query(store, args, compact, project, trace),
        // RAG tools
        "hyphae_ingest_file" => {
            ingest::tool_ingest_file(store, embedder, args, compact, project, trace)
        }
        "hyphae_search_docs" => {
            ingest::tool_search_docs(store, embedder, args, compact, project, trace)
        }
        "hyphae_list_sources" => ingest::tool_list_sources(store, project, trace),
        "hyphae_forget_source" => ingest::tool_forget_source(store, args, project, trace),
        "hyphae_search_all" => {
            ingest::tool_search_all(store, embedder, args, compact, project, trace)
        }
        // Command output tools
        "hyphae_store_command_output" => {
            ingest::tool_store_command_output(store, args, compact, project, trace)
        }
        "hyphae_get_command_chunks" => ingest::tool_get_command_chunks(store, args, trace),
        // Context gathering
        "hyphae_gather_context" => context::tool_gather_context(store, args, project, trace),
        // Session lifecycle tools
        "hyphae_session_start" => session::tool_session_start(store, embedder, args, trace),
        "hyphae_session_end" => session::tool_session_end(store, args, trace),
        "hyphae_session_context" => session::tool_session_context(store, args, trace),
        // Onboarding
        "hyphae_onboard" => super::onboard::tool_onboard(store, project, trace),
        _ => ToolResult::error(format!("unknown tool: {name}")),
    }
}
