use chrono::Utc;
use serde_json::Value;
use spore::logging::workflow_span;

use hyphae_core::{Embedder, Memory, MemoryStore};
use hyphae_store::{SqliteStore, context};

use crate::protocol::ToolResult;

use super::super::{
    ToolTraceContext, get_bounded_i64, get_str, normalize_identity, resolve_workspace_root,
    scoped_worktree_root, workflow_span_context,
};
use super::helpers::dedupe_memory_results;

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
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
        let code_expansion_terms = expand_code_context(
            store,
            query,
            project,
            code_context_requested && code_related,
        );

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

fn expand_code_context(
    store: &SqliteStore,
    query: &str,
    project: Option<&str>,
    enabled: bool,
) -> Vec<String> {
    if !enabled {
        return Vec::new();
    }

    project
        .map(|project| context::expand_with_code_context(store, query, project))
        .unwrap_or_default()
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

fn compute_consolidation_hint(
    store: &SqliteStore,
    query_topic: Option<&str>,
    project: Option<&str>,
) -> Option<String> {
    let topic = query_topic?;
    let memories = match store.get_by_topic(topic, project) {
        Ok(memories) => memories,
        Err(_) => return None,
    };

    if memories.len() <= 20 {
        return None;
    }

    let oldest = memories.iter().min_by_key(|m| m.created_at)?;
    let newest = memories.iter().max_by_key(|m| m.created_at)?;
    let span_days = (newest.created_at - oldest.created_at).num_days();
    if span_days <= 7 {
        return None;
    }

    Some(format!(
        "\n[Hyphae: Topic \"{topic}\" has {} memories spanning {span_days} days. \
         Auto-consolidation recommended. Run hyphae_memory_consolidate(topic: \"{topic}\") \
         to merge redundant entries.]",
        memories.len()
    ))
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

    Ok(dedupe_memory_results(branches, limit))
}

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
                scored_results.push((mem, score * 0.7));
            }
        }
        scored_results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored_results.truncate(limit);
    }
    scored_results
}

pub(crate) fn tool_recall(
    store: &SqliteStore,
    embedder: Option<&dyn Embedder>,
    args: &Value,
    compact: bool,
    project: Option<&str>,
    trace: &ToolTraceContext,
) -> ToolResult {
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
    let workflow_context = workflow_span_context(trace, resolve_workspace_root(args), session_id);
    let _workflow_span = workflow_span("memory_recall", &workflow_context).entered();
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
    let auto_consolidate_hint = compute_consolidation_hint(store, topic, project);

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
