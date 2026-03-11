use crate::error::HyphaeResult;
use crate::ids::{ConceptId, LinkId, MemoirId, MemoryId};
use crate::memoir::{Concept, ConceptLink, Label, Memoir, MemoirStats, Relation};

pub trait MemoirStore {
    // --- Memoir CRUD ---
    fn create_memoir(&self, memoir: Memoir) -> HyphaeResult<MemoirId>;
    fn get_memoir(&self, id: &MemoirId) -> HyphaeResult<Option<Memoir>>;
    fn get_memoir_by_name(&self, name: &str) -> HyphaeResult<Option<Memoir>>;
    fn update_memoir(&self, memoir: &Memoir) -> HyphaeResult<()>;
    fn delete_memoir(&self, id: &MemoirId) -> HyphaeResult<()>;
    fn list_memoirs(&self) -> HyphaeResult<Vec<Memoir>>;

    // --- Concept CRUD ---
    fn add_concept(&self, concept: Concept) -> HyphaeResult<ConceptId>;
    fn get_concept(&self, id: &ConceptId) -> HyphaeResult<Option<Concept>>;
    fn get_concept_by_name(
        &self,
        memoir_id: &MemoirId,
        name: &str,
    ) -> HyphaeResult<Option<Concept>>;
    fn update_concept(&self, concept: &Concept) -> HyphaeResult<()>;
    fn delete_concept(&self, id: &ConceptId) -> HyphaeResult<()>;

    // --- Concept Search ---
    fn list_concepts(&self, memoir_id: &MemoirId) -> HyphaeResult<Vec<Concept>>;
    fn search_concepts_fts(
        &self,
        memoir_id: &MemoirId,
        query: &str,
        limit: usize,
    ) -> HyphaeResult<Vec<Concept>>;
    fn search_concepts_by_label(
        &self,
        memoir_id: &MemoirId,
        label: &Label,
        limit: usize,
    ) -> HyphaeResult<Vec<Concept>>;

    /// Search concepts across all memoirs via FTS.
    fn search_all_concepts_fts(&self, query: &str, limit: usize) -> HyphaeResult<Vec<Concept>>;

    // --- Refinement ---
    fn refine_concept(
        &self,
        id: &ConceptId,
        new_definition: &str,
        new_source_ids: &[MemoryId],
    ) -> HyphaeResult<()>;

    // --- Graph ---
    fn add_link(&self, link: ConceptLink) -> HyphaeResult<LinkId>;
    fn get_links_from(&self, concept_id: &ConceptId) -> HyphaeResult<Vec<ConceptLink>>;
    fn get_links_to(&self, concept_id: &ConceptId) -> HyphaeResult<Vec<ConceptLink>>;
    fn delete_link(&self, id: &LinkId) -> HyphaeResult<()>;
    fn get_neighbors(
        &self,
        concept_id: &ConceptId,
        relation: Option<Relation>,
    ) -> HyphaeResult<Vec<Concept>>;
    fn get_neighborhood(
        &self,
        concept_id: &ConceptId,
        depth: usize,
    ) -> HyphaeResult<(Vec<Concept>, Vec<ConceptLink>)>;

    // --- Stats ---
    fn memoir_stats(&self, memoir_id: &MemoirId) -> HyphaeResult<MemoirStats>;
}
