/// Built-in entity vocabulary: basidiocarp ecosystem tool and project names.
/// Extended at runtime via `HYPHAE_ENTITY_VOCAB` (comma-separated additions).
const BUILTIN_VOCAB: &[&str] = &[
    "hyphae",
    "canopy",
    "cortina",
    "mycelium",
    "rhizome",
    "spore",
    "stipe",
    "lamella",
    "annulus",
    "hymenium",
    "volva",
    "cap",
    "septa",
    "pileus",
    "basidiocarp",
];

/// Extract named entities from `text` using the built-in ecosystem vocabulary
/// plus any names added via `HYPHAE_ENTITY_VOCAB`.
///
/// Returns a sorted, deduplicated list of lowercase entity names found in the text.
#[must_use]
pub fn extract_entities(text: &str) -> Vec<String> {
    let extra: Vec<String> = std::env::var("HYPHAE_ENTITY_VOCAB")
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();

    let text_lower = text.to_lowercase();
    let mut found: Vec<String> = BUILTIN_VOCAB
        .iter()
        .map(|&name| name.to_string())
        .chain(extra)
        .filter(|name| !name.is_empty() && contains_word(&text_lower, name))
        .collect();

    found.sort_unstable();
    found.dedup();
    found
}

/// Returns true if `text` contains `word` as a whole word (surrounded by
/// non-alphanumeric characters or at string boundaries).
fn contains_word(text: &str, word: &str) -> bool {
    let mut start = 0;
    while let Some(rel_pos) = text[start..].find(word) {
        let abs_pos = start + rel_pos;
        let before_ok = abs_pos == 0
            || !text
                .as_bytes()
                .get(abs_pos - 1)
                .is_some_and(|b| b.is_ascii_alphanumeric());
        let after = abs_pos + word.len();
        let after_ok = !text
            .as_bytes()
            .get(after)
            .is_some_and(|b| b.is_ascii_alphanumeric());
        if before_ok && after_ok {
            return true;
        }
        start = abs_pos + 1;
        if start >= text.len() {
            break;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_ecosystem_names() {
        let text = "hyphae session start failed; check canopy task status";
        let entities = extract_entities(text);
        assert!(entities.contains(&"hyphae".to_string()));
        assert!(entities.contains(&"canopy".to_string()));
        assert!(!entities.contains(&"mycelium".to_string()));
    }

    #[test]
    fn whole_word_boundary_enforced() {
        // "cap" must not match inside "capability" or "capsule"
        let text = "capability capsule mapping";
        let entities = extract_entities(text);
        assert!(
            !entities.contains(&"cap".to_string()),
            "cap matched inside longer word"
        );
    }

    #[test]
    fn whole_word_match_at_boundaries() {
        let text = "cap failed to render the page";
        let entities = extract_entities(text);
        assert!(entities.contains(&"cap".to_string()));
    }

    #[test]
    fn returns_sorted_and_deduped() {
        let text = "hymenium hymenium cortina cortina hyphae";
        let entities = extract_entities(text);
        let mut expected = entities.clone();
        expected.sort_unstable();
        expected.dedup();
        assert_eq!(entities, expected, "result should already be sorted and deduped");
    }

    #[test]
    fn empty_text_returns_empty() {
        assert!(extract_entities("").is_empty());
    }

    #[test]
    fn cap_not_matched_in_capability() {
        // Regression: "cap" is a short name and prone to substring false matches.
        assert!(!contains_word("capability", "cap"));
        assert!(!contains_word("escape", "cap")); // no "cap" in "escape"
        assert!(contains_word("cap is broken", "cap"));
    }
}
