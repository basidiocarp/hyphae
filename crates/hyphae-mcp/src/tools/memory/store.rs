use chrono::Utc;
use serde_json::Value;

use hyphae_core::{
    Embedder, Importance, Memory, MemoryId, MemoryStore, Weight, detect_git_context_from,
    detect_secrets,
};
use hyphae_store::SqliteStore;

use crate::protocol::ToolResult;

use super::super::{get_str, validate_max_length, validate_required_string};

pub(crate) fn tool_store(
    store: &SqliteStore,
    embedder: Option<&dyn Embedder>,
    consolidation: &hyphae_core::ConsolidationConfig,
    args: &Value,
    compact: bool,
    project: Option<&str>,
    reject_secrets: bool,
) -> ToolResult {
    let topic = match validate_required_string(args, "topic") {
        Ok(t) => t,
        Err(e) => return e,
    };
    let content = match validate_required_string(args, "content") {
        Ok(c) => c,
        Err(e) => return e,
    };
    if let Err(e) = validate_max_length(content, "content", 32768) {
        return e;
    }
    if let Some(raw) = get_str(args, "raw_excerpt") {
        if let Err(e) = validate_max_length(raw, "raw_excerpt", 65536) {
            return e;
        }
    }
    let importance_str = get_str(args, "importance").unwrap_or("medium");
    let importance = importance_str.parse().unwrap_or(Importance::Medium);

    if reject_secrets {
        let detected = detect_secrets(content);
        if !detected.is_empty() {
            return ToolResult::error(format!(
                "Storing blocked: secrets detected in content [{}]. \
                 To store anyway, disable reject_secrets in config. \
                 Detected: {}",
                topic,
                detected.join(", ")
            ));
        }
    }

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

    let mut builder = Memory::builder(topic.into(), content.into(), importance).keywords(keywords);

    if let Some(p) = project {
        builder = builder.project(p.to_string());
    }

    let git_context = detect_git_context_from(None);
    let detected_branch = git_context.branch;
    let detected_worktree = git_context.worktree;
    if let Some(branch) = get_str(args, "branch")
        .map(str::to_owned)
        .or(detected_branch)
    {
        builder = builder.branch(branch);
    }
    if let Some(worktree) = get_str(args, "worktree")
        .map(str::to_owned)
        .or(detected_worktree)
    {
        builder = builder.worktree(worktree);
    }

    if let Some(raw) = get_str(args, "raw_excerpt") {
        builder = builder.raw_excerpt(raw.into());
    }

    if let Some(ref vec) = embedding {
        builder = builder.embedding(vec.clone());
    }

    let memory = builder.build();

    if let Some(query_emb) = embedding {
        let text = format!("{topic} {content}");
        if let Ok(similar) = store.search_hybrid(&text, &query_emb, 1, 0, project) {
            if let Some((existing, score)) = similar.first() {
                if score > &0.85 && existing.topic == topic {
                    let mut updated = existing.clone();
                    updated.summary = content.to_string();
                    updated.updated_at = Utc::now();
                    updated.weight = Weight::default();
                    if let Some(raw) = get_str(args, "raw_excerpt") {
                        updated.raw_excerpt = Some(raw.into());
                    }
                    if let Some(keywords_arr) = args.get("keywords").and_then(|v| v.as_array()) {
                        updated.keywords = keywords_arr
                            .iter()
                            .filter_map(|v| v.as_str().map(String::from))
                            .collect();
                    }
                    updated.importance = importance;
                    updated.embedding = Some(query_emb);
                    if let Err(e) = store.update(&updated) {
                        return ToolResult::error(format!("failed to update: {e}"));
                    }

                    let warnings = detect_secrets(content);
                    return if compact {
                        if !warnings.is_empty() {
                            ToolResult::text(format!(
                                "ok:{}\n⚠️ Possible secrets detected: {}. Consider using hyphae_memory_forget to remove.",
                                updated.id,
                                warnings.join(", ")
                            ))
                        } else {
                            ToolResult::text(format!("ok:{}", updated.id))
                        }
                    } else {
                        let mut msg = format!(
                            "Updated existing memory (similarity {score:.2}): {}",
                            updated.id
                        );
                        if !warnings.is_empty() {
                            msg.push_str(&format!(
                                "\n⚠️ [Hyphae: Possible secrets detected in stored memory: {}. \
                                 Consider using hyphae_memory_forget to remove if these are real credentials.]",
                                warnings.join(", ")
                            ));
                        }
                        ToolResult::text(msg)
                    };
                }
            }
        }
    }

    match store.store(memory) {
        Ok(id) => {
            let warnings = detect_secrets(content);
            if compact {
                if !warnings.is_empty() {
                    ToolResult::text(format!(
                        "ok:{}\n⚠️ Possible secrets detected: {}. Consider using hyphae_memory_forget to remove.",
                        id,
                        warnings.join(", ")
                    ))
                } else {
                    ToolResult::text(format!("ok:{id}"))
                }
            } else {
                let mut hint =
                    super::maintenance::consolidation_hint(store, consolidation, topic, project);

                if !warnings.is_empty() {
                    hint.push_str(&format!(
                        "\n⚠️ [Hyphae: Possible secrets detected in stored memory: {}. \
                         Consider using hyphae_memory_forget to remove if these are real credentials.]",
                        warnings.join(", ")
                    ));
                }

                ToolResult::text(format!("Stored memory: {id}{hint}"))
            }
        }
        Err(e) => ToolResult::error(format!("failed to store: {e}")),
    }
}

pub(crate) fn tool_forget(store: &SqliteStore, args: &Value) -> ToolResult {
    let id = match get_str(args, "id") {
        Some(id) => id,
        None => return ToolResult::error("missing required field: id".into()),
    };

    let memory_id = MemoryId::from(id);
    match store.delete(&memory_id) {
        Ok(()) => ToolResult::text(format!("Deleted memory: {id}")),
        Err(e) => ToolResult::error(format!("failed to delete: {e}")),
    }
}

pub(crate) fn tool_invalidate(store: &SqliteStore, args: &Value) -> ToolResult {
    let id = match get_str(args, "id") {
        Some(id) => id,
        None => return ToolResult::error("missing required field: id".into()),
    };
    let reason = get_str(args, "reason");
    if let Some(reason) = reason {
        if let Err(e) = validate_max_length(reason, "reason", 1024) {
            return e;
        }
    }
    let superseded_by = get_str(args, "superseded_by").map(MemoryId::from);

    let memory_id = MemoryId::from(id);
    match store.invalidate(&memory_id, reason, superseded_by.as_ref()) {
        Ok(()) => {
            let mut output = format!("Invalidated memory: {id}");
            if let Some(reason) = reason {
                output.push_str(&format!("\nreason: {reason}"));
            }
            if let Some(superseded_by) = superseded_by {
                output.push_str(&format!("\nsuperseded_by: {superseded_by}"));
            }
            ToolResult::text(output)
        }
        Err(e) => ToolResult::error(format!("failed to invalidate: {e}")),
    }
}

pub(crate) fn tool_update(
    store: &SqliteStore,
    embedder: Option<&dyn Embedder>,
    args: &Value,
) -> ToolResult {
    let id = match get_str(args, "id") {
        Some(id) => id,
        None => return ToolResult::error("missing required field: id".into()),
    };
    let content = match validate_required_string(args, "content") {
        Ok(c) => c,
        Err(e) => return e,
    };
    if let Err(e) = validate_max_length(content, "content", 32768) {
        return e;
    }

    let memory_id = MemoryId::from(id);
    let mut memory = match store.get(&memory_id) {
        Ok(Some(m)) => m,
        Ok(None) => return ToolResult::error(format!("memory not found: {id}")),
        Err(e) => return ToolResult::error(format!("db error: {e}")),
    };

    memory.summary = content.to_string();
    memory.updated_at = Utc::now();
    memory.weight = Weight::default();

    if let Some(imp_str) = get_str(args, "importance") {
        if let Ok(imp) = imp_str.parse() {
            memory.importance = imp;
        }
    }

    if let Some(keywords_arr) = args.get("keywords").and_then(|v| v.as_array()) {
        memory.keywords = keywords_arr
            .iter()
            .filter_map(|v| v.as_str().map(String::from))
            .collect();
    }

    if let Some(emb) = embedder {
        let text = format!("{} {}", memory.topic, content);
        if let Ok(vec) = emb.embed(&text) {
            memory.embedding = Some(vec);
        }
    }

    match store.update(&memory) {
        Ok(()) => ToolResult::text(format!("Updated memory: {id}")),
        Err(e) => ToolResult::error(format!("failed to update: {e}")),
    }
}
