use std::collections::HashSet;

use hyphae_core::Memory;

/// Safely truncate a string at a byte boundary, respecting multi-byte UTF-8.
pub(super) fn truncate_str(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Merge branches of memories while preserving first-seen ordering and removing duplicates.
pub(super) fn dedupe_memory_results(branches: Vec<Vec<Memory>>, limit: usize) -> Vec<Memory> {
    let mut results = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

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

/// Extract lowercase keywords from text (words > 3 chars, excluding common words).
pub(super) fn extract_keywords(text: &str) -> Vec<String> {
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
        .collect::<HashSet<_>>()
        .into_iter()
        .collect()
}

/// Extract a common pattern from multiple summaries by finding shared phrases.
pub(super) fn extract_common_pattern(summaries: &[&str]) -> String {
    if summaries.is_empty() {
        return "unknown pattern".to_string();
    }

    if summaries.len() == 1 {
        return summaries[0].to_string();
    }

    let first_tokens: HashSet<String> = summaries[0]
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
        format!(
            "pattern like '{}'",
            summaries[0].chars().take(50).collect::<String>()
        )
    }
}
