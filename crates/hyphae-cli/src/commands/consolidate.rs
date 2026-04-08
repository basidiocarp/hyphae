use std::collections::HashSet;
use std::io::{self, Write};

use anyhow::{Result, bail};
use hyphae_core::{ConsolidationConfig, Importance, Memory, MemoryStore};
use hyphae_store::SqliteStore;

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConsolidationTarget {
    topic: String,
    count: usize,
    threshold: usize,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn cmd_consolidate(
    store: &SqliteStore,
    consolidation: &ConsolidationConfig,
    topic: Option<String>,
    all: bool,
    above_threshold: Option<usize>,
    dry_run: bool,
    yes: bool,
    project: Option<String>,
) -> Result<()> {
    if let Some(topic) = topic {
        let memories = store.get_by_topic(&topic, project.as_deref())?;
        if memories.is_empty() {
            bail!("topic not found or empty: {topic}");
        }

        let target = ConsolidationTarget {
            topic: topic.clone(),
            count: memories.len(),
            threshold: memories.len(),
        };

        if dry_run {
            print_single_topic_preview(&target);
            return Ok(());
        }

        let consolidated = build_consolidated_memory(&topic, &memories);
        store.consolidate_topic(&topic, consolidated)?;
        println!("Consolidated topic '{topic}' ({} memories).", target.count);
        return Ok(());
    }

    let bulk_threshold_override = match (all, above_threshold, dry_run) {
        (_, Some(value), _) => Some(value),
        (true, None, _) => None,
        (false, None, true) => None,
        (false, None, false) => {
            bail!("must specify --topic, --all, --above-threshold, or --dry-run");
        }
    };

    let targets = collect_bulk_targets(
        store,
        consolidation,
        project.as_deref(),
        bulk_threshold_override,
    )?;

    if dry_run {
        print_bulk_preview(&targets, bulk_threshold_override, consolidation);
        return Ok(());
    }

    if targets.is_empty() {
        println!("No topics are above the consolidation threshold.");
        return Ok(());
    }

    if !yes && !prompt_confirmation(&targets)? {
        println!("Consolidation cancelled.");
        return Ok(());
    }

    for target in targets {
        let memories = store.get_by_topic(&target.topic, project.as_deref())?;
        if memories.len() < 2 {
            continue;
        }
        let consolidated = build_consolidated_memory(&target.topic, &memories);
        store.consolidate_topic(&target.topic, consolidated)?;
        println!(
            "Consolidated topic '{}' ({} memories).",
            target.topic, target.count
        );
    }

    Ok(())
}

fn collect_bulk_targets(
    store: &SqliteStore,
    consolidation: &ConsolidationConfig,
    project: Option<&str>,
    threshold_override: Option<usize>,
) -> Result<Vec<ConsolidationTarget>> {
    let mut targets = Vec::new();

    for (topic, count) in store.list_topics(project)? {
        if consolidation.is_exempt(&topic) {
            continue;
        }

        let threshold = threshold_override
            .or_else(|| consolidation.threshold_for_topic(&topic))
            .unwrap_or(consolidation.default_threshold);

        if count >= threshold {
            targets.push(ConsolidationTarget {
                topic,
                count,
                threshold,
            });
        }
    }

    Ok(targets)
}

fn prompt_confirmation(targets: &[ConsolidationTarget]) -> Result<bool> {
    println!("This will consolidate:");
    for target in targets {
        println!(
            "  {} — {} memories (threshold {})",
            target.topic, target.count, target.threshold
        );
    }
    print!("\nContinue? [y/N] ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim().eq_ignore_ascii_case("y"))
}

fn print_bulk_preview(
    targets: &[ConsolidationTarget],
    threshold_override: Option<usize>,
    consolidation: &ConsolidationConfig,
) {
    let threshold_label = if let Some(value) = threshold_override {
        format!("{value} memories")
    } else if consolidation.topics.is_empty() {
        format!("{} memories", consolidation.default_threshold)
    } else {
        "configured thresholds".to_string()
    };
    println!("Topics above threshold ({threshold_label}):");
    for target in targets {
        println!("  {} — {} memories", target.topic, target.count);
    }
    if targets.is_empty() {
        println!("  (none)");
    } else {
        println!();
        println!("Run with --all to consolidate all, or --topic <t> to consolidate one.");
    }
}

fn print_single_topic_preview(target: &ConsolidationTarget) {
    println!(
        "Topic '{}' has {} memories (single-topic consolidate preview).",
        target.topic, target.count
    );
}

fn build_consolidated_memory(topic: &str, memories: &[Memory]) -> Memory {
    let mut ordered = memories.to_vec();
    ordered.sort_by_key(|memory| memory.created_at);

    let summary = if ordered.len() == 1 {
        ordered[0].summary.clone()
    } else {
        let snippets = ordered
            .iter()
            .take(8)
            .map(|memory| truncate_summary(&memory.summary, 120))
            .collect::<Vec<_>>();
        format!(
            "Consolidated {} memories for topic '{topic}': {}",
            ordered.len(),
            snippets.join(" | ")
        )
    };

    let importance = ordered
        .iter()
        .map(|memory| memory.importance)
        .max_by_key(|importance| importance_rank(*importance))
        .unwrap_or(Importance::Medium);

    let mut keywords = Vec::new();
    let mut seen_keywords = HashSet::new();
    for memory in &ordered {
        for keyword in &memory.keywords {
            if seen_keywords.insert(keyword.clone()) {
                keywords.push(keyword.clone());
            }
        }
    }

    let mut related_ids = Vec::new();
    for memory in &ordered {
        related_ids.push(memory.id.clone());
    }

    let newest = ordered.last().expect("non-empty memories");
    let mut builder = Memory::builder(topic.to_string(), summary, importance)
        .keywords(keywords)
        .related_ids(related_ids)
        .weight(1.0);

    if let Some(project) = newest.project.clone() {
        builder = builder.project(project);
    }
    if let Some(branch) = newest.branch.clone() {
        builder = builder.branch(branch);
    }
    if let Some(worktree) = newest.worktree.clone() {
        builder = builder.worktree(worktree);
    }

    builder.build()
}

fn importance_rank(importance: Importance) -> u8 {
    match importance {
        Importance::Critical => 5,
        Importance::High => 4,
        Importance::Medium => 3,
        Importance::Low => 2,
        Importance::Ephemeral => 1,
    }
}

fn truncate_summary(summary: &str, max_chars: usize) -> String {
    if summary.chars().count() <= max_chars {
        return summary.to_string();
    }

    summary.chars().take(max_chars).collect::<String>() + "…"
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyphae_core::{Importance, Memory, MemoryStore};

    fn test_store() -> SqliteStore {
        SqliteStore::in_memory().expect("in-memory store")
    }

    fn make_memory(topic: &str, summary: &str, importance: Importance) -> Memory {
        Memory::builder(topic.to_string(), summary.to_string(), importance).build()
    }

    #[test]
    fn test_bulk_targets_honor_thresholds_and_exemptions() {
        let store = test_store();
        let consolidation = ConsolidationConfig {
            default_threshold: 15,
            topics: [
                (
                    "errors/active".to_string(),
                    hyphae_core::ConsolidationTopicRule::Exempt,
                ),
                (
                    "exploration".to_string(),
                    hyphae_core::ConsolidationTopicRule::Threshold(8),
                ),
            ]
            .into_iter()
            .collect(),
        };

        for idx in 0..16 {
            store
                .store(make_memory(
                    "decisions/canopy",
                    &format!("decision {idx}"),
                    Importance::Medium,
                ))
                .unwrap();
        }
        for idx in 0..9 {
            store
                .store(make_memory(
                    "exploration",
                    &format!("explore {idx}"),
                    Importance::Low,
                ))
                .unwrap();
        }
        for idx in 0..30 {
            store
                .store(make_memory(
                    "errors/active",
                    &format!("error {idx}"),
                    Importance::High,
                ))
                .unwrap();
        }

        let targets = collect_bulk_targets(&store, &consolidation, None, None).unwrap();
        assert_eq!(
            targets
                .iter()
                .map(|target| target.topic.as_str())
                .collect::<Vec<_>>(),
            vec!["decisions/canopy", "exploration"]
        );
    }

    #[test]
    fn test_build_consolidated_memory_merges_keywords_and_related_ids() {
        let mut first = make_memory("topic", "first summary", Importance::Low);
        first.keywords = vec!["alpha".to_string(), "beta".to_string()];
        let mut second = make_memory("topic", "second summary", Importance::Critical);
        second.keywords = vec!["beta".to_string(), "gamma".to_string()];

        let consolidated = build_consolidated_memory("topic", &[first.clone(), second.clone()]);

        assert_eq!(consolidated.topic, "topic");
        assert_eq!(consolidated.importance, Importance::Critical);
        assert_eq!(consolidated.related_ids, vec![first.id, second.id]);
        assert_eq!(
            consolidated.keywords,
            vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()]
        );
        assert!(consolidated.summary.contains("Consolidated 2 memories"));
    }

    #[test]
    fn test_single_topic_consolidation_replaces_originals() {
        let store = test_store();
        let consolidation = ConsolidationConfig::default();

        store
            .store(make_memory(
                "rust",
                "binary search basics",
                Importance::Medium,
            ))
            .unwrap();
        store
            .store(make_memory("rust", "merge sort basics", Importance::High))
            .unwrap();

        cmd_consolidate(
            &store,
            &consolidation,
            Some("rust".to_string()),
            false,
            None,
            false,
            true,
            None,
        )
        .unwrap();

        assert_eq!(store.count_by_topic("rust", None).unwrap(), 1);
    }
}
