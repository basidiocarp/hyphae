use crate::error::HyphaeResult;
use crate::ids::{ConceptId, LinkId, MemoirId, MemoryId};
use crate::memoir::{Concept, ConceptLink, Label, Memoir, MemoirStats, MemoirVersion, Relation};

// ===========================================================================
// Bulk-upsert input types
// ===========================================================================

/// Input for bulk-upserting a concept into a memoir.
#[derive(Debug, Clone)]
pub struct ConceptInput {
    pub name: String,
    pub labels: Vec<Label>,
    pub description: String,
}

/// Input for bulk-upserting a concept link into a memoir.
/// Source and target are identified by concept name within the memoir.
#[derive(Debug, Clone)]
pub struct LinkInput {
    pub source_name: String,
    pub target_name: String,
    pub relation: String,
    pub weight: f32,
}

/// Summary of how many items were created, updated, or left unchanged
/// during a bulk upsert operation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UpsertReport {
    pub created: usize,
    pub updated: usize,
    pub unchanged: usize,
}

pub trait MemoirStore {
    // --- Memoir CRUD ---

    /// Create a memoir and return its ID.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database write fails.
    fn create_memoir(&self, memoir: Memoir) -> HyphaeResult<MemoirId>;

    /// Fetch a memoir by ID.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database query fails.
    fn get_memoir(&self, id: &MemoirId) -> HyphaeResult<Option<Memoir>>;

    /// Fetch a memoir by name.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database query fails.
    fn get_memoir_by_name(&self, name: &str) -> HyphaeResult<Option<Memoir>>;

    /// Update an existing memoir record.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database write fails.
    fn update_memoir(&self, memoir: &Memoir) -> HyphaeResult<()>;

    /// Delete a memoir and cascade to its concepts and links.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database write fails.
    fn delete_memoir(&self, id: &MemoirId) -> HyphaeResult<()>;

    /// List all memoirs.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database query fails.
    fn list_memoirs(&self) -> HyphaeResult<Vec<Memoir>>;

    // --- Concept CRUD ---

    /// Add a concept to a memoir and return its ID.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database write fails.
    fn add_concept(&self, concept: Concept) -> HyphaeResult<ConceptId>;

    /// Fetch a concept by ID.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database query fails.
    fn get_concept(&self, id: &ConceptId) -> HyphaeResult<Option<Concept>>;

    /// Fetch a concept by its name within a memoir.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database query fails.
    fn get_concept_by_name(
        &self,
        memoir_id: &MemoirId,
        name: &str,
    ) -> HyphaeResult<Option<Concept>>;

    /// Update an existing concept record.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database write fails.
    fn update_concept(&self, concept: &Concept) -> HyphaeResult<()>;

    /// Delete a concept and cascade to its links.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database write fails.
    fn delete_concept(&self, id: &ConceptId) -> HyphaeResult<()>;

    // --- Concept Search ---

    /// List all concepts in a memoir.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database query fails.
    fn list_concepts(&self, memoir_id: &MemoirId) -> HyphaeResult<Vec<Concept>>;

    /// List concepts with pagination. Returns `(concepts, has_more)` where `has_more` indicates
    /// whether there are additional pages beyond the current one.
    /// `page_size` is capped at 200 (max), minimum 1.
    ///
    /// # Errors
    ///
    /// Returns `HyphaeError::Database` if the underlying `SQLite` query fails.
    fn list_concepts_paginated(
        &self,
        memoir_id: &MemoirId,
        page_size: usize,
        page: usize,
    ) -> HyphaeResult<(Vec<Concept>, bool)>;

    /// Full-text search over concept names and definitions in a memoir.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database query fails.
    fn search_concepts_fts(
        &self,
        memoir_id: &MemoirId,
        query: &str,
        limit: usize,
    ) -> HyphaeResult<Vec<Concept>>;

    /// Search concepts by label within a memoir.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database query fails.
    fn search_concepts_by_label(
        &self,
        memoir_id: &MemoirId,
        label: &Label,
        limit: usize,
    ) -> HyphaeResult<Vec<Concept>>;

    /// Search concepts across all memoirs via FTS.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database query fails.
    fn search_all_concepts_fts(&self, query: &str, limit: usize) -> HyphaeResult<Vec<Concept>>;

    // --- Refinement ---

    /// Replace the concept definition and append new source memory IDs.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database write fails.
    fn refine_concept(
        &self,
        id: &ConceptId,
        new_definition: &str,
        new_source_ids: &[MemoryId],
    ) -> HyphaeResult<()>;

    /// Replace the concept's definition with a consolidated summary and reset its
    /// revision counter to 0. Called after LLM consolidation fires.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database write fails.
    fn consolidate_concept_definition(
        &self,
        id: &ConceptId,
        new_definition: &str,
    ) -> HyphaeResult<()>;

    // --- Graph ---

    /// Add a link between two concepts and return its ID.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database write fails.
    fn add_link(&self, link: ConceptLink) -> HyphaeResult<LinkId>;

    /// Fetch all links originating from a concept.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database query fails.
    fn get_links_from(&self, concept_id: &ConceptId) -> HyphaeResult<Vec<ConceptLink>>;

    /// Fetch all links pointing to a concept.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database query fails.
    fn get_links_to(&self, concept_id: &ConceptId) -> HyphaeResult<Vec<ConceptLink>>;

    /// Permanently delete a link by ID.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database write fails.
    fn delete_link(&self, id: &LinkId) -> HyphaeResult<()>;

    /// Mark a link as invalid without deleting it.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database write fails.
    fn invalidate_link(&self, id: &LinkId) -> HyphaeResult<()>;

    /// Remove a link identified by source name, target name, and relation string.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database write fails or the link is not found.
    fn remove_link(
        &self,
        memoir_id: &MemoirId,
        from_concept: &str,
        to_concept: &str,
        relation: &str,
    ) -> HyphaeResult<()>;

    /// Fetch all concepts directly connected to a concept, optionally filtered by relation.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database query fails.
    fn get_neighbors(
        &self,
        concept_id: &ConceptId,
        relation: Option<Relation>,
    ) -> HyphaeResult<Vec<Concept>>;

    /// Fetch the `depth`-hop neighborhood around a concept as `(concepts, links)`.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database query fails.
    fn get_neighborhood(
        &self,
        concept_id: &ConceptId,
        depth: usize,
    ) -> HyphaeResult<(Vec<Concept>, Vec<ConceptLink>)>;

    /// List all concept links within a memoir.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database query fails.
    fn list_all_links(&self, memoir_id: &MemoirId) -> HyphaeResult<Vec<ConceptLink>>;

    /// Set or clear the community ID for a concept.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database write fails.
    fn set_concept_community(
        &self,
        concept_id: &ConceptId,
        community_id: Option<&str>,
    ) -> HyphaeResult<()>;

    // --- Stats ---

    /// Return aggregate statistics for all concepts and links in a memoir.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database query fails.
    fn memoir_stats(&self, memoir_id: &MemoirId) -> HyphaeResult<MemoirStats>;

    // --- Bulk upsert ---

    /// Upsert concepts by `(memoir_id, name)` — create if absent, update
    /// definition/labels if changed, skip if identical.  The entire batch
    /// runs inside a single transaction.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database transaction fails.
    fn upsert_concepts(
        &self,
        memoir_id: &MemoirId,
        concepts: &[ConceptInput],
    ) -> HyphaeResult<UpsertReport>;

    /// Upsert concept links by `(source_id, target_id, relation)` — create
    /// if absent, update weight if changed, skip if identical.  Concept
    /// names are resolved to IDs within the memoir.  The entire batch runs
    /// inside a single transaction.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database transaction fails or concept names
    /// cannot be resolved.
    fn upsert_links(&self, memoir_id: &MemoirId, links: &[LinkInput])
    -> HyphaeResult<UpsertReport>;

    /// Delete every concept in `memoir_id` whose name is NOT in
    /// `keep_names`.  Cascades to orphaned links via `ON DELETE CASCADE`.
    /// Returns the number of concepts deleted.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database write fails.
    fn prune_concepts(&self, memoir_id: &MemoirId, keep_names: &[String]) -> HyphaeResult<usize>;

    // --- Memoir versioning ---

    /// Store a version snapshot for a memoir.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database write fails.
    fn store_memoir_version(&self, version: MemoirVersion) -> HyphaeResult<()>;

    /// Retrieve the version history for a memoir, most recent first.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database query fails.
    fn get_memoir_history(
        &self,
        memoir_id: &MemoirId,
        limit: usize,
    ) -> HyphaeResult<Vec<MemoirVersion>>;
}
