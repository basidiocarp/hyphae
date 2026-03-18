use chrono::Utc;
use serde_json::Value;

use hyphae_core::{Embedder, Importance, Memory, MemoryId, MemoryStore, Weight};
use hyphae_store::SqliteStore;
use hyphae_store::context;

use crate::protocol::ToolResult;

use super::{get_bounded_i64, get_str, validate_max_length, validate_required_string};

pub(crate) fn tool_store(
    store: &SqliteStore,
    embedder: Option<&dyn Embedder>,
    args: &Value,
    compact: bool,
    project: Option<&str>,
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

    // Auto-embed if embedder is available, reuse for dedup check
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

    if let Some(raw) = get_str(args, "raw_excerpt") {
        builder = builder.raw_excerpt(raw.into());
    }

    if let Some(ref vec) = embedding {
        builder = builder.embedding(vec.clone());
    }

    let memory = builder.build();

    // Dedup check: if a very similar memory exists in the same topic, update it instead
    if let Some(query_emb) = embedding {
        let text = format!("{topic} {content}");
        if let Ok(similar) = store.search_hybrid(&text, &query_emb, 1, 0, project) {
            if let Some((existing, score)) = similar.first() {
                if score > &0.85 && existing.topic == topic {
                    // Very similar content in same topic — update instead of duplicate
                    let mut updated = existing.clone();
                    updated.summary = content.to_string();
                    updated.updated_at = Utc::now();
                    updated.weight = Weight::default(); // Reset weight on update
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
                    return if compact {
                        ToolResult::text(format!("ok:{}", updated.id))
                    } else {
                        ToolResult::text(format!(
                            "Updated existing memory (similarity {score:.2}): {}",
                            updated.id
                        ))
                    };
                }
            }
        }
    }

    match store.store(memory) {
        Ok(id) => {
            if compact {
                ToolResult::text(format!("ok:{id}"))
            } else {
                // Check if topic needs consolidation
                let hint = if let Ok(count) = store.count_by_topic(topic, project) {
                    if count > 7 {
                        format!(
                            "\n⚠ Topic '{topic}' has {count} entries — consider consolidating with hyphae_memory_consolidate."
                        )
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                };
                ToolResult::text(format!("Stored memory: {id}{hint}"))
            }
        }
        Err(e) => ToolResult::error(format!("failed to store: {e}")),
    }
}

pub(crate) fn tool_recall(
    store: &SqliteStore,
    embedder: Option<&dyn Embedder>,
    args: &Value,
    compact: bool,
    project: Option<&str>,
) -> ToolResult {
    // Auto-decay if >24h since last decay
    if let Err(e) = store.maybe_auto_decay() {
        tracing::warn!("auto-decay failed: {e}");
    }

    let query = match get_str(args, "query") {
        Some(q) => q,
        None => return ToolResult::error("missing required field: query".into()),
    };
    let limit = get_bounded_i64(args, "limit", 5, 1, 100) as usize;
    let offset = get_bounded_i64(args, "offset", 0, 0, 10000) as usize;
    let topic = get_str(args, "topic");
    let keyword = get_str(args, "keyword");
    let code_context = args
        .get("code_context")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Try hybrid search if embedder is available
    if let Some(emb) = embedder {
        if let Ok(query_emb) = emb.embed(query) {
            if let Ok(results) = store.search_hybrid(query, &query_emb, limit, offset, project) {
                let mut scored_results = results;

                // Merge _shared results when searching a specific project
                scored_results = merge_shared_hybrid(
                    store,
                    query,
                    &query_emb,
                    limit,
                    offset,
                    project,
                    scored_results,
                );

                if let Some(t) = topic {
                    scored_results.retain(|(m, _)| m.topic == t);
                }
                if let Some(kw) = keyword {
                    scored_results.retain(|(m, _)| m.keywords.iter().any(|k| k.contains(kw)));
                }

                // Update access counts
                for (mem, _) in &scored_results {
                    if let Err(e) = store.update_access(&mem.id) {
                        tracing::warn!("update_access failed: {e}");
                    }
                }

                if scored_results.is_empty() {
                    return ToolResult::text("No memories found.".into());
                }

                let mut output = String::new();
                if compact {
                    for (mem, _) in &scored_results {
                        output.push_str(&format!("[{}] {}\n", mem.topic, mem.summary));
                    }
                } else {
                    for (mem, score) in &scored_results {
                        output.push_str(&format!(
                    "--- {} [score: {:.3}] ---\n  topic: {}\n  importance: {}\n  weight: {:.3}\n  summary: {}\n",
                    mem.id, score, mem.topic, mem.importance, mem.weight.value(), mem.summary
                ));
                        if !mem.keywords.is_empty() {
                            output.push_str(&format!("  keywords: {}\n", mem.keywords.join(", ")));
                        }
                        if let Some(ref p) = mem.project {
                            output.push_str(&format!("  project: {p}\n"));
                        }
                        if let Some(ref raw) = mem.raw_excerpt {
                            output.push_str(&format!("  raw: {raw}\n"));
                        }
                        output.push('\n');
                    }
                }
                return ToolResult::text(output);
            }
        }
    }

    // Fallback: FTS then keywords
    let mut results = match store.search_fts(query, limit, offset, project) {
        Ok(r) => r,
        Err(e) => return ToolResult::error(format!("search error: {e}")),
    };

    if results.is_empty() {
        let keywords: Vec<&str> = query.split_whitespace().collect();
        results = match store.search_by_keywords(&keywords, limit, offset, project) {
            Ok(r) => r,
            Err(e) => return ToolResult::error(format!("search error: {e}")),
        };
    }

    // Merge _shared results when searching a specific project
    results = merge_shared_fts(store, query, limit, project, results);

    // Session-aware recall boost: when the query mentions sessions,
    // prepend matching session/* memories so recent session context surfaces first.
    if is_session_query(query) {
        let session_limit = 5usize.min(limit);
        if let Ok(session_hits) = store.search_fts(query, session_limit * 4, 0, project) {
            let existing_ids: std::collections::HashSet<String> =
                results.iter().map(|m| m.id.to_string()).collect();
            let session_mems: Vec<_> = session_hits
                .into_iter()
                .filter(|m| {
                    m.topic.starts_with("session/") && !existing_ids.contains(&m.id.to_string())
                })
                .take(session_limit)
                .collect();
            if !session_mems.is_empty() {
                let mut boosted = session_mems;
                boosted.append(&mut results);
                results = boosted;
                results.truncate(limit);
            }
        }
    }

    // Optional code-context expansion: additional FTS pass with expanded terms
    if code_context {
        if let Some(expand_project) = project {
            if context::is_code_related(query) {
                let extra_terms = context::expand_with_code_context(store, query, expand_project);
                if !extra_terms.is_empty() {
                    // Build single expanded query and run a second FTS pass
                    let expanded_query = extra_terms
                        .iter()
                        .map(|t| {
                            // Quote each token for FTS
                            let clean: String = t
                                .chars()
                                .map(|c| {
                                    if matches!(
                                        c,
                                        '-' | '*'
                                            | '"'
                                            | '('
                                            | ')'
                                            | '{'
                                            | '}'
                                            | ':'
                                            | '^'
                                            | '+'
                                            | '~'
                                            | '\\'
                                    ) {
                                        ' '
                                    } else {
                                        c
                                    }
                                })
                                .collect();
                            let tokens: Vec<String> = clean
                                .split_whitespace()
                                .filter(|w| !w.is_empty())
                                .map(|w| format!("\"{w}\""))
                                .collect();
                            tokens.join(" ")
                        })
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>()
                        .join(" OR ");

                    if !expanded_query.is_empty() {
                        if let Ok(expanded) =
                            store.search_fts(&expanded_query, limit, offset, project)
                        {
                            // Merge: append expanded results not already present
                            let existing_ids: std::collections::HashSet<String> =
                                results.iter().map(|m| m.id.to_string()).collect();
                            for mem in expanded {
                                if !existing_ids.contains(&mem.id.to_string()) {
                                    results.push(mem);
                                }
                            }
                            // Re-limit after merge
                            results.truncate(limit);
                        }
                    }
                }
            }
        }
    }

    if let Some(t) = topic {
        results.retain(|m| m.topic == t);
    }
    if let Some(kw) = keyword {
        results.retain(|m| m.keywords.iter().any(|k| k.contains(kw)));
    }

    // Update access counts
    for mem in &results {
        if let Err(e) = store.update_access(&mem.id) {
            tracing::warn!("update_access failed: {e}");
        }
    }

    if results.is_empty() {
        return ToolResult::text("No memories found.".into());
    }

    let mut output = String::new();
    if compact {
        for mem in &results {
            output.push_str(&format!("[{}] {}\n", mem.topic, mem.summary));
        }
    } else {
        for mem in &results {
            output.push_str(&format!(
                "--- {} ---\n  topic: {}\n  importance: {}\n  weight: {:.3}\n  summary: {}\n",
                mem.id,
                mem.topic,
                mem.importance,
                mem.weight.value(),
                mem.summary
            ));
            if !mem.keywords.is_empty() {
                output.push_str(&format!("  keywords: {}\n", mem.keywords.join(", ")));
            }
            if let Some(ref raw) = mem.raw_excerpt {
                output.push_str(&format!("  raw: {raw}\n"));
            }
            output.push('\n');
        }
    }

    ToolResult::text(output)
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

pub(crate) fn tool_consolidate(store: &SqliteStore, args: &Value) -> ToolResult {
    let topic = match validate_required_string(args, "topic") {
        Ok(t) => t,
        Err(e) => return e,
    };
    let summary = match validate_required_string(args, "summary") {
        Ok(s) => s,
        Err(e) => return e,
    };
    if let Err(e) = validate_max_length(summary, "summary", 32768) {
        return e;
    }

    let consolidated = Memory::new(topic.into(), summary.into(), Importance::High);

    match store.consolidate_topic(topic, consolidated) {
        Ok(()) => ToolResult::text(format!("Consolidated topic: {topic}")),
        Err(e) => ToolResult::error(format!("failed to consolidate: {e}")),
    }
}

pub(crate) fn tool_list_topics(store: &SqliteStore, project: Option<&str>) -> ToolResult {
    match store.list_topics(project) {
        Ok(topics) => {
            if topics.is_empty() {
                return ToolResult::text("No topics yet.".into());
            }
            let mut output = String::from("Topics:\n");
            for (topic, count) in &topics {
                output.push_str(&format!("  {topic}: {count} memories\n"));
            }
            ToolResult::text(output)
        }
        Err(e) => ToolResult::error(format!("failed to list topics: {e}")),
    }
}

pub(crate) fn tool_stats(store: &SqliteStore, project: Option<&str>) -> ToolResult {
    match store.stats(project) {
        Ok(stats) => {
            let mut output = format!(
                "Memories: {}\nTopics: {}\nAvg weight: {:.3}\n",
                stats.total_memories, stats.total_topics, stats.avg_weight
            );
            if let Some(oldest) = stats.oldest_memory {
                output.push_str(&format!("Oldest: {}\n", oldest.format("%Y-%m-%d %H:%M")));
            }
            if let Some(newest) = stats.newest_memory {
                output.push_str(&format!("Newest: {}\n", newest.format("%Y-%m-%d %H:%M")));
            }
            ToolResult::text(output)
        }
        Err(e) => ToolResult::error(format!("failed to get stats: {e}")),
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
    memory.weight = Weight::default(); // Reset weight on update (refreshed content)

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

    // Re-embed if embedder available
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

pub(crate) fn tool_health(store: &SqliteStore, args: &Value, project: Option<&str>) -> ToolResult {
    let specific_topic = get_str(args, "topic");

    let topics = if let Some(t) = specific_topic {
        vec![(t.to_string(), 0usize)]
    } else {
        match store.list_topics(project) {
            Ok(t) => t,
            Err(e) => return ToolResult::error(format!("failed to list topics: {e}")),
        }
    };

    if topics.is_empty() {
        return ToolResult::text("No topics yet.".into());
    }

    let mut output = String::from("Memory Health Report:\n\n");
    let mut total_stale = 0usize;
    let mut topics_needing_consolidation = 0usize;

    for (topic, _) in &topics {
        match store.topic_health(topic, project) {
            Ok(health) => {
                let status = if health.needs_consolidation && health.stale_count > 0 {
                    "⚠ NEEDS ATTENTION"
                } else if health.needs_consolidation {
                    "⚠ consolidate"
                } else if health.stale_count > 0 {
                    "○ has stale entries"
                } else {
                    "✓ healthy"
                };

                output.push_str(&format!(
                    "  {topic}: {status}\n    entries: {}  avg_weight: {:.2}  stale: {}  avg_access: {:.1}\n",
                    health.entry_count, health.avg_weight, health.stale_count, health.avg_access_count
                ));

                if health.needs_consolidation {
                    topics_needing_consolidation += 1;
                }
                total_stale += health.stale_count;
            }
            Err(_) => {
                output.push_str(&format!("  {topic}: (error reading)\n"));
            }
        }
    }

    output.push_str(&format!(
        "\nSummary: {} topics, {} need consolidation, {} stale entries total\n",
        topics.len(),
        topics_needing_consolidation,
        total_stale
    ));

    ToolResult::text(output)
}

pub(crate) fn tool_embed_all(
    store: &SqliteStore,
    embedder: Option<&dyn Embedder>,
    args: &Value,
    project: Option<&str>,
) -> ToolResult {
    let embedder = match embedder {
        Some(e) => e,
        None => return ToolResult::error("embeddings not available".into()),
    };

    let topic_filter = get_str(args, "topic");

    // Get all memories, filtered by topic if specified
    let memories = if let Some(t) = topic_filter {
        match store.get_by_topic(t, project) {
            Ok(m) => m,
            Err(e) => return ToolResult::error(format!("failed to list memories: {e}")),
        }
    } else {
        let topics = match store.list_topics(project) {
            Ok(t) => t,
            Err(e) => return ToolResult::error(format!("failed to list topics: {e}")),
        };
        let mut all = Vec::new();
        for (t, _) in &topics {
            if let Ok(mems) = store.get_by_topic(t, project) {
                all.extend(mems);
            }
        }
        all
    };

    // Filter to only those without embeddings
    let to_embed: Vec<&Memory> = memories.iter().filter(|m| m.embedding.is_none()).collect();

    if to_embed.is_empty() {
        return ToolResult::text("All memories already have embeddings.".into());
    }

    let total = to_embed.len();
    let mut embedded = 0;
    let mut errors = 0;

    for mem in &to_embed {
        let text = format!("{} {}", mem.topic, mem.summary);
        match embedder.embed(&text) {
            Ok(vec) => {
                let mut updated = (*mem).clone();
                updated.embedding = Some(vec);
                if store.update(&updated).is_ok() {
                    embedded += 1;
                } else {
                    errors += 1;
                }
            }
            Err(_) => errors += 1,
        }
    }

    ToolResult::text(format!(
        "Embedded {embedded}/{total} memories ({errors} errors)"
    ))
}

/// Detect whether a recall query is asking about session history.
pub(crate) fn is_session_query(query: &str) -> bool {
    let lower = query.to_lowercase();
    const SESSION_KEYWORDS: &[&str] = &[
        "session",
        "last time",
        "previous",
        "yesterday",
        "earlier today",
    ];
    SESSION_KEYWORDS.iter().any(|kw| lower.contains(kw))
}

// ─────────────────────────────────────────────────────────────────────────────
// Cross-project shared knowledge helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Weight multiplier for _shared results relative to project-scoped results.
const SHARED_WEIGHT: f32 = 0.7;

/// Merge _shared results into hybrid search results when the caller is
/// searching a specific project (not `_shared` itself, not global).
fn merge_shared_hybrid(
    store: &SqliteStore,
    query: &str,
    query_emb: &[f32],
    limit: usize,
    offset: usize,
    project: Option<&str>,
    mut scored_results: Vec<(Memory, f32)>,
) -> Vec<(Memory, f32)> {
    let should_merge = matches!(project, Some(p) if p != hyphae_store::SHARED_PROJECT);
    if !should_merge {
        return scored_results;
    }

    let shared = store.search_hybrid(
        query,
        query_emb,
        limit,
        offset,
        Some(hyphae_store::SHARED_PROJECT),
    );
    if let Ok(shared_results) = shared {
        let existing_ids: std::collections::HashSet<String> = scored_results
            .iter()
            .map(|(m, _)| m.id.to_string())
            .collect();
        for (mem, score) in shared_results {
            if !existing_ids.contains(&mem.id.to_string()) {
                scored_results.push((mem, score * SHARED_WEIGHT));
            }
        }
        scored_results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored_results.truncate(limit);
    }
    scored_results
}

/// Merge _shared results into FTS search results when the caller is
/// searching a specific project (not `_shared` itself, not global).
fn merge_shared_fts(
    store: &SqliteStore,
    query: &str,
    limit: usize,
    project: Option<&str>,
    mut results: Vec<Memory>,
) -> Vec<Memory> {
    let should_merge = matches!(project, Some(p) if p != hyphae_store::SHARED_PROJECT);
    if !should_merge {
        return results;
    }

    let shared = store.search_fts(query, limit, 0, Some(hyphae_store::SHARED_PROJECT));
    if let Ok(shared_results) = shared {
        let existing_ids: std::collections::HashSet<String> =
            results.iter().map(|m| m.id.to_string()).collect();
        for mem in shared_results {
            if !existing_ids.contains(&mem.id.to_string()) {
                results.push(mem);
            }
        }
        results.truncate(limit);
    }
    results
}

// ─────────────────────────────────────────────────────────────────────────────
// hyphae_recall_global MCP tool
// ─────────────────────────────────────────────────────────────────────────────

/// Search all projects and return results grouped by project.
pub(crate) fn tool_recall_global(store: &SqliteStore, args: &Value, compact: bool) -> ToolResult {
    let query = match get_str(args, "query") {
        Some(q) => q,
        None => return ToolResult::error("missing required field: query".into()),
    };
    let limit = get_bounded_i64(args, "limit", 10, 1, 100) as usize;

    let results = match store.search_all_projects(query, limit) {
        Ok(r) => r,
        Err(e) => return ToolResult::error(format!("search error: {e}")),
    };

    if results.is_empty() {
        return ToolResult::text("No memories found across any project.".into());
    }

    // Update access counts
    for mem in &results {
        if let Err(e) = store.update_access(&mem.id) {
            tracing::warn!("update_access failed: {e}");
        }
    }

    // Group by project
    let mut by_project: std::collections::BTreeMap<String, Vec<&Memory>> =
        std::collections::BTreeMap::new();
    for mem in &results {
        let project_name = mem.project.as_deref().unwrap_or("(none)").to_string();
        by_project.entry(project_name).or_default().push(mem);
    }

    let mut output = String::new();
    if compact {
        for (project_name, mems) in &by_project {
            output.push_str(&format!("[{project_name}]\n"));
            for mem in mems {
                output.push_str(&format!("  [{}] {}\n", mem.topic, mem.summary));
            }
        }
    } else {
        for (project_name, mems) in &by_project {
            output.push_str(&format!(
                "== Project: {project_name} ({} results) ==\n",
                mems.len()
            ));
            for mem in mems {
                output.push_str(&format!(
                    "  --- {} ---\n    topic: {}\n    importance: {}\n    weight: {:.3}\n    summary: {}\n",
                    mem.id, mem.topic, mem.importance, mem.weight.value(), mem.summary
                ));
                if !mem.keywords.is_empty() {
                    output.push_str(&format!("    keywords: {}\n", mem.keywords.join(", ")));
                }
                output.push('\n');
            }
        }
    }

    ToolResult::text(output)
}
