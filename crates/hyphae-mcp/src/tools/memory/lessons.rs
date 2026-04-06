use serde_json::Value;

use hyphae_core::{Memory, MemoryStore};
use hyphae_store::SqliteStore;

use crate::protocol::ToolResult;

use super::super::get_bounded_i64;
use super::helpers::{extract_common_pattern, extract_keywords};

pub(crate) fn tool_extract_lessons(
    store: &SqliteStore,
    args: &Value,
    project: Option<&str>,
) -> ToolResult {
    let limit = get_bounded_i64(args, "limit", 10, 1, 50) as usize;

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
