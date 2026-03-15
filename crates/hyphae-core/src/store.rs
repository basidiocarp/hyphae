use crate::error::HyphaeResult;
use crate::ids::MemoryId;
use crate::memory::{Memory, StoreStats, TopicHealth};

pub trait MemoryStore {
    // CRUD
    fn store(&self, memory: Memory) -> HyphaeResult<MemoryId>;
    fn get(&self, id: &MemoryId) -> HyphaeResult<Option<Memory>>;
    fn update(&self, memory: &Memory) -> HyphaeResult<()>;
    fn delete(&self, id: &MemoryId) -> HyphaeResult<()>;

    // Search
    fn search_by_keywords(
        &self,
        keywords: &[&str],
        limit: usize,
        project: Option<&str>,
    ) -> HyphaeResult<Vec<Memory>>;
    fn search_fts(
        &self,
        query: &str,
        limit: usize,
        project: Option<&str>,
    ) -> HyphaeResult<Vec<Memory>>;
    fn search_by_embedding(
        &self,
        embedding: &[f32],
        limit: usize,
        project: Option<&str>,
    ) -> HyphaeResult<Vec<(Memory, f32)>>;
    fn search_hybrid(
        &self,
        query: &str,
        embedding: &[f32],
        limit: usize,
        project: Option<&str>,
    ) -> HyphaeResult<Vec<(Memory, f32)>>;

    // Lifecycle
    fn update_access(&self, id: &MemoryId) -> HyphaeResult<()>;
    fn apply_decay(&self, decay_factor: f32) -> HyphaeResult<usize>;
    fn prune(&self, weight_threshold: f32) -> HyphaeResult<usize>;

    // Organization
    fn get_by_topic(&self, topic: &str, project: Option<&str>) -> HyphaeResult<Vec<Memory>>;
    fn list_topics(&self, project: Option<&str>) -> HyphaeResult<Vec<(String, usize)>>;
    fn consolidate_topic(&self, topic: &str, consolidated: Memory) -> HyphaeResult<()>;

    // Stats
    fn count(&self, project: Option<&str>) -> HyphaeResult<usize>;
    fn count_by_topic(&self, topic: &str, project: Option<&str>) -> HyphaeResult<usize>;
    fn stats(&self, project: Option<&str>) -> HyphaeResult<StoreStats>;
    fn topic_health(&self, topic: &str, project: Option<&str>) -> HyphaeResult<TopicHealth>;
}
