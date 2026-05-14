use serde_json::Value;
use spore::logging::workflow_span;

use hyphae_core::{ConsolidationConfig, Embedder, Importance, MemoirStore, Memory, MemoryStore};
use hyphae_store::{SHARED_PROJECT, SqliteStore};

use crate::protocol::ToolResult;

use super::super::{
    ToolTraceContext, get_bounded_i64, get_str, validate_required_string, workflow_span_context,
};
use crate::text::truncate_str;

const MAX_CONSOLIDATED_BYTES: usize = 64 * 1024;

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

pub(super) fn consolidation_hint(
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

pub(crate) fn tool_consolidate(
    store: &SqliteStore,
    args: &Value,
    project: Option<&str>,
    trace: &ToolTraceContext,
) -> ToolResult {
    let topic = match validate_required_string(args, "topic") {
        Ok(t) => t,
        Err(e) => return e,
    };
    let summary = match validate_required_string(args, "summary") {
        Ok(s) => s,
        Err(e) => return e,
    };
    if let Err(e) = super::super::validate_max_length(summary, "summary", 32768) {
        return e;
    }

    let workflow_context = workflow_span_context(trace, None, Some(topic));
    let _workflow_span = workflow_span("memory_consolidate", &workflow_context).entered();

    // Read existing memories once — used for both constitution guard, importance derivation,
    // and the consolidated-size cap check.
    let existing_memories = match store.get_by_topic(topic, project) {
        Ok(m) => m,
        Err(e) => {
            return ToolResult::error(format!(
                "failed to load existing memories for topic '{topic}': {e}"
            ));
        }
    };

    // Reject if the merged content (existing + new summary) exceeds the cap. This prevents
    // a topic from growing unboundedly through repeated consolidation cycles.
    let existing_total: usize = existing_memories.iter().map(|m| m.summary.len()).sum();
    if existing_total + summary.len() > MAX_CONSOLIDATED_BYTES {
        return ToolResult::error(format!(
            "topic '{topic}' would produce a consolidated body of {} bytes, exceeding the {} byte limit — split the topic or reduce existing memories before consolidating",
            existing_total + summary.len(),
            MAX_CONSOLIDATED_BYTES
        ));
    }

    // Derive importance from the highest-importance source memory so that a
    // consolidation of critical or high-importance memories does not silently
    // downgrade them to High.
    //
    // Constitution memories are permanently exempt from consolidation — they
    // represent governance policy that must never be merged or discarded.
    let importance = if existing_memories.is_empty() {
        Importance::High
    } else {
        if existing_memories
            .iter()
            .any(|m| m.importance == Importance::Constitution)
        {
            return ToolResult::error(format!(
                "topic '{topic}' contains constitution memories and cannot be consolidated"
            ));
        }
        fn rank(i: Importance) -> u8 {
            match i {
                Importance::Constitution => 5,
                Importance::Critical => 4,
                Importance::High => 3,
                Importance::Medium => 2,
                Importance::Low => 1,
                Importance::Ephemeral => 0,
            }
        }
        existing_memories
            .iter()
            .map(|m| m.importance)
            .max_by_key(|i| rank(*i))
            .unwrap_or(Importance::High)
    };

    let mut builder = Memory::builder(topic.into(), summary.into(), importance);
    if let Some(p) = project {
        builder = builder.project(p.to_string());
    }
    let consolidated = builder.build();

    match store.consolidate_topic(topic, consolidated) {
        Ok(()) => ToolResult::text(format!("Consolidated topic: {topic}")),
        Err(e) => ToolResult::error(format!("failed to consolidate: {e}")),
    }
}

pub(crate) fn tool_list_invalidated(
    store: &SqliteStore,
    args: &Value,
    project: Option<&str>,
    trace: &ToolTraceContext,
) -> ToolResult {
    let limit = get_bounded_i64(args, "limit", 20, 1, 100) as usize;
    let offset = get_bounded_i64(args, "offset", 0, 0, 10_000) as usize;
    let workflow_context = workflow_span_context(trace, None, None);
    let _workflow_span = workflow_span("memory_list_invalidated", &workflow_context).entered();

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

pub(crate) fn tool_list_topics(
    store: &SqliteStore,
    project: Option<&str>,
    trace: &ToolTraceContext,
) -> ToolResult {
    let workflow_context = workflow_span_context(trace, None, None);
    let _workflow_span = workflow_span("memory_list_topics", &workflow_context).entered();
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

pub(crate) fn tool_stats(
    store: &SqliteStore,
    project: Option<&str>,
    trace: &ToolTraceContext,
) -> ToolResult {
    let workflow_context = workflow_span_context(trace, None, None);
    let _workflow_span = workflow_span("memory_stats", &workflow_context).entered();
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

pub(crate) fn tool_health_with_rules(
    store: &SqliteStore,
    consolidation: &ConsolidationConfig,
    args: &Value,
    project: Option<&str>,
    trace: &ToolTraceContext,
) -> ToolResult {
    let specific_topic = get_str(args, "topic");
    let workflow_context = workflow_span_context(trace, None, None);
    let _workflow_span = workflow_span("memory_health", &workflow_context).entered();

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
    trace: &ToolTraceContext,
) -> ToolResult {
    let embedder = match embedder {
        Some(e) => e,
        None => return ToolResult::error("embeddings not available".into()),
    };
    let workflow_context = workflow_span_context(trace, None, None);
    let _workflow_span = workflow_span("memory_embed_all", &workflow_context).entered();

    let topic_filter = get_str(args, "topic");

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

const PROMOTION_THRESHOLD: usize = 15;

pub(crate) fn tool_recall_global(
    store: &SqliteStore,
    args: &Value,
    compact: bool,
    project: Option<&str>,
    trace: &ToolTraceContext,
) -> ToolResult {
    let query = match get_str(args, "query") {
        Some(q) => q,
        None => return ToolResult::error("missing required field: query".into()),
    };
    let limit = get_bounded_i64(args, "limit", 10, 1, 100) as usize;
    let unrestricted = args
        .get("unrestricted")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let workflow_context = workflow_span_context(trace, None, None);
    let _workflow_span = workflow_span("recall_global", &workflow_context).entered();

    let results = if unrestricted {
        // Restore all-projects behavior when explicitly requested
        match store.search_all_projects(query, limit) {
            Ok(r) => r,
            Err(e) => return ToolResult::error(format!("search error: {e}")),
        }
    } else {
        // Build allow-list: _shared + caller project + linked projects
        let mut allow_list_strings = vec![SHARED_PROJECT.to_string()];
        if let Some(p) = project {
            allow_list_strings.push(p.to_string());

            // Add linked projects
            match store.get_linked_projects(p) {
                Ok(linked) => {
                    allow_list_strings.extend(linked);
                }
                Err(e) => {
                    tracing::warn!("failed to get linked projects for {}: {}", p, e);
                }
            }
        }

        // Convert to string references for the search
        let allow_list: Vec<&str> = allow_list_strings.iter().map(String::as_str).collect();

        // Search only the allowed projects
        match store.search_related_projects(query, &allow_list, limit) {
            Ok(r) => r,
            Err(e) => return ToolResult::error(format!("search error: {e}")),
        }
    };

    if results.is_empty() {
        return ToolResult::text("No memories found across any project.".into());
    }

    for mem in &results {
        if let Err(e) = store.update_access(&mem.id) {
            tracing::warn!("update_access failed: {e}");
        }
    }

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

pub(crate) fn tool_promote_to_memoir(
    store: &SqliteStore,
    args: &Value,
    project: Option<&str>,
    trace: &ToolTraceContext,
) -> ToolResult {
    let topic = match validate_required_string(args, "topic") {
        Ok(t) => t,
        Err(e) => return e,
    };
    let workflow_context = workflow_span_context(trace, None, Some(topic));
    let _workflow_span = workflow_span("promote_to_memoir", &workflow_context).entered();

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

    let memoir_name = topic.replace('/', "-");
    if let Ok(memoirs) = store.list_memoirs() {
        if memoirs.iter().any(|m| m.name == memoir_name) {
            return ToolResult::text(format!(
                "A memoir named \"{memoir_name}\" already exists. \
                 Use hyphae_memoir_refine to update its concepts instead."
            ));
        }
    }

    let mut keyword_freq: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for mem in &memories {
        for kw in &mem.keywords {
            *keyword_freq.entry(kw.clone()).or_default() += 1;
        }
    }

    let mut top_keywords: Vec<_> = keyword_freq.into_iter().collect();
    top_keywords.sort_by_key(|b| std::cmp::Reverse(b.1));
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

#[cfg(test)]
mod tests {
    use super::*;
    use hyphae_store::SqliteStore;
    use serde_json::json;

    fn test_store() -> SqliteStore {
        SqliteStore::in_memory().expect("in-memory store")
    }

    fn make_trace() -> ToolTraceContext {
        ToolTraceContext {
            request_id: Some("test-request".to_string()),
            session_id: Some("test-session".to_string()),
            workspace_root: None,
        }
    }

    fn get_text_content(result: &ToolResult) -> String {
        result
            .content
            .first()
            .map(|c| c.text.clone())
            .unwrap_or_default()
    }

    #[test]
    fn test_consolidate_within_limit_succeeds() {
        let store = test_store();
        let trace = make_trace();

        let args = json!({
            "topic": "test/topic",
            "summary": "A short consolidated summary"
        });

        let result = tool_consolidate(&store, &args, None, &trace);
        assert!(!result.is_error, "consolidation should succeed");
        let text = get_text_content(&result);
        assert!(text.contains("Consolidated topic"));
    }

    #[test]
    fn test_consolidate_exceeds_limit_returns_error() {
        use hyphae_core::memory::{Importance, Memory};
        let store = test_store();
        let trace = make_trace();

        // Pre-populate the topic with 50KB of existing memory content.
        // Then attempt to consolidate with a 20KB summary — total 70KB > 64KB cap.
        let existing_body = "e".repeat(50 * 1024);
        let mem =
            Memory::builder("cap/test".into(), existing_body.into(), Importance::Medium).build();
        store.store(mem).expect("store pre-existing memory");

        let new_summary = "n".repeat(20 * 1024);
        let args = json!({
            "topic": "cap/test",
            "summary": new_summary
        });

        let result = tool_consolidate(&store, &args, None, &trace);
        assert!(
            result.is_error,
            "50KB existing + 20KB new should exceed 64KB cap"
        );
        let error_text = get_text_content(&result);
        assert!(
            error_text.contains("cap/test"),
            "error should mention topic name, got: {error_text}"
        );
        assert!(
            error_text.contains("bytes"),
            "error should mention byte limit, got: {error_text}"
        );
    }

    #[test]
    fn test_consolidate_summary_at_input_limit_succeeds() {
        let store = test_store();
        let trace = make_trace();

        let large_summary = "x".repeat(32000);
        let args = json!({
            "topic": "errors/resolved",
            "summary": large_summary
        });

        let result = tool_consolidate(&store, &args, None, &trace);
        assert!(!result.is_error, "summary under 32KB limit should succeed");
        let text = get_text_content(&result);
        assert!(text.contains("Consolidated topic"));
        assert!(text.contains("errors/resolved"));
    }

    #[test]
    fn test_consolidate_summary_exceeds_input_limit_rejected() {
        let store = test_store();
        let trace = make_trace();

        let too_large_summary = "x".repeat(32769);
        let args = json!({
            "topic": "test/topic",
            "summary": too_large_summary
        });

        let result = tool_consolidate(&store, &args, None, &trace);
        assert!(result.is_error, "summary over 32KB limit should fail");
        let error_text = get_text_content(&result);
        assert!(error_text.contains("exceeds maximum length"));
    }
}
