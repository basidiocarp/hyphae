use serde_json::Value;
use spore::logging::workflow_span;

use hyphae_core::{Memory, MemoryStore, ReflexionStore};
use hyphae_store::SqliteStore;

use crate::protocol::ToolResult;

use super::super::{ToolTraceContext, get_bounded_i64, workflow_span_context};
use super::helpers::{extract_common_pattern, extract_keywords};

pub(crate) fn tool_extract_lessons(
    store: &SqliteStore,
    args: &Value,
    project: Option<&str>,
    trace: &ToolTraceContext,
) -> ToolResult {
    let limit = get_bounded_i64(args, "limit", 10, 1, 50) as usize;
    let workflow_context = workflow_span_context(trace, None, None);
    let _workflow_span = workflow_span("extract_lessons", &workflow_context).entered();

    let corrections = store
        .get_by_topic("corrections", project)
        .unwrap_or_default();
    let errors_resolved = store
        .get_by_topic("errors/resolved", project)
        .unwrap_or_default();
    let tests_resolved = store
        .get_by_topic("tests/resolved", project)
        .unwrap_or_default();

    let mut all_memories: Vec<(&str, &Memory)> = Vec::new();
    all_memories.extend(corrections.iter().map(|m| ("corrections", m)));
    all_memories.extend(errors_resolved.iter().map(|m| ("errors/resolved", m)));
    all_memories.extend(tests_resolved.iter().map(|m| ("tests/resolved", m)));

    if all_memories.is_empty() {
        return ToolResult::text(
            "No memories found in corrections, errors/resolved, or tests/resolved topics.".into(),
        );
    }

    all_memories.truncate(50);

    let mut keyword_groups: std::collections::HashMap<String, Vec<(&str, &Memory)>> =
        std::collections::HashMap::new();

    for (topic_type, mem) in &all_memories {
        let mut keywords = mem.keywords.clone();
        keywords.extend(extract_keywords(&mem.summary));

        if keywords.is_empty() {
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

    let mut lessons: Vec<String> = Vec::new();

    for (keyword, group_mems) in keyword_groups {
        if group_mems.len() < 2 {
            continue;
        }

        let mut type_counts = std::collections::HashMap::new();
        for (topic_type, _) in &group_mems {
            *type_counts.entry(*topic_type).or_insert(0) += 1;
        }

        let summaries: Vec<&str> = group_mems.iter().map(|(_, m)| m.summary.as_str()).collect();
        let pattern = extract_common_pattern(&summaries);

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

    lessons.sort();
    lessons.truncate(limit);

    // Collect reflexion patterns
    let reflexion_records = store.list_reflexions_by_pattern(20).unwrap_or_default();

    let mut reflexion_patterns: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();

    for record in &reflexion_records {
        *reflexion_patterns
            .entry(record.abstract_pattern.clone())
            .or_insert(0) += 1;
    }

    // Only mention reflexion patterns in the header when there are some — avoids
    // a misleading "0 reflexion patterns" line when no reflexion records exist yet.
    let mut output = if reflexion_patterns.is_empty() {
        format!(
            "Lessons extracted from {} corrections, {} error resolutions, {} test fixes:\n\n",
            corrections.len(),
            errors_resolved.len(),
            tests_resolved.len(),
        )
    } else {
        format!(
            "Lessons extracted from {} corrections, {} error resolutions, {} test fixes, {} reflexion patterns:\n\n",
            corrections.len(),
            errors_resolved.len(),
            tests_resolved.len(),
            reflexion_records.len(),
        )
    };

    let mut lesson_idx = 1;

    for lesson in lessons.iter().take(limit) {
        output.push_str(&format!("{}. {}\n", lesson_idx, lesson));
        lesson_idx += 1;
    }

    if !reflexion_patterns.is_empty() {
        output.push_str("\nReflexion-derived patterns:\n");
        let mut sorted_patterns: Vec<_> = reflexion_patterns.iter().collect();
        sorted_patterns.sort_by_key(|&(_, count)| std::cmp::Reverse(*count));

        for (pattern, count) in sorted_patterns
            .iter()
            .take(limit.saturating_sub(lessons.len()))
        {
            output.push_str(&format!(
                "{}. [reflexion] {}: {} occurrences\n",
                lesson_idx, pattern, count
            ));
            lesson_idx += 1;
        }
    }

    output.push_str("\nUse these lessons to avoid repeating past mistakes.\n");

    ToolResult::text(output)
}
