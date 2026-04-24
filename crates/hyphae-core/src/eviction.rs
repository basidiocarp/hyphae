use crate::{memory::Memory, tier::MemoryTier};

/// Determines which memories to include when context budget is limited.
/// Highest-priority entries are returned first; lower-priority are evicted.
///
/// # Budget semantics
///
/// `token_budget` applies only to non-Core entries. Core entries always pass
/// through regardless of budget and are not counted against it. Callers that
/// need a hard total-token ceiling should filter the returned vec themselves.
pub trait EvictionPolicy: Send + Sync {
    fn select_for_context<'a>(
        &self,
        candidates: &'a [&'a Memory],
        token_budget: usize,
    ) -> Vec<&'a Memory>;
}

/// Default policy: Core > Recall (recency) > Archival (never unless searched).
pub struct DefaultEvictionPolicy;

impl EvictionPolicy for DefaultEvictionPolicy {
    fn select_for_context<'a>(
        &self,
        candidates: &'a [&'a Memory],
        token_budget: usize,
    ) -> Vec<&'a Memory> {
        let mut sorted: Vec<&'a Memory> = candidates.to_vec();
        sorted.sort_by(|a, b| {
            tier_priority(a.tier)
                .cmp(&tier_priority(b.tier))
                .then(b.created_at.cmp(&a.created_at)) // recency within tier
        });

        let mut used = 0usize;
        sorted
            .into_iter()
            .filter(|e| {
                let tokens = estimate_tokens(&e.summary);
                if e.tier == MemoryTier::Core {
                    true // always include Core
                } else if used + tokens <= token_budget {
                    used += tokens;
                    true
                } else {
                    false
                }
            })
            .collect()
    }
}

fn tier_priority(tier: MemoryTier) -> u8 {
    match tier {
        MemoryTier::Core => 0,
        MemoryTier::Recall => 1,
        MemoryTier::Archival => 2,
    }
}

fn estimate_tokens(content: &str) -> usize {
    (content.len() / 4).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Importance;
    use crate::memory::Memory;

    fn make_memory(tier: MemoryTier, summary: &str) -> Memory {
        let mut m = Memory::new("test".to_string(), summary.to_string(), Importance::Medium);
        m.tier = tier;
        m
    }

    #[test]
    fn core_always_included_even_over_budget() {
        let policy = DefaultEvictionPolicy;
        let candidates = [
            make_memory(MemoryTier::Core, "core fact"),
            make_memory(MemoryTier::Recall, "a".repeat(1000).as_str()),
        ];
        let candidates_refs: Vec<&Memory> = candidates.iter().collect();
        let result = policy.select_for_context(&candidates_refs, 1); // tiny budget
        assert!(result.iter().any(|m| m.tier == MemoryTier::Core));
    }

    #[test]
    fn archival_evicted_before_recall() {
        let policy = DefaultEvictionPolicy;
        let candidates = [
            make_memory(MemoryTier::Archival, "archive"),
            make_memory(MemoryTier::Recall, "recent"),
        ];
        let candidates_refs: Vec<&Memory> = candidates.iter().collect();
        let result = policy.select_for_context(&candidates_refs, 10);
        // Recall should appear before Archival in priority order
        let tiers: Vec<_> = result.iter().map(|m| m.tier).collect();
        assert_eq!(tiers[0], MemoryTier::Recall);
    }

    #[test]
    fn zero_budget_only_returns_core() {
        let policy = DefaultEvictionPolicy;
        let candidates = [
            make_memory(MemoryTier::Core, "always"),
            make_memory(MemoryTier::Recall, "maybe"),
            make_memory(MemoryTier::Archival, "never"),
        ];
        let candidates_refs: Vec<&Memory> = candidates.iter().collect();
        let result = policy.select_for_context(&candidates_refs, 0);
        assert!(result.iter().all(|m| m.tier == MemoryTier::Core));
        assert_eq!(result.len(), 1);
    }
}
