use serde_json::Value;
use spore::logging::workflow_span;

use hyphae_core::{Embedder, Importance, Memory, MemoryStore, detect_git_context_from};
use hyphae_store::SqliteStore;

use crate::protocol::ToolResult;

use super::super::{
    ToolTraceContext, get_str, resolve_workspace_root, validate_max_length,
    validate_required_string, workflow_span_context,
};

/// Store a permanent governance policy memory.
///
/// This wraps `hyphae_memory_store` with `importance` fixed to
/// `Constitution`. Constitution memories never decay and are excluded
/// from consolidation. They are intended for rules that must persist
/// indefinitely, such as "never store secrets" or "always validate at
/// system boundaries".
pub(crate) fn tool_constitution_store(
    store: &SqliteStore,
    embedder: Option<&dyn Embedder>,
    args: &Value,
    compact: bool,
    project: Option<&str>,
    trace: &ToolTraceContext,
) -> ToolResult {
    let content = match validate_required_string(args, "content") {
        Ok(c) => c,
        Err(e) => return e,
    };
    if let Err(e) = validate_max_length(content, "content", 32768) {
        return e;
    }

    // Default topic to `constitution/<project>` when not provided.
    let topic_owned: String;
    let topic = if let Some(t) = get_str(args, "topic") {
        t
    } else {
        topic_owned = match project {
            Some(p) => format!("constitution/{p}"),
            None => "constitution".to_string(),
        };
        topic_owned.as_str()
    };

    let workflow_context = workflow_span_context(trace, resolve_workspace_root(args), None);
    let _workflow_span = workflow_span("constitution_store", &workflow_context).entered();

    let embedding = if let Some(emb) = embedder {
        let text = format!("{topic} {content}");
        match emb.embed(&text) {
            Ok(vec) => Some(vec),
            Err(e) => {
                tracing::warn!("embedding failed: {e}");
                None
            }
        }
    } else {
        None
    };

    let keywords: Vec<String> = args
        .get("keywords")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let mut builder = Memory::builder(
        topic.to_string(),
        content.to_string(),
        Importance::Constitution,
    )
    .keywords(keywords);

    if let Some(p) = project {
        builder = builder.project(p.to_string());
    }

    let git_context = detect_git_context_from(None);
    if let Some(branch) = get_str(args, "branch")
        .map(str::to_owned)
        .or(git_context.branch)
    {
        builder = builder.branch(branch);
    }
    if let Some(worktree) = get_str(args, "worktree")
        .map(str::to_owned)
        .or(git_context.worktree)
    {
        builder = builder.worktree(worktree);
    }

    if let Some(raw) = get_str(args, "raw_excerpt") {
        builder = builder.raw_excerpt(raw.to_string());
    }

    if let Some(ref vec) = embedding {
        builder = builder.embedding(vec.clone());
    }

    let memory = builder.build();

    match store.store(memory) {
        Ok(id) => {
            if compact {
                ToolResult::text(format!("ok:{id}"))
            } else {
                ToolResult::text(format!(
                    "Stored constitution policy: {id}\ntopic: {topic}\n\
                     This memory will never decay and is excluded from consolidation."
                ))
            }
        }
        Err(e) => ToolResult::error(format!("failed to store constitution: {e}")),
    }
}
