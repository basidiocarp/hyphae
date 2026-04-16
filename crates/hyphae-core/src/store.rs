use crate::error::HyphaeResult;
use crate::ids::MemoryId;
use crate::memory::{Memory, StoreStats, TopicHealth};

/// Core memory storage trait.
///
/// # Lifecycle hooks
///
/// Three optional hooks allow backends to participate in agent lifecycle events:
/// [`MemoryStore::queue_prefetch`], [`MemoryStore::on_pre_compress`], and
/// [`MemoryStore::on_delegation`]. All three have default no-op implementations
/// so existing backends compile without changes.
///
/// Hook failures are isolated — callers should log and continue rather than
/// propagating the error to the caller. Use `.unwrap_or_default()` or log and
/// proceed when invoking hooks from orchestration code.
pub trait MemoryStore {
    // CRUD
    fn store(&self, memory: Memory) -> HyphaeResult<MemoryId>;
    fn get(&self, id: &MemoryId) -> HyphaeResult<Option<Memory>>;
    fn update(&self, memory: &Memory) -> HyphaeResult<()>;
    fn delete(&self, id: &MemoryId) -> HyphaeResult<()>;
    fn invalidate(
        &self,
        id: &MemoryId,
        reason: Option<&str>,
        superseded_by: Option<&MemoryId>,
    ) -> HyphaeResult<()>;
    fn list_invalidated(
        &self,
        limit: usize,
        offset: usize,
        project: Option<&str>,
    ) -> HyphaeResult<Vec<Memory>>;

    // Search
    fn search_by_keywords(
        &self,
        keywords: &[&str],
        limit: usize,
        offset: usize,
        project: Option<&str>,
    ) -> HyphaeResult<Vec<Memory>>;
    fn search_fts(
        &self,
        query: &str,
        limit: usize,
        offset: usize,
        project: Option<&str>,
    ) -> HyphaeResult<Vec<Memory>>;
    fn search_fts_in_topic(
        &self,
        query: &str,
        topic: &str,
        limit: usize,
        offset: usize,
        project: Option<&str>,
    ) -> HyphaeResult<Vec<Memory>>;
    fn search_by_embedding(
        &self,
        embedding: &[f32],
        limit: usize,
        offset: usize,
        project: Option<&str>,
    ) -> HyphaeResult<Vec<(Memory, f32)>>;
    fn search_hybrid(
        &self,
        query: &str,
        embedding: &[f32],
        limit: usize,
        offset: usize,
        project: Option<&str>,
    ) -> HyphaeResult<Vec<(Memory, f32)>>;

    // Lifecycle
    fn update_access(&self, id: &MemoryId) -> HyphaeResult<()>;
    fn apply_decay(&self, decay_factor: f32) -> HyphaeResult<usize>;
    fn prune(&self, weight_threshold: f32) -> HyphaeResult<usize>;
    fn prune_expired(&self) -> HyphaeResult<usize>;

    // Organization
    fn get_by_topic(&self, topic: &str, project: Option<&str>) -> HyphaeResult<Vec<Memory>>;
    fn list_topics(&self, project: Option<&str>) -> HyphaeResult<Vec<(String, usize)>>;
    fn consolidate_topic(&self, topic: &str, consolidated: Memory) -> HyphaeResult<()>;

    // Stats
    fn count(&self, project: Option<&str>) -> HyphaeResult<usize>;
    fn count_by_topic(&self, topic: &str, project: Option<&str>) -> HyphaeResult<usize>;
    fn stats(&self, project: Option<&str>) -> HyphaeResult<StoreStats>;
    fn topic_health(&self, topic: &str, project: Option<&str>) -> HyphaeResult<TopicHealth>;

    // Provider lifecycle hooks
    //
    // These hooks are optional. Default implementations are no-ops so backends
    // that do not need a hook compile unchanged. Hook failures are isolated:
    // callers must not let a failing hook block the primary operation.

    /// Signal that a background prefetch should begin for the given query hint.
    ///
    /// Call this before the model turn starts so that relevant memories can be
    /// warmed before they are needed. The default implementation is a no-op.
    /// Implementors may choose to ignore the hint or enqueue asynchronous work.
    fn queue_prefetch(&self, query_hint: &str) -> HyphaeResult<()> {
        let _ = query_hint;
        Ok(())
    }

    /// Return the IDs of entries that must be protected from compression.
    ///
    /// Called before context compression occurs. Backends may return a subset
    /// of `candidate_ids` that contain content critical enough to preserve.
    /// The default returns an empty vec, protecting nothing.
    ///
    /// Callers must treat a hook error as non-fatal: log it and proceed with
    /// compression as if the backend returned an empty protection set.
    fn on_pre_compress(&self, candidate_ids: &[&str]) -> HyphaeResult<Vec<String>> {
        let _ = candidate_ids;
        Ok(vec![])
    }

    /// Notify the backend that a task is being delegated to another agent.
    ///
    /// Backends may use this to flush, snapshot, or annotate memory state so
    /// the receiving agent has relevant context. The default implementation is
    /// a no-op.
    fn on_delegation(&self, target_agent_id: &str) -> HyphaeResult<()> {
        let _ = target_agent_id;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::HyphaeError;
    use crate::ids::MemoryId;
    use crate::memory::{Memory, StoreStats, TopicHealth};

    /// Minimal stub that only implements the required methods.
    /// Default hook implementations are inherited from the trait.
    struct StubStore;

    impl MemoryStore for StubStore {
        fn store(&self, _memory: Memory) -> HyphaeResult<MemoryId> {
            unimplemented!()
        }
        fn get(&self, _id: &MemoryId) -> HyphaeResult<Option<Memory>> {
            unimplemented!()
        }
        fn update(&self, _memory: &Memory) -> HyphaeResult<()> {
            unimplemented!()
        }
        fn delete(&self, _id: &MemoryId) -> HyphaeResult<()> {
            unimplemented!()
        }
        fn invalidate(
            &self,
            _id: &MemoryId,
            _reason: Option<&str>,
            _superseded_by: Option<&MemoryId>,
        ) -> HyphaeResult<()> {
            unimplemented!()
        }
        fn list_invalidated(
            &self,
            _limit: usize,
            _offset: usize,
            _project: Option<&str>,
        ) -> HyphaeResult<Vec<Memory>> {
            unimplemented!()
        }
        fn search_by_keywords(
            &self,
            _keywords: &[&str],
            _limit: usize,
            _offset: usize,
            _project: Option<&str>,
        ) -> HyphaeResult<Vec<Memory>> {
            unimplemented!()
        }
        fn search_fts(
            &self,
            _query: &str,
            _limit: usize,
            _offset: usize,
            _project: Option<&str>,
        ) -> HyphaeResult<Vec<Memory>> {
            unimplemented!()
        }
        fn search_fts_in_topic(
            &self,
            _query: &str,
            _topic: &str,
            _limit: usize,
            _offset: usize,
            _project: Option<&str>,
        ) -> HyphaeResult<Vec<Memory>> {
            unimplemented!()
        }
        fn search_by_embedding(
            &self,
            _embedding: &[f32],
            _limit: usize,
            _offset: usize,
            _project: Option<&str>,
        ) -> HyphaeResult<Vec<(Memory, f32)>> {
            unimplemented!()
        }
        fn search_hybrid(
            &self,
            _query: &str,
            _embedding: &[f32],
            _limit: usize,
            _offset: usize,
            _project: Option<&str>,
        ) -> HyphaeResult<Vec<(Memory, f32)>> {
            unimplemented!()
        }
        fn update_access(&self, _id: &MemoryId) -> HyphaeResult<()> {
            unimplemented!()
        }
        fn apply_decay(&self, _decay_factor: f32) -> HyphaeResult<usize> {
            unimplemented!()
        }
        fn prune(&self, _weight_threshold: f32) -> HyphaeResult<usize> {
            unimplemented!()
        }
        fn prune_expired(&self) -> HyphaeResult<usize> {
            unimplemented!()
        }
        fn get_by_topic(&self, _topic: &str, _project: Option<&str>) -> HyphaeResult<Vec<Memory>> {
            unimplemented!()
        }
        fn list_topics(&self, _project: Option<&str>) -> HyphaeResult<Vec<(String, usize)>> {
            unimplemented!()
        }
        fn consolidate_topic(&self, _topic: &str, _consolidated: Memory) -> HyphaeResult<()> {
            unimplemented!()
        }
        fn count(&self, _project: Option<&str>) -> HyphaeResult<usize> {
            unimplemented!()
        }
        fn count_by_topic(&self, _topic: &str, _project: Option<&str>) -> HyphaeResult<usize> {
            unimplemented!()
        }
        fn stats(&self, _project: Option<&str>) -> HyphaeResult<StoreStats> {
            unimplemented!()
        }
        fn topic_health(&self, _topic: &str, _project: Option<&str>) -> HyphaeResult<TopicHealth> {
            unimplemented!()
        }
    }

    #[test]
    fn test_queue_prefetch_default_is_noop() {
        let store = StubStore;
        assert!(store.queue_prefetch("recent errors in spore").is_ok());
    }

    #[test]
    fn test_queue_prefetch_default_ignores_empty_hint() {
        let store = StubStore;
        assert!(store.queue_prefetch("").is_ok());
    }

    #[test]
    fn test_on_pre_compress_default_returns_empty_vec() {
        let store = StubStore;
        let candidates = ["mem-1", "mem-2", "mem-3"];
        let protected = store
            .on_pre_compress(&candidates)
            .expect("on_pre_compress default must not fail");
        assert!(
            protected.is_empty(),
            "default on_pre_compress must protect nothing"
        );
    }

    #[test]
    fn test_on_pre_compress_default_with_no_candidates() {
        let store = StubStore;
        let protected = store
            .on_pre_compress(&[])
            .expect("on_pre_compress with empty slice must not fail");
        assert!(protected.is_empty());
    }

    #[test]
    fn test_on_delegation_default_is_noop() {
        let store = StubStore;
        assert!(store.on_delegation("agent-abc-123").is_ok());
    }

    #[test]
    fn test_on_delegation_default_ignores_empty_agent_id() {
        let store = StubStore;
        assert!(store.on_delegation("").is_ok());
    }

    /// Demonstrate the isolation pattern callers should use when a hook fails.
    #[test]
    fn test_hook_failure_isolation_pattern() {
        struct FailingStore;

        impl MemoryStore for FailingStore {
            fn store(&self, _: Memory) -> HyphaeResult<MemoryId> {
                unimplemented!()
            }
            fn get(&self, _: &MemoryId) -> HyphaeResult<Option<Memory>> {
                unimplemented!()
            }
            fn update(&self, _: &Memory) -> HyphaeResult<()> {
                unimplemented!()
            }
            fn delete(&self, _: &MemoryId) -> HyphaeResult<()> {
                unimplemented!()
            }
            fn invalidate(
                &self,
                _: &MemoryId,
                _: Option<&str>,
                _: Option<&MemoryId>,
            ) -> HyphaeResult<()> {
                unimplemented!()
            }
            fn list_invalidated(
                &self,
                _: usize,
                _: usize,
                _: Option<&str>,
            ) -> HyphaeResult<Vec<Memory>> {
                unimplemented!()
            }
            fn search_by_keywords(
                &self,
                _: &[&str],
                _: usize,
                _: usize,
                _: Option<&str>,
            ) -> HyphaeResult<Vec<Memory>> {
                unimplemented!()
            }
            fn search_fts(
                &self,
                _: &str,
                _: usize,
                _: usize,
                _: Option<&str>,
            ) -> HyphaeResult<Vec<Memory>> {
                unimplemented!()
            }
            fn search_fts_in_topic(
                &self,
                _: &str,
                _: &str,
                _: usize,
                _: usize,
                _: Option<&str>,
            ) -> HyphaeResult<Vec<Memory>> {
                unimplemented!()
            }
            fn search_by_embedding(
                &self,
                _: &[f32],
                _: usize,
                _: usize,
                _: Option<&str>,
            ) -> HyphaeResult<Vec<(Memory, f32)>> {
                unimplemented!()
            }
            fn search_hybrid(
                &self,
                _: &str,
                _: &[f32],
                _: usize,
                _: usize,
                _: Option<&str>,
            ) -> HyphaeResult<Vec<(Memory, f32)>> {
                unimplemented!()
            }
            fn update_access(&self, _: &MemoryId) -> HyphaeResult<()> {
                unimplemented!()
            }
            fn apply_decay(&self, _: f32) -> HyphaeResult<usize> {
                unimplemented!()
            }
            fn prune(&self, _: f32) -> HyphaeResult<usize> {
                unimplemented!()
            }
            fn prune_expired(&self) -> HyphaeResult<usize> {
                unimplemented!()
            }
            fn get_by_topic(&self, _: &str, _: Option<&str>) -> HyphaeResult<Vec<Memory>> {
                unimplemented!()
            }
            fn list_topics(&self, _: Option<&str>) -> HyphaeResult<Vec<(String, usize)>> {
                unimplemented!()
            }
            fn consolidate_topic(&self, _: &str, _: Memory) -> HyphaeResult<()> {
                unimplemented!()
            }
            fn count(&self, _: Option<&str>) -> HyphaeResult<usize> {
                unimplemented!()
            }
            fn count_by_topic(&self, _: &str, _: Option<&str>) -> HyphaeResult<usize> {
                unimplemented!()
            }
            fn stats(&self, _: Option<&str>) -> HyphaeResult<StoreStats> {
                unimplemented!()
            }
            fn topic_health(&self, _: &str, _: Option<&str>) -> HyphaeResult<TopicHealth> {
                unimplemented!()
            }

            // Override to simulate a backend that rejects the hook.
            fn on_delegation(&self, _target_agent_id: &str) -> HyphaeResult<()> {
                Err(HyphaeError::Validation(
                    "delegation not supported".to_string(),
                ))
            }
        }

        let store = FailingStore;
        // Callers must isolate hook failures — use unwrap_or_default() or log and proceed.
        store.on_delegation("agent-xyz").unwrap_or(());
    }
}
