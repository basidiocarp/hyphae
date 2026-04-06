use chrono::Utc;
use serde_json::Value;

use hyphae_core::{
    ConsolidationConfig, Embedder, Importance, MemoirStore, Memory, MemoryId, MemoryStore, Weight,
    detect_git_context_from, detect_secrets,
};
use hyphae_store::context;
use hyphae_store::{SqliteStore, collect_evaluation_window};

use crate::protocol::ToolResult;

use super::{
    get_bounded_i64, get_str, normalize_identity, scoped_worktree_root, validate_max_length,
    validate_required_string,
};

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Safely truncate a string at a byte boundary, respecting multi-byte UTF-8.
fn truncate_str(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

// ─────────────────────────────────────────────────────────────────────────────
// Gap 10: Age indicator for stale memory feedback
// ─────────────────────────────────────────────────────────────────────────────

const STALE_DAYS_THRESHOLD: i64 = 30;

fn age_indicator(mem: &Memory) -> Option<String> {
    let days = (Utc::now() - mem.last_accessed).num_days();
    if days >= STALE_DAYS_THRESHOLD {
        Some(format!(
            "  ⚠ last accessed {days}d ago — if outdated, use hyphae_memory_update to correct\n"
        ))
    } else {
        None
    }
}

fn log_recall_results(
    store: &SqliteStore,
    query: &str,
    memory_ids: &[String],
    explicit_session_id: Option<&str>,
    project: Option<&str>,
) {
    let session_id = explicit_session_id.map(ToOwned::to_owned).or_else(|| {
        project.and_then(|name| match store.active_session_id(name) {
            Ok(session) => session,
            Err(e) => {
                tracing::warn!("active_session_id failed: {e}");
                None
            }
        })
    });

    if let Err(e) = store.log_recall_event(session_id.as_deref(), query, memory_ids, project) {
        tracing::warn!("log_recall_event failed: {e}");
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct RecallHeuristics {
    session_query: bool,
    code_related: bool,
    code_expansion_terms: Vec<String>,
}

impl RecallHeuristics {
    fn detect(
        store: &SqliteStore,
        query: &str,
        project: Option<&str>,
        code_context_requested: bool,
    ) -> Self {
        let session_query = is_session_query(query);
        let code_related = context::is_code_related(query);
        let code_expansion_terms = if code_context_requested && code_related {
            project
                .map(|project| context::expand_with_code_context(store, query, project))
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        Self {
            session_query,
            code_related,
            code_expansion_terms,
        }
    }

    fn prefer_context_aware_recall(&self) -> bool {
        self.session_query || !self.code_expansion_terms.is_empty()
    }
}

fn search_primary_fts(
    store: &SqliteStore,
    query: &str,
    limit: usize,
    offset: usize,
    project: Option<&str>,
    scoped_worktree: Option<&str>,
) -> Result<Vec<Memory>, ToolResult> {
    let mut results = match if let Some(worktree) = scoped_worktree {
        store.search_fts_scoped(query, limit, offset, project, Some(worktree))
    } else {
        store.search_fts(query, limit, offset, project)
    } {
        Ok(r) => r,
        Err(e) => return Err(ToolResult::error(format!("search error: {e}"))),
    };

    if results.is_empty() {
        let keywords: Vec<&str> = query.split_whitespace().collect();
        results = match if let Some(worktree) = scoped_worktree {
            store.search_by_keywords_scoped(&keywords, limit, offset, project, Some(worktree))
        } else {
            store.search_by_keywords(&keywords, limit, offset, project)
        } {
            Ok(r) => r,
            Err(e) => return Err(ToolResult::error(format!("search error: {e}"))),
        };
    }

    Ok(results)
}

fn collect_session_candidates(
    store: &SqliteStore,
    query: &str,
    limit: usize,
    project: Option<&str>,
    scoped_worktree: Option<&str>,
) -> Vec<Memory> {
    let session_limit = 5usize.min(limit.max(1));
    let session_hits = if let Some(worktree) = scoped_worktree {
        store.search_fts_scoped(query, session_limit * 4, 0, project, Some(worktree))
    } else {
        store.search_fts(query, session_limit * 4, 0, project)
    };

    match session_hits {
        Ok(session_hits) => session_hits
            .into_iter()
            .filter(|m| m.topic.starts_with("session/"))
            .take(session_limit)
            .collect(),
        Err(e) => {
            tracing::warn!("context-aware recall session search failed: {e}");
            Vec::new()
        }
    }
}

fn collect_code_context_candidates(
    store: &SqliteStore,
    extra_terms: &[String],
    limit: usize,
    offset: usize,
    project: Option<&str>,
    scoped_worktree: Option<&str>,
) -> Vec<Memory> {
    if extra_terms.is_empty() {
        return Vec::new();
    }

    let branch_limit = limit.saturating_mul(4).max(4);
    let mut results = Vec::new();
    let mut existing_ids: std::collections::HashSet<String> = std::collections::HashSet::new();

    for term in extra_terms {
        let expanded = if let Some(worktree) = scoped_worktree {
            store.search_fts_scoped(term, branch_limit, offset, project, Some(worktree))
        } else {
            store.search_fts(term, branch_limit, offset, project)
        };

        if let Ok(expanded) = expanded {
            for mem in expanded {
                let mem_id = mem.id.to_string();
                if existing_ids.insert(mem_id) {
                    results.push(mem);
                }
                if results.len() >= branch_limit {
                    break;
                }
            }
        }

        if results.len() >= branch_limit {
            break;
        }
    }

    results.truncate(branch_limit);
    results
}

fn collect_shared_candidates(
    store: &SqliteStore,
    query: &str,
    limit: usize,
    project: Option<&str>,
    _scoped_worktree: Option<&str>,
) -> Vec<Memory> {
    let should_merge = matches!(project, Some(p) if p != hyphae_store::SHARED_PROJECT);
    if !should_merge {
        return Vec::new();
    }

    let shared = store.search_fts(
        query,
        limit.saturating_mul(4).max(4),
        0,
        Some(hyphae_store::SHARED_PROJECT),
    );
    match shared {
        Ok(shared_results) => shared_results,
        Err(e) => {
            tracing::warn!("context-aware recall shared search failed: {e}");
            Vec::new()
        }
    }
}

fn merge_prioritized_candidates(branches: Vec<Vec<Memory>>, limit: usize) -> Vec<Memory> {
    let mut results = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for branch in branches {
        for mem in branch {
            if seen.insert(mem.id.to_string()) {
                results.push(mem);
            }
        }
    }

    results.truncate(limit);
    results
}

fn topic_needs_consolidation(
    store: &SqliteStore,
    consolidation: &ConsolidationConfig,
    topic: &str,
    project: Option<&str>,
) -> bool {
    let Some(threshold) = consolidation.threshold_for_topic(topic) else {
        return false;
    };

    match store.count_by_topic(topic, project) {
        Ok(count) => count >= threshold,
        Err(e) => {
            tracing::warn!("count_by_topic failed for consolidation check: {e}");
            false
        }
    }
}

fn consolidation_hint(
    store: &SqliteStore,
    consolidation: &ConsolidationConfig,
    topic: &str,
    project: Option<&str>,
) -> String {
    if consolidation.is_exempt(topic) {
        return String::new();
    }

    let Some(threshold) = consolidation.threshold_for_topic(topic) else {
        return String::new();
    };

    match store.count_by_topic(topic, project) {
        Ok(count) if count >= threshold => format!(
            "\n⚠ Topic '{topic}' has {count} entries — consider consolidating with hyphae_memory_consolidate."
        ),
        Ok(_) => String::new(),
        Err(_) => String::new(),
    }
}

fn run_context_aware_recall(
    store: &SqliteStore,
    query: &str,
    limit: usize,
    offset: usize,
    project: Option<&str>,
    scoped_worktree: Option<&str>,
    heuristics: &RecallHeuristics,
) -> Result<Vec<Memory>, ToolResult> {
    let mut branches = Vec::new();

    if heuristics.session_query {
        branches.push(collect_session_candidates(
            store,
            query,
            limit,
            project,
            scoped_worktree,
        ));
    }

    if !heuristics.code_expansion_terms.is_empty() {
        branches.push(collect_code_context_candidates(
            store,
            &heuristics.code_expansion_terms,
            limit,
            offset,
            project,
            scoped_worktree,
        ));
    }

    branches.push(search_primary_fts(
        store,
        query,
        limit,
        offset,
        project,
        scoped_worktree,
    )?);

    if matches!(project, Some(p) if p != hyphae_store::SHARED_PROJECT) {
        branches.push(collect_shared_candidates(
            store,
            query,
            limit,
            project,
            scoped_worktree,
        ));
    }

    Ok(merge_prioritized_candidates(branches, limit))
}

pub(crate) fn tool_store(
    store: &SqliteStore,
    embedder: Option<&dyn Embedder>,
    consolidation: &ConsolidationConfig,
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

    // ─────────────────────────────────────────────────────────────────────────────
    // Secrets Rejection (if enabled)
    // ─────────────────────────────────────────────────────────────────────────────
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
                // Check if topic needs consolidation
                let mut hint = consolidation_hint(store, consolidation, topic, project);

                // Add secrets warning if detected
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
    let session_id = get_str(args, "session_id");
    let raw_project_root = get_str(args, "project_root");
    let raw_worktree_id = get_str(args, "worktree_id");
    if raw_project_root.is_some() ^ raw_worktree_id.is_some() {
        return ToolResult::error(
            "project_root and worktree_id must be provided together".to_string(),
        );
    }
    let (project_root, worktree_id) = normalize_identity(raw_project_root, raw_worktree_id);
    let scoped_worktree = scoped_worktree_root(project_root, worktree_id);
    let code_context_requested = args
        .get("code_context")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let session_project = if let Some(session_id) = session_id {
        match store.feedback_session_project(session_id, project) {
            Ok(project_name) => Some(project_name),
            Err(e) => return ToolResult::error(format!("invalid session_id: {e}")),
        }
    } else {
        None
    };
    let project = session_project.as_deref().or(project);
    let heuristics = RecallHeuristics::detect(store, query, project, code_context_requested);

    // ─────────────────────────────────────────────────────────────────────────────
    // Compute auto-consolidation hint (if topic filter is used)
    // ─────────────────────────────────────────────────────────────────────────────
    let auto_consolidate_hint = if let Some(t) = topic {
        let memories = if let Some(worktree) = scoped_worktree {
            store.get_by_topic_scoped(t, project, Some(worktree))
        } else {
            store.get_by_topic(t, project)
        };
        if let Ok(memories) = memories {
            if memories.len() > 20 {
                // Check if span >7 days
                let oldest = memories.iter().min_by_key(|m| m.created_at);
                let newest = memories.iter().max_by_key(|m| m.created_at);
                if let (Some(old_mem), Some(new_mem)) = (oldest, newest) {
                    let span_days = (new_mem.created_at - old_mem.created_at).num_days();
                    if span_days > 7 {
                        Some(format!(
                            "\n[Hyphae: Topic \"{t}\" has {} memories spanning {span_days} days. \
                             Auto-consolidation recommended. Run hyphae_memory_consolidate(topic: \"{t}\") \
                             to merge redundant entries.]",
                            memories.len()
                        ))
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    // Try hybrid search if embedder is available
    if let Some(emb) = embedder.filter(|_| !heuristics.prefer_context_aware_recall()) {
        if let Ok(query_emb) = emb.embed(query) {
            let hybrid_results = if let Some(worktree) = scoped_worktree {
                store.search_hybrid_scoped(
                    query,
                    &query_emb,
                    limit,
                    offset,
                    project,
                    Some(worktree),
                )
            } else {
                store.search_hybrid(query, &query_emb, limit, offset, project)
            };
            if let Ok(results) = hybrid_results {
                let mut scored_results = results;

                // Merge _shared results when searching a specific project
                scored_results = merge_shared_hybrid(
                    store,
                    query,
                    &query_emb,
                    limit,
                    offset,
                    project,
                    scoped_worktree,
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

                let memory_ids: Vec<String> = scored_results
                    .iter()
                    .map(|(mem, _)| mem.id.to_string())
                    .collect();
                log_recall_results(store, query, &memory_ids, session_id, project);

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
                        if let Some(ref branch) = mem.branch {
                            output.push_str(&format!("  branch: {branch}\n"));
                        }
                        if let Some(ref worktree) = mem.worktree {
                            output.push_str(&format!("  worktree: {worktree}\n"));
                        }
                        if let Some(ref raw) = mem.raw_excerpt {
                            output.push_str(&format!("  raw: {raw}\n"));
                        }
                        if let Some(age) = age_indicator(mem) {
                            output.push_str(&age);
                        }
                        output.push('\n');
                    }
                }
                if let Some(hint) = auto_consolidate_hint.as_ref() {
                    output.push_str(hint);
                }
                return ToolResult::text(output);
            }
        }
    }

    let mut results = match run_context_aware_recall(
        store,
        query,
        limit,
        offset,
        project,
        scoped_worktree,
        &heuristics,
    ) {
        Ok(results) => results,
        Err(err) => return err,
    };

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

    let memory_ids: Vec<String> = results.iter().map(|mem| mem.id.to_string()).collect();
    log_recall_results(store, query, &memory_ids, session_id, project);

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
            if let Some(ref p) = mem.project {
                output.push_str(&format!("  project: {p}\n"));
            }
            if let Some(ref branch) = mem.branch {
                output.push_str(&format!("  branch: {branch}\n"));
            }
            if let Some(ref worktree) = mem.worktree {
                output.push_str(&format!("  worktree: {worktree}\n"));
            }
            if let Some(ref raw) = mem.raw_excerpt {
                output.push_str(&format!("  raw: {raw}\n"));
            }
            if let Some(age) = age_indicator(mem) {
                output.push_str(&age);
            }
            output.push('\n');
        }
    }

    if let Some(hint) = auto_consolidate_hint.as_ref() {
        output.push_str(hint);
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

pub(crate) fn tool_list_invalidated(
    store: &SqliteStore,
    args: &Value,
    project: Option<&str>,
) -> ToolResult {
    let limit = get_bounded_i64(args, "limit", 20, 1, 100) as usize;
    let offset = get_bounded_i64(args, "offset", 0, 0, 10_000) as usize;

    match store.list_invalidated(limit, offset, project) {
        Ok(memories) => {
            if memories.is_empty() {
                return ToolResult::text("No invalidated memories.".into());
            }

            let mut output = String::from("Invalidated memories:\n");
            for mem in memories {
                output.push_str(&format!("  [{}] [{}] {}\n", mem.id, mem.topic, mem.summary));
                if let Some(reason) = mem.invalidation_reason {
                    output.push_str(&format!("    reason: {reason}\n"));
                }
                if let Some(superseded_by) = mem.superseded_by {
                    output.push_str(&format!("    superseded_by: {superseded_by}\n"));
                }
                if let Some(invalidated_at) = mem.invalidated_at {
                    output.push_str(&format!(
                        "    invalidated_at: {}\n",
                        invalidated_at.format("%Y-%m-%d %H:%M")
                    ));
                }
                if let Some(project) = mem.project {
                    output.push_str(&format!("    project: {project}\n"));
                }
            }

            ToolResult::text(output)
        }
        Err(e) => ToolResult::error(format!("failed to list invalidated memories: {e}")),
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
    let consolidation = ConsolidationConfig::default();
    tool_health_with_rules(store, &consolidation, args, project)
}

pub(crate) fn tool_health_with_rules(
    store: &SqliteStore,
    consolidation: &ConsolidationConfig,
    args: &Value,
    project: Option<&str>,
) -> ToolResult {
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
                let needs_consolidation =
                    topic_needs_consolidation(store, consolidation, topic, project);
                let status = if needs_consolidation && health.stale_count > 0 {
                    "⚠ NEEDS ATTENTION"
                } else if needs_consolidation {
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

                if needs_consolidation {
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
    _worktree: Option<&str>,
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
                if let Some(branch) = &mem.branch {
                    output.push_str(&format!("    branch: {branch}\n"));
                }
                if let Some(worktree) = &mem.worktree {
                    output.push_str(&format!("    worktree: {worktree}\n"));
                }
                output.push('\n');
            }
        }
    }

    ToolResult::text(output)
}

// ─────────────────────────────────────────────────────────────────────────────
// Gap 8: Memory-to-memoir promotion
// ─────────────────────────────────────────────────────────────────────────────

const PROMOTION_THRESHOLD: usize = 15;

/// Suggest promoting a topic's memories into a structured memoir.
/// Lists all memories so the agent can create the memoir with proper concepts.
pub(crate) fn tool_promote_to_memoir(
    store: &SqliteStore,
    args: &Value,
    project: Option<&str>,
) -> ToolResult {
    let topic = match validate_required_string(args, "topic") {
        Ok(t) => t,
        Err(e) => return e,
    };

    let memories = match store.get_by_topic(topic, project) {
        Ok(m) => m,
        Err(e) => return ToolResult::error(format!("failed to read topic: {e}")),
    };

    if memories.is_empty() {
        return ToolResult::text(format!("Topic \"{topic}\" has no memories to promote."));
    }

    if memories.len() < PROMOTION_THRESHOLD {
        return ToolResult::text(format!(
            "Topic \"{topic}\" has {} memories (threshold: {PROMOTION_THRESHOLD}). \
             Not enough to warrant promotion yet.",
            memories.len()
        ));
    }

    // Check if a memoir already exists for this topic
    let memoir_name = topic.replace('/', "-");
    if let Ok(memoirs) = store.list_memoirs() {
        if memoirs.iter().any(|m| m.name == memoir_name) {
            return ToolResult::text(format!(
                "A memoir named \"{memoir_name}\" already exists. \
                 Use hyphae_memoir_refine to update its concepts instead."
            ));
        }
    }

    // Extract keyword frequency to suggest concepts
    let mut keyword_freq: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for mem in &memories {
        for kw in &mem.keywords {
            *keyword_freq.entry(kw.clone()).or_default() += 1;
        }
    }

    let mut top_keywords: Vec<_> = keyword_freq.into_iter().collect();
    top_keywords.sort_by(|a, b| b.1.cmp(&a.1));
    let suggested_concepts: Vec<_> = top_keywords
        .iter()
        .take(10)
        .map(|(k, c)| format!("{k} ({c}x)"))
        .collect();

    let mut output = format!(
        "Topic \"{topic}\" has {} memories ready for promotion to memoir \"{memoir_name}\".\n\n",
        memories.len()
    );

    if !suggested_concepts.is_empty() {
        output.push_str("Suggested concepts (from keywords):\n");
        for c in &suggested_concepts {
            output.push_str(&format!("  - {c}\n"));
        }
        output.push('\n');
    }

    output.push_str("Memory summaries:\n");
    for mem in memories.iter().take(20) {
        let summary = format!("{}...", truncate_str(&mem.summary, 120));
        output.push_str(&format!("  [{}] {summary}\n", mem.importance));
    }

    output.push_str(&format!(
        "\nTo promote, create the memoir and add concepts:\n\
         1. hyphae_memoir_create(name: \"{memoir_name}\", description: \"...\")\n\
         2. hyphae_memoir_add_concept(memoir: \"{memoir_name}\", name: \"<concept>\", definition: \"...\")\n\
         3. hyphae_memoir_link(memoir: \"{memoir_name}\", source: \"...\", target: \"...\", relation: \"...\")\n"
    ));

    ToolResult::text(output)
}

// ─────────────────────────────────────────────────────────────────────────────
// Extract lessons from corrections, resolved errors, and test fixes
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) fn tool_extract_lessons(
    store: &SqliteStore,
    args: &Value,
    project: Option<&str>,
) -> ToolResult {
    let limit = get_bounded_i64(args, "limit", 10, 1, 50) as usize;

    // Read memories from three topics
    let corrections = store
        .get_by_topic("corrections", project)
        .unwrap_or_default();
    let errors_resolved = store
        .get_by_topic("errors/resolved", project)
        .unwrap_or_default();
    let tests_resolved = store
        .get_by_topic("tests/resolved", project)
        .unwrap_or_default();

    let mut all_memories = Vec::new();
    all_memories.extend(corrections.iter().map(|m| ("corrections", m)));
    all_memories.extend(errors_resolved.iter().map(|m| ("errors/resolved", m)));
    all_memories.extend(tests_resolved.iter().map(|m| ("tests/resolved", m)));

    if all_memories.is_empty() {
        return ToolResult::text(
            "No memories found in corrections, errors/resolved, or tests/resolved topics.".into(),
        );
    }

    // Take up to 50 memories total
    all_memories.truncate(50);

    // Group by keyword overlap: build a map of keywords to memories
    let mut keyword_groups: std::collections::HashMap<String, Vec<(&str, &Memory)>> =
        std::collections::HashMap::new();

    for (topic_type, mem) in &all_memories {
        // Combine keywords and extract keywords from summary
        let mut keywords = mem.keywords.clone();
        keywords.extend(extract_keywords(&mem.summary));

        if keywords.is_empty() {
            // If no keywords, use first few words as synthetic keyword
            let words: Vec<&str> = mem.summary.split_whitespace().take(3).collect();
            keywords.push(words.join(" ").to_lowercase());
        }

        for kw in keywords {
            let kw_lower = kw.to_lowercase();
            keyword_groups
                .entry(kw_lower)
                .or_default()
                .push((topic_type, mem));
        }
    }

    // Extract lessons: groups with 2+ entries
    let mut lessons: Vec<String> = Vec::new();

    for (keyword, group_mems) in keyword_groups {
        if group_mems.len() < 2 {
            continue;
        }

        // Count by topic type
        let mut type_counts = std::collections::HashMap::new();
        for (topic_type, _) in &group_mems {
            *type_counts.entry(*topic_type).or_insert(0) += 1;
        }

        // Extract common pattern from summaries
        let summaries: Vec<&str> = group_mems.iter().map(|(_, m)| m.summary.as_str()).collect();
        let pattern = extract_common_pattern(&summaries);

        // Build lesson message based on topic type prevalence
        let lesson = if let Some(count) = type_counts.get("corrections") {
            if *count >= 2 {
                format!(
                    "[corrections] When working with '{}': {} — avoided {} times",
                    keyword, pattern, count
                )
            } else {
                continue;
            }
        } else if let Some(count) = type_counts.get("errors/resolved") {
            format!(
                "[errors] Common issue in '{}': {} — resolved {} times",
                keyword, pattern, count
            )
        } else if let Some(count) = type_counts.get("tests/resolved") {
            format!(
                "[tests] Test failures in '{}': {} — fixed {} times",
                keyword, pattern, count
            )
        } else {
            continue;
        };

        lessons.push(lesson);
    }

    if lessons.is_empty() {
        return ToolResult::text(
            "No patterns found (need 2+ memories per keyword to extract lessons).".into(),
        );
    }

    // Sort and limit
    lessons.sort();
    lessons.truncate(limit);

    let mut output = format!(
        "Lessons extracted from {} corrections, {} error resolutions, {} test fixes:\n\n",
        corrections.len(),
        errors_resolved.len(),
        tests_resolved.len()
    );

    for (i, lesson) in lessons.iter().enumerate() {
        output.push_str(&format!("{}. {}\n", i + 1, lesson));
    }

    output.push_str("\nUse these lessons to avoid repeating past mistakes.\n");

    ToolResult::text(output)
}

// ─────────────────────────────────────────────────────────────────────────────
// Helper functions for lesson extraction
// ─────────────────────────────────────────────────────────────────────────────

/// Extract lowercase keywords from text (words > 3 chars, excluding common words).
fn extract_keywords(text: &str) -> Vec<String> {
    const STOP_WORDS: &[&str] = &[
        "the", "and", "or", "but", "not", "in", "on", "at", "to", "for", "of", "is", "was", "are",
        "be", "been", "being", "have", "has", "had", "do", "does", "did", "will", "would",
        "should", "could", "may", "might", "can", "must", "a", "an", "as", "with", "from", "by",
        "this", "that", "these", "those", "i", "you", "he", "she", "it", "we", "they", "what",
        "which", "who", "when", "where", "why", "how",
    ];

    text.split_whitespace()
        .map(|w| {
            w.chars()
                .filter(|c| c.is_alphanumeric())
                .collect::<String>()
                .to_lowercase()
        })
        .filter(|w| w.len() > 3 && !STOP_WORDS.contains(&w.as_str()))
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect()
}

/// Extract a common pattern from multiple summaries by finding shared phrases.
fn extract_common_pattern(summaries: &[&str]) -> String {
    if summaries.is_empty() {
        return "unknown pattern".to_string();
    }

    if summaries.len() == 1 {
        return summaries[0].to_string();
    }

    // For multiple summaries, extract shared tokens
    let first_tokens: std::collections::HashSet<String> = summaries[0]
        .split_whitespace()
        .map(|w| w.to_lowercase())
        .collect();

    let mut common: Vec<String> = first_tokens
        .into_iter()
        .filter(|token| {
            summaries[1..]
                .iter()
                .all(|s| s.to_lowercase().contains(token))
        })
        .collect();

    if !common.is_empty() {
        common.sort();
        format!("avoid {}", common.join(" "))
    } else {
        // No shared tokens, just show length and first summary
        format!(
            "pattern like '{}'",
            summaries[0].chars().take(50).collect::<String>()
        )
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Evaluation Tool
// ─────────────────────────────────────────────────────────────────────────────

pub(crate) fn tool_evaluate(
    store: &SqliteStore,
    args: &Value,
    project: Option<&str>,
) -> ToolResult {
    let days = get_bounded_i64(args, "days", 14, 2, 365);
    let midpoint = days / 2;
    let previous_window_days = days - midpoint;
    let proj_name = project.unwrap_or("all projects");
    let recent_window = match collect_evaluation_window(store, 0, midpoint, project) {
        Ok(window) => window,
        Err(e) => {
            return ToolResult::error(format!("failed to collect recent evaluation window: {e}"));
        }
    };
    let previous_window = match collect_evaluation_window(store, midpoint, days, project) {
        Ok(window) => window,
        Err(e) => {
            return ToolResult::error(format!("failed to collect previous evaluation window: {e}"));
        }
    };

    // Check if we have enough data
    if recent_window.session_count == 0 && previous_window.session_count == 0 {
        return ToolResult::text(
            "Insufficient data: no sessions found in the evaluation window. \
            Metrics require at least 1 session per window. Try extending --days or checking that structured sessions are being recorded."
                .into(),
        );
    }
    let recent_error_rate = recent_window.error_rate();
    let previous_error_rate = previous_window.error_rate();
    let recent_correction_rate = recent_window.correction_rate();
    let previous_correction_rate = previous_window.correction_rate();
    let recent_resolution_rate = recent_window.resolution_rate();
    let previous_resolution_rate = previous_window.resolution_rate();
    let recent_test_rate = recent_window.test_fix_rate();
    let previous_test_rate = previous_window.test_fix_rate();

    // Calculate trends
    let trend_improving = |prev: f64, recent: f64, lower_is_better: bool| -> (bool, f64) {
        if prev == 0.0 {
            return (false, 0.0);
        }
        let delta = ((recent - prev) / prev).abs();
        let improving = if lower_is_better {
            recent < prev
        } else {
            recent > prev
        };
        (improving, delta * 100.0)
    };

    let (error_improving, error_pct) =
        trend_improving(previous_error_rate, recent_error_rate, true);
    let (correction_improving, correction_pct) =
        trend_improving(previous_correction_rate, recent_correction_rate, true);
    let (resolution_improving, resolution_pct) =
        trend_improving(previous_resolution_rate, recent_resolution_rate, false);
    let (test_improving, test_pct) = trend_improving(previous_test_rate, recent_test_rate, false);

    // Format report
    let mut output = String::new();
    output.push_str(&format!("\nAgent Evaluation Report (last {days} days)\n"));
    output.push_str(&format!("Project: {}\n\n", proj_name));
    output.push_str(&format!(
        "{:<25} {:>14} {:>14} {}\n",
        "Metric",
        format!("Previous {}d", previous_window_days),
        format!("Recent {}d", midpoint),
        "Trend"
    ));
    output.push_str(&format!(
        "{:<25} {:>14} {:>14} {}\n",
        "-".repeat(25),
        "-".repeat(14),
        "-".repeat(14),
        "-".repeat(30)
    ));

    // Error rate
    output.push_str(&format!(
        "{:<25} {:>14.2} {:>14.2} {}\n",
        "Errors per session",
        previous_error_rate,
        recent_error_rate,
        if error_improving {
            format!("↓ {:.0}% better", error_pct)
        } else {
            format!("↑ {:.0}% worse", error_pct)
        }
    ));

    // Correction rate
    output.push_str(&format!(
        "{:<25} {:>14.2} {:>14.2} {}\n",
        "Self-corrections/session",
        previous_correction_rate,
        recent_correction_rate,
        if correction_improving {
            format!("↓ {:.0}% better", correction_pct)
        } else {
            format!("↑ {:.0}% worse", correction_pct)
        }
    ));

    // Resolution rate
    output.push_str(&format!(
        "{:<25} {:>13.0}% {:>13.0}% {}\n",
        "Error resolution rate",
        previous_resolution_rate * 100.0,
        recent_resolution_rate * 100.0,
        if resolution_improving {
            format!("↑ {:.0}% better", resolution_pct)
        } else {
            format!("↓ {:.0}% worse", resolution_pct)
        }
    ));

    // Test fix rate
    output.push_str(&format!(
        "{:<25} {:>13.0}% {:>13.0}% {}\n",
        "Test fix rate",
        previous_test_rate * 100.0,
        recent_test_rate * 100.0,
        if test_improving {
            format!("↑ {:.0}% better", test_pct)
        } else {
            format!("↓ {:.0}% worse", test_pct)
        }
    ));

    // Session count
    output.push_str(&format!(
        "{:<25} {:>14} {:>14}\n",
        "Sessions", previous_window.session_count, recent_window.session_count
    ));

    output.push('\n');

    // Overall assessment
    let improving_count = [
        error_improving,
        correction_improving,
        resolution_improving,
        test_improving,
    ]
    .iter()
    .filter(|&&x| x)
    .count();

    let assessment = match improving_count {
        4 => "Excellent: All metrics improving",
        3 => "Good: Most metrics improving",
        2 => "Fair: Some improvement",
        1 => "Mixed: Limited improvement",
        _ => "Needs attention: Most metrics declining or stable",
    };

    output.push_str(&format!("Overall: {}\n", assessment));

    ToolResult::text(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyphae_core::memoir::{Concept, Memoir};
    use hyphae_core::{Importance, MemoirStore, MemoryStore, ids::MemoirId};
    use serde_json::json;

    fn test_store() -> SqliteStore {
        SqliteStore::in_memory().expect("in-memory store")
    }

    fn store_memory(
        store: &SqliteStore,
        topic: &str,
        summary: &str,
        project: Option<&str>,
        worktree: Option<&str>,
    ) {
        let mut builder =
            Memory::builder(topic.to_string(), summary.to_string(), Importance::Medium);
        if let Some(project) = project {
            builder = builder.project(project.to_string());
        }
        if let Some(worktree) = worktree {
            builder = builder.worktree(worktree.to_string());
        }
        store.store(builder.build()).unwrap();
    }

    fn make_code_memoir(store: &SqliteStore, project: &str) -> MemoirId {
        let memoir = Memoir::new(
            format!("code:{project}"),
            format!("Code memoir for {project}"),
        );
        store.create_memoir(memoir).unwrap()
    }

    fn add_concept(store: &SqliteStore, memoir_id: &MemoirId, name: &str, definition: &str) {
        let concept = Concept::new(memoir_id.clone(), name.to_string(), definition.to_string());
        store.add_concept(concept).unwrap();
    }

    #[test]
    fn test_detect_secrets_normal_content() {
        assert!(detect_secrets("normal memory content").is_empty());
    }

    #[test]
    fn test_detect_secrets_api_key() {
        let warnings = detect_secrets("api_key=sk-abc123def456ghi789jkl");
        assert!(!warnings.is_empty());
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("API key") || w.contains("OpenAI"))
        );
    }

    #[test]
    fn test_detect_secrets_password() {
        let warnings = detect_secrets("password: mysecretpassword123");
        assert!(!warnings.is_empty());
        assert!(warnings.iter().any(|w| w.contains("password")));
    }

    #[test]
    fn test_detect_secrets_github_token() {
        let warnings = detect_secrets("ghp_abcdefghijklmnopqrstuvwxyz1234567890abc");
        assert!(!warnings.is_empty());
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("GitHub") || w.contains("token"))
        );
    }

    #[test]
    fn test_detect_secrets_aws_key() {
        let warnings = detect_secrets("AKIAIOSFODNN7EXAMPLE");
        assert!(!warnings.is_empty());
        assert!(warnings.iter().any(|w| w.contains("AWS")));
    }

    #[test]
    fn test_detect_secrets_bearer_token() {
        let warnings = detect_secrets("Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9");
        assert!(!warnings.is_empty());
        assert!(
            warnings
                .iter()
                .any(|w| w.contains("Bearer") || w.contains("token"))
        );
    }

    #[test]
    fn test_detect_secrets_private_key() {
        let warnings = detect_secrets("-----BEGIN PRIVATE KEY-----");
        assert!(!warnings.is_empty());
        assert!(warnings.iter().any(|w| w.contains("private key")));
    }

    #[test]
    fn test_detect_secrets_rsa_private_key() {
        let warnings = detect_secrets("-----BEGIN RSA PRIVATE KEY-----");
        assert!(!warnings.is_empty());
        assert!(warnings.iter().any(|w| w.contains("private key")));
    }

    #[test]
    fn test_detect_secrets_regular_text_with_token_word() {
        let warnings = detect_secrets("I need a token for my project");
        assert!(warnings.is_empty());
    }

    #[test]
    fn test_detect_secrets_multiple_types() {
        let content = "api_key=sk-abc123def456ghi789jkl and password: secretpassword123";
        let warnings = detect_secrets(content);
        assert!(!warnings.is_empty());
    }

    #[test]
    fn test_recall_heuristics_detect_session_and_code_expansion_terms() {
        let store = test_store();
        let memoir_id = make_code_memoir(&store, "demo");
        add_concept(
            &store,
            &memoir_id,
            "TokenValidator",
            "verify_token path validates auth tokens",
        );

        let heuristics =
            RecallHeuristics::detect(&store, "previous verify_token failure", Some("demo"), true);

        assert!(heuristics.session_query);
        assert!(heuristics.code_related);
        assert_eq!(
            heuristics.code_expansion_terms,
            vec!["TokenValidator".to_string()]
        );
        assert!(heuristics.prefer_context_aware_recall());
    }

    #[test]
    fn test_tool_recall_boosts_session_memories_for_session_queries() {
        let store = test_store();
        store_memory(
            &store,
            "notes",
            "login flow root cause summary",
            Some("demo"),
            None,
        );
        store_memory(
            &store,
            "session/demo",
            "previous session login flow fix applied to the auth redirect",
            Some("demo"),
            None,
        );

        let result = tool_recall(
            &store,
            None,
            &json!({"query": "previous session login flow", "limit": 1}),
            true,
            Some("demo"),
        );

        assert!(!result.is_error);
        let output = &result.content[0].text;
        let first_line = output.lines().next().unwrap_or_default();
        assert!(
            first_line.starts_with("[session/demo]"),
            "session memory should be surfaced first, got: {output}"
        );
    }

    #[test]
    fn test_tool_recall_expands_code_context_for_code_queries() {
        let store = test_store();
        let memoir_id = make_code_memoir(&store, "demo");
        add_concept(
            &store,
            &memoir_id,
            "TokenValidator",
            "verify_token auth validation path",
        );
        store_memory(
            &store,
            "errors/resolved",
            "TokenValidator panic on expired token",
            Some("demo"),
            None,
        );

        let result = tool_recall(
            &store,
            None,
            &json!({"query": "verify_token panic", "code_context": true}),
            true,
            Some("demo"),
        );

        assert!(!result.is_error);
        assert!(
            result.content[0]
                .text
                .contains("TokenValidator panic on expired token"),
            "code-context expansion should surface the related memory, got: {}",
            result.content[0].text
        );
    }

    #[test]
    fn test_tool_recall_prioritizes_code_context_hits_over_primary_results() {
        let store = test_store();
        let memoir_id = make_code_memoir(&store, "demo");
        add_concept(
            &store,
            &memoir_id,
            "TokenValidator",
            "verify_token auth validation path",
        );
        store_memory(
            &store,
            "errors/resolved",
            "TokenValidator panic on expired token",
            Some("demo"),
            None,
        );
        store_memory(
            &store,
            "notes",
            "verify_token failure root cause review",
            Some("demo"),
            None,
        );

        let result = tool_recall(
            &store,
            None,
            &json!({"query": "verify_token failure", "code_context": true, "limit": 1}),
            true,
            Some("demo"),
        );

        assert!(!result.is_error);
        let output = &result.content[0].text;
        let first_line = output.lines().next().unwrap_or_default();
        assert!(
            first_line.contains("TokenValidator panic on expired token"),
            "code-context expansion should beat primary results at the limit, got: {output}"
        );
        assert!(!output.contains("verify_token failure root cause review"));
    }

    #[test]
    fn test_tool_recall_keeps_shared_fallback_last_when_specific_hits_exist() {
        let store = test_store();
        let memoir_id = make_code_memoir(&store, "demo");
        add_concept(
            &store,
            &memoir_id,
            "TokenValidator",
            "verify_token auth validation path",
        );
        store_memory(
            &store,
            "errors/resolved",
            "TokenValidator panic on expired token",
            Some("demo"),
            None,
        );
        store_memory(
            &store,
            "notes",
            "verify_token failure root cause review",
            Some("demo"),
            None,
        );
        store_memory(
            &store,
            "shared-notes",
            "verify_token failure was tracked in shared notes",
            Some(hyphae_store::SHARED_PROJECT),
            None,
        );

        let result = tool_recall(
            &store,
            None,
            &json!({"query": "verify_token failure", "code_context": true, "limit": 3}),
            true,
            Some("demo"),
        );

        assert!(!result.is_error);
        let lines: Vec<&str> = result.content[0].text.lines().collect();
        assert_eq!(
            lines.len(),
            3,
            "expected three prioritized results, got: {lines:?}"
        );
        assert!(
            lines[0].contains("TokenValidator panic on expired token"),
            "code-context hit should come first, got: {lines:?}"
        );
        assert!(
            lines[1].contains("verify_token failure root cause review"),
            "primary project hit should come after code-context hits, got: {lines:?}"
        );
        assert!(
            lines[2].contains("verify_token failure was tracked in shared notes"),
            "shared fallback should come last, got: {lines:?}"
        );
    }

    #[test]
    fn test_tool_recall_expands_multiple_code_context_terms() {
        let store = test_store();
        let memoir_id = make_code_memoir(&store, "demo");
        add_concept(
            &store,
            &memoir_id,
            "TokenValidator",
            "verify_token auth validation path",
        );
        add_concept(
            &store,
            &memoir_id,
            "SessionCache",
            "session_cache login cache path",
        );
        store_memory(
            &store,
            "errors/resolved",
            "TokenValidator panic on expired token",
            Some("demo"),
            None,
        );
        store_memory(
            &store,
            "architecture",
            "SessionCache stale entry on login refresh",
            Some("demo"),
            None,
        );

        let result = tool_recall(
            &store,
            None,
            &json!({"query": "verify_token session_cache", "code_context": true, "limit": 10}),
            true,
            Some("demo"),
        );

        assert!(!result.is_error);
        assert!(
            result.content[0]
                .text
                .contains("TokenValidator panic on expired token"),
            "first expanded code term should surface a related memory, got: {}",
            result.content[0].text
        );
        assert!(
            result.content[0]
                .text
                .contains("SessionCache stale entry on login refresh"),
            "second expanded code term should also surface a related memory, got: {}",
            result.content[0].text
        );
    }

    #[test]
    fn test_tool_recall_does_not_expand_code_context_for_non_code_queries() {
        let store = test_store();
        let memoir_id = make_code_memoir(&store, "demo");
        add_concept(
            &store,
            &memoir_id,
            "TokenValidator",
            "auth validation path for request handling",
        );
        store_memory(
            &store,
            "errors/resolved",
            "TokenValidator panic on expired token",
            Some("demo"),
            None,
        );

        let result = tool_recall(
            &store,
            None,
            &json!({"query": "authentication issue", "code_context": true}),
            true,
            Some("demo"),
        );

        assert!(!result.is_error);
        assert_eq!(result.content[0].text, "No memories found.");
    }

    #[test]
    fn test_tool_recall_identity_scoping_respects_active_worktree() {
        let store = test_store();
        store_memory(
            &store,
            "architecture",
            "Alpha worktree target memory",
            Some("demo"),
            Some("/repo/demo"),
        );
        store_memory(
            &store,
            "architecture",
            "Beta worktree target memory",
            Some("demo"),
            Some("/repo/other"),
        );

        let result = tool_recall(
            &store,
            None,
            &json!({
                "query": "target memory",
                "project_root": "/repo/demo",
                "worktree_id": "wt-alpha"
            }),
            true,
            Some("demo"),
        );

        assert!(!result.is_error);
        let output = &result.content[0].text;
        assert!(output.contains("Alpha worktree target memory"));
        assert!(!output.contains("Beta worktree target memory"));
    }

    #[test]
    fn test_tool_evaluate_uses_structured_sessions() {
        let store = test_store();
        let (session_id, _) = store
            .session_start("demo-project", Some("structured session"))
            .unwrap();
        store
            .log_outcome_signal(
                Some(&session_id),
                "correction",
                -1,
                Some("cortina.post_tool_use"),
                Some("demo-project"),
            )
            .unwrap();
        store
            .session_end(&session_id, Some("done"), None, Some("0"))
            .unwrap();

        let result = tool_evaluate(&store, &json!({ "days": 14 }), Some("demo-project"));

        assert!(!result.is_error);
        let output = &result.content[0].text;
        assert!(output.contains("Agent Evaluation Report"));
        assert!(output.contains("Self-corrections/session"));
        assert!(!output.contains("Insufficient data"));
    }

    #[test]
    fn test_tool_evaluate_uses_correct_odd_day_headers() {
        let store = test_store();
        let (session_id, _) = store
            .session_start("demo-project", Some("structured session"))
            .unwrap();
        store
            .session_end(&session_id, Some("done"), None, Some("0"))
            .unwrap();

        let result = tool_evaluate(&store, &json!({ "days": 5 }), Some("demo-project"));

        assert!(!result.is_error);
        let output = &result.content[0].text;
        assert!(output.contains("Previous 3d"));
        assert!(output.contains("Recent 2d"));
    }
}
