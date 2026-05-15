use std::collections::{HashMap, HashSet};

use chrono::Utc;
use rusqlite::{OptionalExtension, params};

use hyphae_core::{
    Concept, ConceptId, ConceptInput, ConceptLink, HyphaeError, HyphaeResult, Label, LinkId,
    LinkInput, Memoir, MemoirId, MemoirStats, MemoirStore, MemoryId, Relation, UpsertReport,
};

use super::SqliteStore;
use super::helpers::{
    CONCEPT_COLS, LINK_COLS, MEMOIR_COLS, row_to_concept, row_to_link, row_to_memoir,
};
use super::search::sanitize_fts_query;

// ─────────────────────────────────────────────────────────────────────────────
// Relation Normalization
// ─────────────────────────────────────────────────────────────────────────────

/// Normalize a freeform relation string to a canonical Relation enum value.
/// Maps synonyms and freeform text to one of the 9 canonical relation types.
pub(crate) fn normalize_relation(relation: &str) -> String {
    // Parse as Relation, which handles all normalization via FromStr
    match relation.parse::<Relation>() {
        Ok(r) => r.to_string(),
        // Fallback: if unknown, return as lowercase for graceful degradation
        Err(_) => relation.to_lowercase(),
    }
}

impl MemoirStore for SqliteStore {
    fn create_memoir(&self, memoir: Memoir) -> HyphaeResult<MemoirId> {
        self.conn
            .execute(
                "INSERT INTO memoirs (id, name, description, created_at, updated_at, consolidation_threshold, author, git_hash, parent_version_id, decay, authority, source, compiled_at, invalidated_at, invalidated_by, freshness_ttl_secs)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
                params![
                    memoir.id.as_ref(),
                    memoir.name,
                    memoir.description,
                    memoir.created_at.to_rfc3339(),
                    memoir.updated_at.to_rfc3339(),
                    memoir.consolidation_threshold,
                    memoir.author,
                    memoir.git_hash,
                    memoir.parent_version_id,
                    format!("{:?}", memoir.meta.decay).to_lowercase(),
                    format!("{:?}", memoir.meta.authority).to_lowercase(),
                    format!("{:?}", memoir.meta.source).to_lowercase(),
                    memoir.meta.compiled_at.map(|dt| dt.to_rfc3339()),
                    memoir.meta.invalidated_at.map(|dt| dt.to_rfc3339()),
                    &memoir.meta.invalidated_by,
                    memoir.meta.freshness_ttl_secs.map(|v| v as i64),
                ],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let version = hyphae_core::MemoirVersion {
            version_id: hyphae_core::MemoryId::new().to_string(),
            memoir_id: memoir.id.clone(),
            version_seq: 1,
            author: memoir.author.clone(),
            git_hash: memoir.git_hash.clone(),
            diff_summary: "memoir created".to_string(),
            created_at: Utc::now(),
        };
        self.store_memoir_version(version)?;

        Ok(memoir.id)
    }

    fn get_memoir(&self, id: &MemoirId) -> HyphaeResult<Option<Memoir>> {
        let sql = format!("SELECT {MEMOIR_COLS} FROM memoirs WHERE id = ?1");
        let mut stmt = self
            .conn
            .prepare_cached(&sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        stmt.query_row(params![id.as_ref()], row_to_memoir)
            .optional()
            .map_err(|e| HyphaeError::Database(e.to_string()))
    }

    fn get_memoir_by_name(&self, name: &str) -> HyphaeResult<Option<Memoir>> {
        let sql = format!("SELECT {MEMOIR_COLS} FROM memoirs WHERE name = ?1");
        let mut stmt = self
            .conn
            .prepare_cached(&sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        stmt.query_row(params![name], row_to_memoir)
            .optional()
            .map_err(|e| HyphaeError::Database(e.to_string()))
    }

    fn update_memoir(&self, memoir: &Memoir) -> HyphaeResult<()> {
        let changed = self
            .conn
            .execute(
                "UPDATE memoirs SET name = ?2, description = ?3, updated_at = ?4,
                 consolidation_threshold = ?5, author = ?6, git_hash = ?7, parent_version_id = ?8,
                 decay = ?9, authority = ?10, source = ?11, compiled_at = ?12, invalidated_at = ?13, invalidated_by = ?14, freshness_ttl_secs = ?15
                 WHERE id = ?1",
                params![
                    memoir.id.as_ref(),
                    memoir.name,
                    memoir.description,
                    memoir.updated_at.to_rfc3339(),
                    memoir.consolidation_threshold,
                    memoir.author,
                    memoir.git_hash,
                    memoir.parent_version_id,
                    format!("{:?}", memoir.meta.decay).to_lowercase(),
                    format!("{:?}", memoir.meta.authority).to_lowercase(),
                    format!("{:?}", memoir.meta.source).to_lowercase(),
                    memoir.meta.compiled_at.map(|dt| dt.to_rfc3339()),
                    memoir.meta.invalidated_at.map(|dt| dt.to_rfc3339()),
                    &memoir.meta.invalidated_by,
                    memoir.meta.freshness_ttl_secs.map(|v| v as i64),
                ],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        if changed == 0 {
            return Err(HyphaeError::NotFound(memoir.id.to_string()));
        }

        let next_seq: u32 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(version_seq), 0) + 1 FROM memoir_versions WHERE memoir_id = ?1",
                params![memoir.id.as_ref()],
                |row| row.get(0),
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let version = hyphae_core::MemoirVersion {
            version_id: hyphae_core::MemoryId::new().to_string(),
            memoir_id: memoir.id.clone(),
            version_seq: next_seq,
            author: memoir.author.clone(),
            git_hash: memoir.git_hash.clone(),
            diff_summary: "memoir updated".to_string(),
            created_at: Utc::now(),
        };
        self.store_memoir_version(version)?;

        Ok(())
    }

    fn delete_memoir(&self, id: &MemoirId) -> HyphaeResult<()> {
        let changed = self
            .conn
            .execute("DELETE FROM memoirs WHERE id = ?1", params![id.as_ref()])
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        if changed == 0 {
            return Err(HyphaeError::NotFound(id.to_string()));
        }
        Ok(())
    }

    fn list_memoirs(&self) -> HyphaeResult<Vec<Memoir>> {
        let sql = format!("SELECT {MEMOIR_COLS} FROM memoirs ORDER BY name");
        let mut stmt = self
            .conn
            .prepare_cached(&sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let rows = stmt
            .query_map([], row_to_memoir)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| HyphaeError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    fn add_concept(&self, concept: Concept) -> HyphaeResult<ConceptId> {
        let labels_json = serde_json::to_string(&concept.labels)?;
        let source_ids_json = serde_json::to_string(&concept.source_memory_ids)?;
        let block_type_str = concept.block_type.and_then(|bt| {
            serde_json::to_value(bt)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
        });

        self.conn
            .execute(
                "INSERT INTO concepts (id, memoir_id, name, definition, labels, confidence,
                 revision, created_at, updated_at, source_memory_ids, abstract_text, overview_text, block_type)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
                params![
                    concept.id.as_ref(),
                    concept.memoir_id.as_ref(),
                    concept.name,
                    concept.definition,
                    labels_json,
                    concept.confidence.value(),
                    concept.revision,
                    concept.created_at.to_rfc3339(),
                    concept.updated_at.to_rfc3339(),
                    source_ids_json,
                    concept.abstract_text,
                    concept.overview_text,
                    block_type_str,
                ],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        Ok(concept.id)
    }

    fn get_concept(&self, id: &ConceptId) -> HyphaeResult<Option<Concept>> {
        let sql = format!("SELECT {CONCEPT_COLS} FROM concepts WHERE id = ?1");
        let mut stmt = self
            .conn
            .prepare_cached(&sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        stmt.query_row(params![id.as_ref()], row_to_concept)
            .optional()
            .map_err(|e| HyphaeError::Database(e.to_string()))
    }

    fn get_concept_by_name(
        &self,
        memoir_id: &MemoirId,
        name: &str,
    ) -> HyphaeResult<Option<Concept>> {
        let sql = format!("SELECT {CONCEPT_COLS} FROM concepts WHERE memoir_id = ?1 AND name = ?2");
        let mut stmt = self
            .conn
            .prepare_cached(&sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        stmt.query_row(params![memoir_id.as_ref(), name], row_to_concept)
            .optional()
            .map_err(|e| HyphaeError::Database(e.to_string()))
    }

    fn update_concept(&self, concept: &Concept) -> HyphaeResult<()> {
        let labels_json = serde_json::to_string(&concept.labels)?;
        let source_ids_json = serde_json::to_string(&concept.source_memory_ids)?;
        let block_type_str = concept.block_type.and_then(|bt| {
            serde_json::to_value(bt)
                .ok()
                .and_then(|v| v.as_str().map(str::to_string))
        });

        let changed = self
            .conn
            .execute(
                // community_id is intentionally excluded — use set_concept_community() to change it.
                "UPDATE concepts SET memoir_id = ?2, name = ?3, definition = ?4, labels = ?5,
                 confidence = ?6, revision = ?7, updated_at = ?8, source_memory_ids = ?9,
                 abstract_text = ?10, overview_text = ?11, block_type = ?12
                 WHERE id = ?1",
                params![
                    concept.id.as_ref(),
                    concept.memoir_id.as_ref(),
                    concept.name,
                    concept.definition,
                    labels_json,
                    concept.confidence.value(),
                    concept.revision,
                    concept.updated_at.to_rfc3339(),
                    source_ids_json,
                    concept.abstract_text,
                    concept.overview_text,
                    block_type_str,
                ],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        if changed == 0 {
            return Err(HyphaeError::NotFound(concept.id.to_string()));
        }
        Ok(())
    }

    fn delete_concept(&self, id: &ConceptId) -> HyphaeResult<()> {
        // Fetch the concept before deletion so we can invalidate its paired memory entry
        let concept = self
            .get_concept(id)?
            .ok_or_else(|| HyphaeError::NotFound(id.to_string()))?;

        // SAFETY: No nested transactions — this method does not call other &self methods
        // that open transactions. The &self receiver is required by the MemoirStore trait.
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let changed = tx
            .execute("DELETE FROM concepts WHERE id = ?1", params![id.as_ref()])
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        if changed == 0 {
            return Err(HyphaeError::NotFound(id.to_string()));
        }

        // Invalidate the dual-written memory entry using the stable ID format
        // memoir-{memoir_id}-{concept_name}
        let memory_id: MemoryId = format!("memoir-{}-{}", concept.memoir_id, concept.name).into();
        let now = Utc::now().to_rfc3339();

        if let Err(e) = tx.execute(
            "UPDATE memories SET invalidated_at = ?1, invalidation_reason = 'concept_deleted'
             WHERE id = ?2",
            params![now, memory_id.as_ref()],
        ) {
            tracing::warn!("failed to invalidate paired memory for deleted concept {id}: {e}");
        }

        tx.commit()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        Ok(())
    }

    fn list_concepts(&self, memoir_id: &MemoirId) -> HyphaeResult<Vec<Concept>> {
        let sql = format!("SELECT {CONCEPT_COLS} FROM concepts WHERE memoir_id = ?1 ORDER BY name");
        let mut stmt = self
            .conn
            .prepare_cached(&sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        let rows = stmt
            .query_map(params![memoir_id.as_ref()], row_to_concept)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        rows.map(|r| r.map_err(|e| HyphaeError::Database(e.to_string())))
            .collect()
    }
    fn list_concepts_paginated(
        &self,
        memoir_id: &MemoirId,
        page_size: usize,
        page: usize,
    ) -> HyphaeResult<(Vec<Concept>, bool)> {
        let capped_page_size = page_size.min(200).max(1);
        let offset = page * capped_page_size;

        let sql = format!(
            "SELECT {CONCEPT_COLS} FROM concepts WHERE memoir_id = ?1 ORDER BY name LIMIT ? OFFSET ?"
        );
        let mut stmt = self
            .conn
            .prepare_cached(&sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(
                params![
                    memoir_id.as_ref(),
                    (capped_page_size + 1) as i64,
                    offset as i64
                ],
                row_to_concept,
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for (idx, row) in rows.enumerate() {
            if idx >= capped_page_size {
                // We fetched one extra to detect if there are more pages
                return Ok((results, true));
            }
            results.push(row.map_err(|e| HyphaeError::Database(e.to_string()))?);
        }
        Ok((results, false))
    }

    fn search_concepts_fts(
        &self,
        memoir_id: &MemoirId,
        query: &str,
        limit: usize,
    ) -> HyphaeResult<Vec<Concept>> {
        let sanitized = sanitize_fts_query(query);
        if sanitized.is_empty() {
            return Ok(Vec::new());
        }

        let sql = "SELECT c.id, c.memoir_id, c.name, c.definition, c.labels, c.confidence,
                    c.revision, c.created_at, c.updated_at, c.source_memory_ids, c.community_id,
                    c.abstract_text, c.overview_text, c.block_type
             FROM concepts c
             JOIN concepts_fts fts ON c.rowid = fts.rowid
             WHERE c.memoir_id = ?1
               AND concepts_fts MATCH ?2
             ORDER BY fts.rank, c.name ASC
             LIMIT ?3"
            .to_string();

        let mut stmt = self
            .conn
            .prepare_cached(&sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(
                params![memoir_id.as_ref(), sanitized, limit as i64],
                row_to_concept,
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| HyphaeError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    fn search_all_concepts_fts(&self, query: &str, limit: usize) -> HyphaeResult<Vec<Concept>> {
        let sanitized = sanitize_fts_query(query);
        if sanitized.is_empty() {
            return Ok(Vec::new());
        }

        let sql = "SELECT c.id, c.memoir_id, c.name, c.definition, c.labels, c.confidence,
                    c.revision, c.created_at, c.updated_at, c.source_memory_ids, c.community_id,
                    c.abstract_text, c.overview_text, c.block_type
             FROM concepts c
             JOIN concepts_fts fts ON c.rowid = fts.rowid
             WHERE concepts_fts MATCH ?1
             ORDER BY fts.rank, c.name ASC
             LIMIT ?2"
            .to_string();

        let mut stmt = self
            .conn
            .prepare_cached(&sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![sanitized, limit as i64], row_to_concept)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| HyphaeError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    fn search_concepts_by_label(
        &self,
        memoir_id: &MemoirId,
        label: &Label,
        limit: usize,
    ) -> HyphaeResult<Vec<Concept>> {
        let sql = format!(
            "SELECT {CONCEPT_COLS} FROM concepts
             WHERE memoir_id = ?1
               AND EXISTS (
                   SELECT 1 FROM json_each(labels) AS j
                   WHERE json_extract(j.value, '$.namespace') = ?2
                     AND json_extract(j.value, '$.value') = ?3
               )
             ORDER BY confidence DESC
             LIMIT ?4"
        );

        let mut stmt = self
            .conn
            .prepare_cached(&sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(
                params![
                    memoir_id.as_ref(),
                    label.namespace,
                    label.value,
                    limit as i64
                ],
                row_to_concept,
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| HyphaeError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    fn refine_concept(
        &self,
        id: &ConceptId,
        new_definition: &str,
        new_source_ids: &[MemoryId],
    ) -> HyphaeResult<()> {
        let concept = self
            .get_concept(id)?
            .ok_or_else(|| HyphaeError::NotFound(id.to_string()))?;

        let mut seen: HashSet<String> = concept
            .source_memory_ids
            .iter()
            .map(|id| id.to_string())
            .collect();
        let mut merged_sources = concept.source_memory_ids;
        for sid in new_source_ids {
            if seen.insert(sid.to_string()) {
                merged_sources.push(sid.clone());
            }
        }
        let source_ids_json = serde_json::to_string(&merged_sources)?;

        let now = Utc::now().to_rfc3339();
        let new_confidence = (concept.confidence.value() + 0.1).min(1.0);

        self.conn
            .execute(
                "UPDATE concepts SET definition = ?2, revision = revision + 1,
                 confidence = ?3, updated_at = ?4, source_memory_ids = ?5
                 WHERE id = ?1",
                params![
                    id.as_ref(),
                    new_definition,
                    new_confidence,
                    now,
                    source_ids_json
                ],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        Ok(())
    }

    fn consolidate_concept_definition(
        &self,
        id: &ConceptId,
        new_definition: &str,
    ) -> HyphaeResult<()> {
        let now = Utc::now().to_rfc3339();
        let changed = self
            .conn
            .execute(
                "UPDATE concepts SET definition = ?2, revision = 0, updated_at = ?3 WHERE id = ?1",
                params![id.as_ref(), new_definition, now],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        if changed == 0 {
            return Err(HyphaeError::NotFound(id.to_string()));
        }
        Ok(())
    }

    fn add_link(&self, link: ConceptLink) -> HyphaeResult<LinkId> {
        let normalized_relation = normalize_relation(&link.relation.to_string());
        self.conn
            .execute(
                "INSERT INTO concept_links (id, source_id, target_id, relation, weight, link_count, created_at, valid_from, valid_to)
                 VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?7, ?8)
                 ON CONFLICT(source_id, target_id, relation) DO UPDATE SET
                    link_count = link_count + 1,
                    valid_from = excluded.valid_from,
                    valid_to = NULL",
                params![
                    link.id.as_ref(),
                    link.source_id.as_ref(),
                    link.target_id.as_ref(),
                    normalized_relation,
                    link.weight.value(),
                    link.created_at.to_rfc3339(),
                    link.valid_from.to_rfc3339(),
                    link.valid_to.map(|dt| dt.to_rfc3339()),
                ],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        Ok(link.id)
    }

    fn get_links_from(&self, concept_id: &ConceptId) -> HyphaeResult<Vec<ConceptLink>> {
        let sql = format!(
            "SELECT {LINK_COLS} FROM concept_links WHERE source_id = ?1 AND (valid_to IS NULL OR valid_to = '')"
        );
        let mut stmt = self
            .conn
            .prepare_cached(&sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![concept_id.as_ref()], row_to_link)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| HyphaeError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    fn get_links_to(&self, concept_id: &ConceptId) -> HyphaeResult<Vec<ConceptLink>> {
        let sql = format!(
            "SELECT {LINK_COLS} FROM concept_links WHERE target_id = ?1 AND (valid_to IS NULL OR valid_to = '')"
        );
        let mut stmt = self
            .conn
            .prepare_cached(&sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![concept_id.as_ref()], row_to_link)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| HyphaeError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    fn delete_link(&self, id: &LinkId) -> HyphaeResult<()> {
        let changed = self
            .conn
            .execute(
                "DELETE FROM concept_links WHERE id = ?1",
                params![id.as_ref()],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        if changed == 0 {
            return Err(HyphaeError::NotFound(id.to_string()));
        }
        Ok(())
    }

    fn invalidate_link(&self, id: &LinkId) -> HyphaeResult<()> {
        let now = Utc::now().to_rfc3339();
        // SAFETY: No nested transactions — this method does not call other &self methods
        // that open transactions. The &self receiver is required by the MemoirStore trait.
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let changed = tx
            .execute(
                "UPDATE concept_links SET valid_to = ?1 WHERE id = ?2 AND (valid_to IS NULL OR valid_to = '')",
                params![now, id.as_ref()],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        if changed == 0 {
            return Err(HyphaeError::NotFound(format!(
                "link not found or already invalidated: {id}"
            )));
        }
        // Also invalidate the reverse-direction edge if it exists. Invalidation is
        // symmetric because get_neighbors walks both directions, so a stale reverse
        // edge would still surface the concept after the forward edge is invalidated.
        tx.execute(
            "UPDATE concept_links SET valid_to = ?1
             WHERE source_id = (SELECT target_id FROM concept_links WHERE id = ?2)
               AND target_id = (SELECT source_id FROM concept_links WHERE id = ?2)
               AND relation  = (SELECT relation  FROM concept_links WHERE id = ?2)
               AND (valid_to IS NULL OR valid_to = '')",
            params![now, id.as_ref()],
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

        tx.commit()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        Ok(())
    }

    fn remove_link(
        &self,
        memoir_id: &MemoirId,
        from_concept: &str,
        to_concept: &str,
        relation: &str,
    ) -> HyphaeResult<()> {
        let from = self
            .get_concept_by_name(memoir_id, from_concept)?
            .ok_or_else(|| HyphaeError::NotFound(format!("concept not found: {from_concept}")))?;
        let to = self
            .get_concept_by_name(memoir_id, to_concept)?
            .ok_or_else(|| HyphaeError::NotFound(format!("concept not found: {to_concept}")))?;
        let normalized = normalize_relation(relation);
        let changed = self
            .conn
            .execute(
                "DELETE FROM concept_links WHERE source_id = ?1 AND target_id = ?2 AND relation = ?3",
                params![from.id.as_ref(), to.id.as_ref(), normalized],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        if changed == 0 {
            return Err(HyphaeError::NotFound(format!(
                "no '{relation}' link from '{from_concept}' to '{to_concept}'"
            )));
        }
        // remove_link is symmetric: also remove the reverse edge if one exists.
        // add_link only inserts the forward direction, but edges can be manually
        // inserted or copied in both directions. get_neighbors walks both directions,
        // so a stale reverse edge would continue surfacing the concept after removal.
        let _ = self.conn.execute(
            "DELETE FROM concept_links WHERE source_id = ?1 AND target_id = ?2 AND relation = ?3",
            params![to.id.as_ref(), from.id.as_ref(), normalized],
        );
        Ok(())
    }

    fn get_neighbors(
        &self,
        concept_id: &ConceptId,
        relation: Option<Relation>,
    ) -> HyphaeResult<Vec<Concept>> {
        let (sql, p_relation);

        let base = format!(
            "SELECT {CONCEPT_COLS} FROM concepts WHERE id IN (
                SELECT target_id FROM concept_links WHERE source_id = ?1 AND (valid_to IS NULL OR valid_to = '') {{filter}}
                UNION
                SELECT source_id FROM concept_links WHERE target_id = ?1 AND (valid_to IS NULL OR valid_to = '') {{filter}}
            )"
        );

        if let Some(ref r) = relation {
            p_relation = r.to_string();
            let filtered = base.replace("{filter}", "AND relation = ?2");
            sql = filtered;
        } else {
            p_relation = String::new();
            sql = base.replace("{filter}", "");
        };

        let mut stmt = self
            .conn
            .prepare_cached(&sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let rows = if relation.is_some() {
            stmt.query_map(params![concept_id.as_ref(), p_relation], row_to_concept)
                .map_err(|e| HyphaeError::Database(e.to_string()))?
        } else {
            stmt.query_map(params![concept_id.as_ref()], row_to_concept)
                .map_err(|e| HyphaeError::Database(e.to_string()))?
        };

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| HyphaeError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    fn get_neighborhood(
        &self,
        concept_id: &ConceptId,
        depth: usize,
    ) -> HyphaeResult<(Vec<Concept>, Vec<ConceptLink>)> {
        const MAX_NODES: usize = 200;

        let mut visited: HashSet<String> = HashSet::new();
        let mut all_links: Vec<ConceptLink> = Vec::new();

        // Verify root exists
        if self.get_concept(concept_id)?.is_none() {
            return Err(HyphaeError::NotFound(concept_id.to_string()));
        }

        // Cap depth at 10 to prevent runaway traversals
        let capped_depth = depth.min(10);

        visited.insert(concept_id.to_string());
        let mut frontier: Vec<String> = vec![concept_id.to_string()];

        for _ in 0..capped_depth {
            if frontier.is_empty() || visited.len() >= MAX_NODES {
                break;
            }

            let placeholders: String = (1..=frontier.len())
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(", ");

            // Batch-fetch outgoing links for all frontier nodes
            let outgoing_sql = format!(
                "SELECT {LINK_COLS} FROM concept_links WHERE source_id IN ({placeholders}) AND (valid_to IS NULL OR valid_to = '')"
            );
            let params: Vec<&dyn rusqlite::types::ToSql> = frontier
                .iter()
                .map(|s| s as &dyn rusqlite::types::ToSql)
                .collect();
            let mut stmt = self
                .conn
                .prepare(&outgoing_sql)
                .map_err(|e| HyphaeError::Database(e.to_string()))?;
            let outgoing: Vec<ConceptLink> = stmt
                .query_map(params.as_slice(), row_to_link)
                .map_err(|e| HyphaeError::Database(e.to_string()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| HyphaeError::Database(e.to_string()))?;

            // Batch-fetch incoming links for all frontier nodes
            let incoming_sql = format!(
                "SELECT {LINK_COLS} FROM concept_links WHERE target_id IN ({placeholders}) AND (valid_to IS NULL OR valid_to = '')"
            );
            let mut stmt = self
                .conn
                .prepare(&incoming_sql)
                .map_err(|e| HyphaeError::Database(e.to_string()))?;
            let incoming: Vec<ConceptLink> = stmt
                .query_map(params.as_slice(), row_to_link)
                .map_err(|e| HyphaeError::Database(e.to_string()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| HyphaeError::Database(e.to_string()))?;

            let mut next_frontier = Vec::new();

            for link in outgoing {
                if visited.len() < MAX_NODES && visited.insert(link.target_id.to_string()) {
                    next_frontier.push(link.target_id.to_string());
                }
                all_links.push(link);
            }

            for link in incoming {
                if visited.len() < MAX_NODES && visited.insert(link.source_id.to_string()) {
                    next_frontier.push(link.source_id.to_string());
                }
                all_links.push(link);
            }

            frontier = next_frontier;
        }

        // Batch-fetch all visited concepts in one query
        let all_ids: Vec<String> = visited.into_iter().collect();
        let placeholders: String = (1..=all_ids.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let concept_sql =
            format!("SELECT {CONCEPT_COLS} FROM concepts WHERE id IN ({placeholders})");
        let mut stmt = self
            .conn
            .prepare(&concept_sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        let params: Vec<&dyn rusqlite::types::ToSql> = all_ids
            .iter()
            .map(|s| s as &dyn rusqlite::types::ToSql)
            .collect();
        let concepts: Vec<Concept> = stmt
            .query_map(params.as_slice(), row_to_concept)
            .map_err(|e| HyphaeError::Database(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        Ok((concepts, all_links))
    }

    fn upsert_concepts(
        &self,
        memoir_id: &MemoirId,
        concepts: &[ConceptInput],
    ) -> HyphaeResult<UpsertReport> {
        // SAFETY: No nested transactions — this method does not call other &self methods
        // that open transactions. The &self receiver is required by the MemoirStore trait.
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut report = UpsertReport::default();

        for input in concepts {
            let existing = self.get_concept_by_name(memoir_id, &input.name)?;

            if let Some(concept) = existing {
                let same_definition = concept.definition == input.description;
                let same_labels = concept.labels == input.labels;

                if same_definition && same_labels {
                    report.unchanged += 1;
                } else {
                    let updated = Concept {
                        definition: input.description.clone(),
                        labels: input.labels.clone(),
                        revision: concept.revision + 1,
                        updated_at: Utc::now(),
                        ..concept
                    };
                    self.update_concept(&updated)?;
                    report.updated += 1;
                }
            } else {
                let mut concept = Concept::new(
                    memoir_id.clone(),
                    input.name.clone(),
                    input.description.clone(),
                );
                concept.labels = input.labels.clone();
                self.add_concept(concept)?;
                report.created += 1;
            }
        }

        tx.commit()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        Ok(report)
    }

    fn upsert_links(
        &self,
        memoir_id: &MemoirId,
        links: &[LinkInput],
    ) -> HyphaeResult<UpsertReport> {
        // Build name → ConceptId map up-front (one query)
        let all_concepts = self.list_concepts(memoir_id)?;
        let name_to_id: HashMap<&str, &ConceptId> = all_concepts
            .iter()
            .map(|c| (c.name.as_str(), &c.id))
            .collect();

        // SAFETY: No nested transactions — this method does not call other &self methods
        // that open transactions. The &self receiver is required by the MemoirStore trait.
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut report = UpsertReport::default();

        for input in links {
            let source_id = name_to_id
                .get(input.source_name.as_str())
                .ok_or_else(|| HyphaeError::NotFound(format!("concept '{}'", input.source_name)))?;
            let target_id = name_to_id
                .get(input.target_name.as_str())
                .ok_or_else(|| HyphaeError::NotFound(format!("concept '{}'", input.target_name)))?;

            let relation_str = input.relation.to_lowercase();

            // Look for an existing link with the same (source, target, relation)
            let existing: Option<(String, f32)> = self
                .conn
                .query_row(
                    "SELECT id, weight FROM concept_links
                     WHERE source_id = ?1 AND target_id = ?2 AND relation = ?3",
                    params![source_id.as_ref(), target_id.as_ref(), relation_str],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, f32>(1)?)),
                )
                .optional()
                .map_err(|e| HyphaeError::Database(e.to_string()))?;

            if let Some((existing_id, existing_weight)) = existing {
                let weight_changed = (existing_weight - input.weight).abs() > f32::EPSILON;
                if weight_changed {
                    self.conn
                        .execute(
                            "UPDATE concept_links SET weight = ?2 WHERE id = ?1",
                            params![existing_id, input.weight],
                        )
                        .map_err(|e| HyphaeError::Database(e.to_string()))?;
                    report.updated += 1;
                } else {
                    report.unchanged += 1;
                }
            } else {
                let relation: Relation = relation_str.parse().unwrap_or(Relation::RelatedTo);
                let mut link =
                    ConceptLink::new((*source_id).clone(), (*target_id).clone(), relation);
                link.weight = hyphae_core::Weight::new_clamped(input.weight);
                self.add_link(link)?;
                report.created += 1;
            }
        }

        tx.commit()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        Ok(report)
    }

    fn prune_concepts(&self, memoir_id: &MemoirId, keep_names: &[String]) -> HyphaeResult<usize> {
        if keep_names.is_empty() {
            // Delete all concepts in this memoir
            let deleted = self
                .conn
                .execute(
                    "DELETE FROM concepts WHERE memoir_id = ?1",
                    params![memoir_id.as_ref()],
                )
                .map_err(|e| HyphaeError::Database(e.to_string()))?;
            return Ok(deleted);
        }

        // Build a parameterized NOT IN clause
        let placeholders: String = (1..=keep_names.len())
            .map(|i| format!("?{}", i + 1))
            .collect::<Vec<_>>()
            .join(", ");

        let sql =
            format!("DELETE FROM concepts WHERE memoir_id = ?1 AND name NOT IN ({placeholders})");

        let mut param_values: Vec<&dyn rusqlite::types::ToSql> =
            Vec::with_capacity(keep_names.len() + 1);
        let memoir_id_str = memoir_id.to_string();
        param_values.push(&memoir_id_str);
        for name in keep_names {
            param_values.push(name);
        }

        let deleted = self
            .conn
            .execute(&sql, param_values.as_slice())
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        Ok(deleted)
    }

    fn memoir_stats(&self, memoir_id: &MemoirId) -> HyphaeResult<MemoirStats> {
        let total_concepts: usize = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM concepts WHERE memoir_id = ?1",
                params![memoir_id.as_ref()],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n as usize)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let total_links: usize = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM concept_links
                 WHERE source_id IN (SELECT id FROM concepts WHERE memoir_id = ?1)",
                params![memoir_id.as_ref()],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n as usize)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let avg_confidence: f32 = if total_concepts > 0 {
            self.conn
                .query_row(
                    "SELECT AVG(confidence) FROM concepts WHERE memoir_id = ?1",
                    params![memoir_id.as_ref()],
                    |row| row.get(0),
                )
                .map_err(|e| HyphaeError::Database(e.to_string()))?
        } else {
            0.0
        };

        let mut label_stmt = self
            .conn
            .prepare(
                "SELECT json_extract(j.value, '$.namespace') || ':' || json_extract(j.value, '$.value'),
                        COUNT(*)
                 FROM concepts, json_each(concepts.labels) AS j
                 WHERE memoir_id = ?1
                 GROUP BY 1
                 ORDER BY 2 DESC",
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        let label_counts: Vec<(String, usize)> = label_stmt
            .query_map(params![memoir_id.as_ref()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1).map(|n| n as usize)?,
                ))
            })
            .map_err(|e| HyphaeError::Database(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        Ok(MemoirStats {
            total_concepts,
            total_links,
            avg_confidence,
            label_counts,
        })
    }

    fn list_all_links(&self, memoir_id: &MemoirId) -> HyphaeResult<Vec<ConceptLink>> {
        let mut stmt = self
            .conn
            .prepare_cached(
                "SELECT cl.id, cl.source_id, cl.target_id, cl.relation, cl.weight, cl.link_count, cl.created_at, cl.valid_from, cl.valid_to \
                 FROM concept_links cl \
                 JOIN concepts c ON cl.source_id = c.id \
                 WHERE c.memoir_id = ?1 AND (cl.valid_to IS NULL OR cl.valid_to = '')",
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        let rows = stmt
            .query_map(params![memoir_id.as_ref()], row_to_link)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| HyphaeError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    fn set_concept_community(
        &self,
        concept_id: &ConceptId,
        community_id: Option<&str>,
    ) -> HyphaeResult<()> {
        let changed = self
            .conn
            .execute(
                "UPDATE concepts SET community_id = ?2 WHERE id = ?1",
                params![concept_id.as_ref(), community_id],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        if changed == 0 {
            return Err(HyphaeError::NotFound(concept_id.to_string()));
        }
        Ok(())
    }

    fn store_memoir_version(&self, version: hyphae_core::MemoirVersion) -> HyphaeResult<()> {
        let created_at = version.created_at.to_rfc3339();
        self.conn.execute(
            "INSERT INTO memoir_versions (version_id, memoir_id, version_seq, author, git_hash, diff_summary, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                version.version_id,
                version.memoir_id.as_ref(),
                version.version_seq,
                version.author,
                version.git_hash,
                version.diff_summary,
                created_at,
            ],
        ).map_err(|e| HyphaeError::Database(e.to_string()))?;
        Ok(())
    }

    fn get_memoir_history(
        &self,
        memoir_id: &MemoirId,
        limit: usize,
    ) -> HyphaeResult<Vec<hyphae_core::MemoirVersion>> {
        let mut stmt = self.conn.prepare(
            "SELECT version_id, memoir_id, version_seq, author, git_hash, diff_summary, created_at
             FROM memoir_versions WHERE memoir_id = ?1 ORDER BY version_seq DESC LIMIT ?2"
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![memoir_id.as_ref(), limit as i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, u32>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                ))
            })
            .map_err(|e| HyphaeError::Database(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let versions = rows
            .into_iter()
            .map(
                |(vid, mid, seq, author, git_hash, diff_summary, created_at_str)| {
                    let created_at = created_at_str
                        .parse::<chrono::DateTime<chrono::Utc>>()
                        .unwrap_or_else(|_| {
                            tracing::warn!(
                                version_id = %vid,
                                raw = %created_at_str,
                                "failed to parse memoir_versions.created_at; substituting now()"
                            );
                            chrono::Utc::now()
                        });
                    hyphae_core::MemoirVersion {
                        version_id: vid,
                        memoir_id: hyphae_core::MemoirId::from(mid),
                        version_seq: seq,
                        author,
                        git_hash,
                        diff_summary,
                        created_at,
                    }
                },
            )
            .collect::<Vec<_>>();

        Ok(versions)
    }
}

/// Additional methods on SqliteStore not part of the MemoirStore trait
impl SqliteStore {
    /// List concepts with an optional limit to avoid loading multi-thousand-concept memoirs.
    pub fn list_concepts_capped(
        &self,
        memoir_id: &MemoirId,
        limit: usize,
    ) -> HyphaeResult<Vec<Concept>> {
        let sql = format!(
            "SELECT {CONCEPT_COLS} FROM concepts WHERE memoir_id = ?1 ORDER BY name LIMIT ?2"
        );
        let mut stmt = self
            .conn
            .prepare_cached(&sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        let rows = stmt
            .query_map(params![memoir_id.as_ref(), limit as i64], row_to_concept)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        rows.map(|r| r.map_err(|e| HyphaeError::Database(e.to_string())))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use hyphae_core::{
        Concept, ConceptId, ConceptInput, ConceptLink, HyphaeError, Label, LinkInput, Memoir,
        MemoirStore, MemoryId, MemoryStore, Relation,
    };

    use super::super::SqliteStore;
    use super::normalize_relation;

    fn test_store() -> SqliteStore {
        SqliteStore::in_memory().unwrap()
    }

    fn make_inputs(count: usize) -> Vec<ConceptInput> {
        (0..count)
            .map(|i| ConceptInput {
                name: format!("concept_{i}"),
                labels: vec![],
                description: format!("description for concept_{i}"),
            })
            .collect()
    }

    #[test]
    fn test_upsert_concepts_creates_new() {
        let store = test_store();
        let memoir = Memoir::new("test".into(), "".into());
        let memoir_id = store.create_memoir(memoir).unwrap();

        let inputs = make_inputs(10);
        let report = store.upsert_concepts(&memoir_id, &inputs).unwrap();

        assert_eq!(report.created, 10);
        assert_eq!(report.updated, 0);
        assert_eq!(report.unchanged, 0);

        let concepts = store.list_concepts(&memoir_id).unwrap();
        assert_eq!(concepts.len(), 10);
    }

    #[test]
    fn test_upsert_concepts_update_and_unchanged() {
        let store = test_store();
        let memoir = Memoir::new("test2".into(), "".into());
        let memoir_id = store.create_memoir(memoir).unwrap();

        // Create 10 concepts
        let inputs = make_inputs(10);
        store.upsert_concepts(&memoir_id, &inputs).unwrap();

        // Second upsert: 2 changed descriptions + 1 new + 7 unchanged
        let mut second_batch: Vec<ConceptInput> = make_inputs(10);
        second_batch[0].description = "CHANGED description".into();
        second_batch[3].description = "ALSO CHANGED".into();
        second_batch.push(ConceptInput {
            name: "concept_10".into(),
            labels: vec![],
            description: "brand new".into(),
        });

        let report = store.upsert_concepts(&memoir_id, &second_batch).unwrap();

        assert_eq!(report.created, 1, "one new concept");
        assert_eq!(report.updated, 2, "two changed concepts");
        assert_eq!(report.unchanged, 8, "eight unchanged concepts");

        let concepts = store.list_concepts(&memoir_id).unwrap();
        assert_eq!(concepts.len(), 11);
    }

    #[test]
    fn test_upsert_concepts_label_change_triggers_update() {
        let store = test_store();
        let memoir = Memoir::new("test3".into(), "".into());
        let memoir_id = store.create_memoir(memoir).unwrap();

        let initial = vec![ConceptInput {
            name: "alpha".into(),
            labels: vec![],
            description: "same".into(),
        }];
        store.upsert_concepts(&memoir_id, &initial).unwrap();

        let with_label = vec![ConceptInput {
            name: "alpha".into(),
            labels: vec![Label::new("code", "function").unwrap()],
            description: "same".into(),
        }];
        let report = store.upsert_concepts(&memoir_id, &with_label).unwrap();

        assert_eq!(report.updated, 1);
        assert_eq!(report.created, 0);
        assert_eq!(report.unchanged, 0);
    }

    #[test]
    fn test_upsert_links_creates_and_updates() {
        let store = test_store();
        let memoir = Memoir::new("links_test".into(), "".into());
        let memoir_id = store.create_memoir(memoir).unwrap();

        // Create the concepts the links will reference
        let concept_inputs = vec![
            ConceptInput {
                name: "a".into(),
                labels: vec![],
                description: "node a".into(),
            },
            ConceptInput {
                name: "b".into(),
                labels: vec![],
                description: "node b".into(),
            },
            ConceptInput {
                name: "c".into(),
                labels: vec![],
                description: "node c".into(),
            },
        ];
        store.upsert_concepts(&memoir_id, &concept_inputs).unwrap();

        let links = vec![
            LinkInput {
                source_name: "a".into(),
                target_name: "b".into(),
                relation: "depends_on".into(),
                weight: 0.5,
            },
            LinkInput {
                source_name: "b".into(),
                target_name: "c".into(),
                relation: "part_of".into(),
                weight: 0.8,
            },
        ];
        let report = store.upsert_links(&memoir_id, &links).unwrap();
        assert_eq!(report.created, 2);
        assert_eq!(report.updated, 0);
        assert_eq!(report.unchanged, 0);

        // Re-upsert same links — should be unchanged
        let report2 = store.upsert_links(&memoir_id, &links).unwrap();
        assert_eq!(report2.created, 0);
        assert_eq!(report2.updated, 0);
        assert_eq!(report2.unchanged, 2);

        // Update weight on one link
        let updated_links = vec![LinkInput {
            source_name: "a".into(),
            target_name: "b".into(),
            relation: "depends_on".into(),
            weight: 0.9,
        }];
        let report3 = store.upsert_links(&memoir_id, &updated_links).unwrap();
        assert_eq!(report3.updated, 1);
        assert_eq!(report3.unchanged, 0);
    }

    #[test]
    fn test_prune_concepts_removes_missing_and_cascades_links() {
        let store = test_store();
        let memoir = Memoir::new("prune_test".into(), "".into());
        let memoir_id = store.create_memoir(memoir).unwrap();

        // Create concepts a, b, c
        let concept_inputs = vec![
            ConceptInput {
                name: "a".into(),
                labels: vec![],
                description: "a".into(),
            },
            ConceptInput {
                name: "b".into(),
                labels: vec![],
                description: "b".into(),
            },
            ConceptInput {
                name: "c".into(),
                labels: vec![],
                description: "c".into(),
            },
        ];
        store.upsert_concepts(&memoir_id, &concept_inputs).unwrap();

        // Link a → b
        let links = vec![LinkInput {
            source_name: "a".into(),
            target_name: "b".into(),
            relation: "depends_on".into(),
            weight: 0.5,
        }];
        store.upsert_links(&memoir_id, &links).unwrap();

        // Prune — keep only b and c (remove a)
        let keep = vec!["b".to_string(), "c".to_string()];
        let deleted = store.prune_concepts(&memoir_id, &keep).unwrap();
        assert_eq!(deleted, 1, "one concept deleted");

        let remaining = store.list_concepts(&memoir_id).unwrap();
        assert_eq!(remaining.len(), 2);
        assert!(remaining.iter().all(|c| c.name != "a"));

        // Link a → b should be gone via CASCADE
        let concept_b = store.get_concept_by_name(&memoir_id, "b").unwrap().unwrap();
        let links_to_b = store.get_links_to(&concept_b.id).unwrap();
        assert!(links_to_b.is_empty(), "cascaded link should be deleted");
    }

    #[test]
    fn test_prune_concepts_empty_keep_list_deletes_all() {
        let store = test_store();
        let memoir = Memoir::new("prune_all".into(), "".into());
        let memoir_id = store.create_memoir(memoir).unwrap();

        let inputs = make_inputs(5);
        store.upsert_concepts(&memoir_id, &inputs).unwrap();

        let deleted = store.prune_concepts(&memoir_id, &[]).unwrap();
        assert_eq!(deleted, 5);

        let remaining = store.list_concepts(&memoir_id).unwrap();
        assert!(remaining.is_empty());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Relation Normalization Tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_normalize_relation_part_of() {
        // Canonical form
        assert_eq!(normalize_relation("part_of"), "part_of");
        // Variants without separator
        assert_eq!(normalize_relation("partof"), "part_of");
        // Synonyms
        assert_eq!(normalize_relation("contains"), "part_of");
        assert_eq!(normalize_relation("has"), "part_of");
        assert_eq!(normalize_relation("owns"), "part_of");
        assert_eq!(normalize_relation("includes"), "part_of");
    }

    #[test]
    fn test_normalize_relation_depends_on() {
        // Canonical form
        assert_eq!(normalize_relation("depends_on"), "depends_on");
        // Variants
        assert_eq!(normalize_relation("dependson"), "depends_on");
        assert_eq!(normalize_relation("depends-on"), "depends_on");
        // Synonyms
        assert_eq!(normalize_relation("imports"), "depends_on");
        assert_eq!(normalize_relation("uses"), "depends_on");
        assert_eq!(normalize_relation("requires"), "depends_on");
    }

    #[test]
    fn test_normalize_relation_related_to() {
        // Canonical form
        assert_eq!(normalize_relation("related_to"), "related_to");
        // Variants without separator
        assert_eq!(normalize_relation("relatedto"), "related_to");
        // Synonyms
        assert_eq!(normalize_relation("references"), "related_to");
        assert_eq!(normalize_relation("refers_to"), "related_to");
        assert_eq!(normalize_relation("refers-to"), "related_to");
    }

    #[test]
    fn test_normalize_relation_refines() {
        // Canonical form
        assert_eq!(normalize_relation("refines"), "refines");
        // Synonyms
        assert_eq!(normalize_relation("implements"), "refines");
        assert_eq!(normalize_relation("realizes"), "refines");
        assert_eq!(normalize_relation("satisfies"), "refines");
    }

    #[test]
    fn test_normalize_relation_case_insensitive() {
        // Test uppercase
        assert_eq!(normalize_relation("DEPENDS_ON"), "depends_on");
        assert_eq!(normalize_relation("CONTAINS"), "part_of");
        // Test mixed case
        assert_eq!(normalize_relation("Depends_On"), "depends_on");
        assert_eq!(normalize_relation("Contains"), "part_of");
    }

    #[test]
    fn test_normalize_relation_contradict_and_others() {
        // Test other enum variants
        assert_eq!(normalize_relation("contradicts"), "contradicts");
        assert_eq!(normalize_relation("caused_by"), "caused_by");
        assert_eq!(normalize_relation("instance_of"), "instance_of");
        assert_eq!(normalize_relation("alternative_to"), "alternative_to");
        assert_eq!(normalize_relation("superseded_by"), "superseded_by");
    }

    #[test]
    fn test_normalize_relation_unknown_fallback() {
        // Unknown relations should fall back to lowercase (graceful degradation)
        assert_eq!(normalize_relation("UNKNOWN_RELATION"), "unknown_relation");
        assert_eq!(normalize_relation("custom-relation"), "custom-relation");
    }

    #[test]
    fn test_link_normalization_in_database() {
        let store = test_store();
        let memoir = Memoir::new("norm_test".into(), "Testing relation normalization".into());
        let memoir_id = store.create_memoir(memoir).unwrap();

        // Create two concepts
        let concept_inputs = vec![
            ConceptInput {
                name: "source".into(),
                labels: vec![],
                description: "source concept".into(),
            },
            ConceptInput {
                name: "target".into(),
                labels: vec![],
                description: "target concept".into(),
            },
        ];
        store.upsert_concepts(&memoir_id, &concept_inputs).unwrap();

        // Create links with various synonym forms
        let links = vec![LinkInput {
            source_name: "source".into(),
            target_name: "target".into(),
            relation: "contains".into(), // Synonym for part_of
            weight: 0.5,
        }];
        store.upsert_links(&memoir_id, &links).unwrap();

        // Retrieve the link and verify it was normalized to canonical form
        let source_concept = store
            .get_concept_by_name(&memoir_id, "source")
            .unwrap()
            .unwrap();
        let links_from_source = store.get_links_from(&source_concept.id).unwrap();

        assert_eq!(links_from_source.len(), 1);
        assert_eq!(
            links_from_source[0].relation,
            Relation::PartOf,
            "Synonym 'contains' should be normalized to PartOf"
        );
    }

    #[test]
    fn test_remove_link_removes_only_specified_relation() {
        let store = test_store();
        let memoir = Memoir::new("unlink_test".into(), "Testing remove_link".into());
        let memoir_id = store.create_memoir(memoir).unwrap();

        let concepts = vec![
            ConceptInput {
                name: "a".into(),
                labels: vec![],
                description: "concept a".into(),
            },
            ConceptInput {
                name: "b".into(),
                labels: vec![],
                description: "concept b".into(),
            },
        ];
        store.upsert_concepts(&memoir_id, &concepts).unwrap();

        let a = store.get_concept_by_name(&memoir_id, "a").unwrap().unwrap();
        let b = store.get_concept_by_name(&memoir_id, "b").unwrap().unwrap();

        // Add two distinct links between the same pair
        store
            .add_link(ConceptLink::new(
                a.id.clone(),
                b.id.clone(),
                Relation::DependsOn,
            ))
            .unwrap();
        store
            .add_link(ConceptLink::new(
                a.id.clone(),
                b.id.clone(),
                Relation::RelatedTo,
            ))
            .unwrap();

        let links = store.get_links_from(&a.id).unwrap();
        assert_eq!(links.len(), 2, "should have two links before unlink");

        // Remove only the depends_on link
        store
            .remove_link(&memoir_id, "a", "b", "depends_on")
            .unwrap();

        let remaining = store.get_links_from(&a.id).unwrap();
        assert_eq!(remaining.len(), 1, "should have one link after unlink");
        assert_eq!(
            remaining[0].relation,
            Relation::RelatedTo,
            "related_to should survive"
        );
    }

    #[test]
    fn test_remove_link_not_found_errors() {
        let store = test_store();
        let memoir = Memoir::new("unlink_err".into(), "".into());
        let memoir_id = store.create_memoir(memoir).unwrap();
        let concepts = vec![
            ConceptInput {
                name: "x".into(),
                labels: vec![],
                description: "".into(),
            },
            ConceptInput {
                name: "y".into(),
                labels: vec![],
                description: "".into(),
            },
        ];
        store.upsert_concepts(&memoir_id, &concepts).unwrap();

        // No link exists yet — should return NotFound
        let result = store.remove_link(&memoir_id, "x", "y", "related_to");
        assert!(result.is_err(), "should error when no link exists");

        // Missing concept — should return NotFound
        let result = store.remove_link(&memoir_id, "x", "nonexistent", "related_to");
        assert!(result.is_err(), "should error when concept not found");
    }

    #[test]
    fn test_remove_link_clears_reverse_edge() {
        let store = test_store();
        let memoir = Memoir::new("bidir_remove".into(), "".into());
        let memoir_id = store.create_memoir(memoir).unwrap();
        let concepts = vec![
            ConceptInput {
                name: "A".into(),
                labels: vec![],
                description: "".into(),
            },
            ConceptInput {
                name: "B".into(),
                labels: vec![],
                description: "".into(),
            },
        ];
        store.upsert_concepts(&memoir_id, &concepts).unwrap();

        let a = store.get_concept_by_name(&memoir_id, "A").unwrap().unwrap();
        let b = store.get_concept_by_name(&memoir_id, "B").unwrap().unwrap();

        // Insert both directions
        store
            .add_link(ConceptLink::new(
                a.id.clone(),
                b.id.clone(),
                Relation::DependsOn,
            ))
            .unwrap();
        store
            .add_link(ConceptLink::new(
                b.id.clone(),
                a.id.clone(),
                Relation::DependsOn,
            ))
            .unwrap();

        // remove_link A→B should also remove B→A
        store
            .remove_link(&memoir_id, "A", "B", "depends_on")
            .unwrap();

        let neighbors = store.get_neighbors(&a.id, None).unwrap();
        assert_eq!(
            neighbors.len(),
            0,
            "reverse edge B→A should also be removed so A has no neighbors"
        );
    }

    #[test]
    fn test_invalidate_link_clears_reverse_edge() {
        let store = test_store();
        let memoir = Memoir::new("bidir_invalidate".into(), "".into());
        let memoir_id = store.create_memoir(memoir).unwrap();
        let concepts = vec![
            ConceptInput {
                name: "X".into(),
                labels: vec![],
                description: "".into(),
            },
            ConceptInput {
                name: "Y".into(),
                labels: vec![],
                description: "".into(),
            },
        ];
        store.upsert_concepts(&memoir_id, &concepts).unwrap();

        let x = store.get_concept_by_name(&memoir_id, "X").unwrap().unwrap();
        let y = store.get_concept_by_name(&memoir_id, "Y").unwrap().unwrap();

        // Insert both directions
        let link_id_xy = store
            .add_link(ConceptLink::new(
                x.id.clone(),
                y.id.clone(),
                Relation::RelatedTo,
            ))
            .unwrap();
        store
            .add_link(ConceptLink::new(
                y.id.clone(),
                x.id.clone(),
                Relation::RelatedTo,
            ))
            .unwrap();

        // Invalidate the forward edge; the reverse should be invalidated too
        store.invalidate_link(&link_id_xy).unwrap();

        let neighbors = store.get_neighbors(&x.id, None).unwrap();
        assert_eq!(
            neighbors.len(),
            0,
            "reverse edge Y→X should also be invalidated so X has no neighbors"
        );
    }

    #[test]
    fn test_consolidate_concept_definition_resets_revision() {
        let store = test_store();
        let memoir = Memoir::new("test".to_string(), "".to_string());
        store.create_memoir(memoir.clone()).unwrap();

        let concept = Concept::new(
            memoir.id.clone(),
            "Alpha".to_string(),
            "original".to_string(),
        );
        store.add_concept(concept.clone()).unwrap();

        // Refine a few times to bump revision (starts at 1)
        store.refine_concept(&concept.id, "updated 1", &[]).unwrap();
        store.refine_concept(&concept.id, "updated 2", &[]).unwrap();

        let before = store.get_concept(&concept.id).unwrap().unwrap();
        assert_eq!(before.revision, 3); // 1 initial + 2 refines

        // Consolidate
        store
            .consolidate_concept_definition(&concept.id, "consolidated summary")
            .unwrap();

        let after = store.get_concept(&concept.id).unwrap().unwrap();
        assert_eq!(
            after.revision, 0,
            "revision should reset to 0 after consolidation"
        );
        assert_eq!(after.definition, "consolidated summary");
    }

    #[test]
    fn test_consolidate_concept_definition_not_found() {
        let store = test_store();
        let fake_id = ConceptId::from("fake_id_that_does_not_exist");
        let result = store.consolidate_concept_definition(&fake_id, "whatever");
        assert!(result.is_err());
    }

    #[test]
    fn test_abstract_and_overview_text_roundtrip() {
        let store = test_store();
        let memoir = Memoir::new("test_tiered_content".into(), "".into());
        let memoir_id = store.create_memoir(memoir).unwrap();

        let mut concept = Concept::new(
            memoir_id.clone(),
            "TestConcept".to_string(),
            "A detailed definition of the concept.".to_string(),
        );
        concept.abstract_text = Some("Short abstract summary".to_string());
        concept.overview_text =
            Some("This is a longer overview paragraph providing context.".to_string());

        let concept_id = store.add_concept(concept.clone()).unwrap();

        // Retrieve by ID and verify round-trip
        let retrieved = store.get_concept(&concept_id).unwrap().unwrap();
        assert_eq!(
            retrieved.abstract_text,
            Some("Short abstract summary".to_string())
        );
        assert_eq!(
            retrieved.overview_text,
            Some("This is a longer overview paragraph providing context.".to_string())
        );

        // Retrieve by name and verify round-trip
        let retrieved_by_name = store
            .get_concept_by_name(&memoir_id, "TestConcept")
            .unwrap()
            .unwrap();
        assert_eq!(
            retrieved_by_name.abstract_text,
            Some("Short abstract summary".to_string())
        );
        assert_eq!(
            retrieved_by_name.overview_text,
            Some("This is a longer overview paragraph providing context.".to_string())
        );

        // Update the concept with new tiered content and verify
        let mut updated = retrieved.clone();
        updated.abstract_text = Some("Updated abstract".to_string());
        updated.overview_text = Some("Updated overview with more details.".to_string());

        store.update_concept(&updated).unwrap();

        let final_concept = store.get_concept(&concept_id).unwrap().unwrap();
        assert_eq!(
            final_concept.abstract_text,
            Some("Updated abstract".to_string())
        );
        assert_eq!(
            final_concept.overview_text,
            Some("Updated overview with more details.".to_string())
        );
    }

    #[test]
    fn test_temporal_link_invalidation() {
        let store = test_store();
        let memoir = Memoir::new("test_temporal".into(), "".into());
        let memoir_id = store.create_memoir(memoir).unwrap();

        // Create two concepts
        let concept_a = Concept::new(
            memoir_id.clone(),
            "ConceptA".to_string(),
            "First concept".to_string(),
        );
        let concept_b = Concept::new(
            memoir_id.clone(),
            "ConceptB".to_string(),
            "Second concept".to_string(),
        );

        let id_a = store.add_concept(concept_a).unwrap();
        let id_b = store.add_concept(concept_b).unwrap();

        // Create a link between them
        let link = ConceptLink::new(id_a.clone(), id_b.clone(), Relation::DependsOn);
        let link_id = store.add_link(link).unwrap();

        // Verify link appears in get_links_from (currently valid)
        let links_from = store.get_links_from(&id_a).unwrap();
        assert_eq!(links_from.len(), 1);
        assert_eq!(links_from[0].id, link_id);
        assert!(links_from[0].valid_to.is_none());

        // Verify link appears in get_links_to (currently valid)
        let links_to = store.get_links_to(&id_b).unwrap();
        assert_eq!(links_to.len(), 1);
        assert_eq!(links_to[0].id, link_id);

        // Invalidate the link
        store.invalidate_link(&link_id).unwrap();

        // Verify link does NOT appear in get_links_from (currently valid filter)
        let links_from_after = store.get_links_from(&id_a).unwrap();
        assert_eq!(links_from_after.len(), 0);

        // Verify link does NOT appear in get_links_to (currently valid filter)
        let links_to_after = store.get_links_to(&id_b).unwrap();
        assert_eq!(links_to_after.len(), 0);

        // Verify link has valid_to set
        let neighbors = store.get_neighbors(&id_a, None).unwrap();
        assert_eq!(
            neighbors.len(),
            0,
            "invalidated links should not appear in neighbors"
        );

        // Verify double-invalidation is rejected
        let double_invalidate = store.invalidate_link(&link_id);
        assert!(
            double_invalidate.is_err(),
            "double-invalidation should fail"
        );
        match double_invalidate {
            Err(HyphaeError::NotFound(_)) => {} // Expected
            _ => panic!("expected NotFound error on double-invalidation"),
        }
    }

    #[test]
    fn test_delete_concept_invalidates_memory() {
        let store = test_store();
        let memoir = Memoir::new("test_concept_orphan".into(), "".into());
        let memoir_id = store.create_memoir(memoir).unwrap();

        // Create a concept
        let concept = Concept::new(
            memoir_id.clone(),
            "TestConcept".to_string(),
            "A test concept definition".to_string(),
        );
        let concept_id = store.add_concept(concept).unwrap();

        // Verify concept exists
        let fetched = store.get_concept(&concept_id).unwrap();
        assert!(fetched.is_some(), "concept should exist before deletion");

        // Manually create a memory entry matching the stable ID pattern
        // This simulates what tool_memoir_add_concept does
        let memory_id: MemoryId = format!("memoir-{}-TestConcept", memoir_id).into();
        let memory = hyphae_core::Memory::new(
            format!("memoir/{}", memoir_id),
            "TestConcept: A test concept definition".to_string(),
            hyphae_core::Importance::High,
        );
        let mut memory_with_id = memory;
        memory_with_id.id = memory_id.clone();

        // Store the memory
        store.store(memory_with_id).unwrap();

        // Verify the memory exists and is not invalidated
        let stored_memory = store.get(&memory_id).unwrap();
        assert!(
            stored_memory.is_some(),
            "memory should exist before concept deletion"
        );
        let stored_mem = stored_memory.unwrap();
        assert!(
            stored_mem.invalidated_at.is_none(),
            "memory should not be invalidated yet"
        );

        // Delete the concept
        store.delete_concept(&concept_id).unwrap();

        // Verify concept is deleted
        let deleted = store.get_concept(&concept_id).unwrap();
        assert!(deleted.is_none(), "concept should be deleted");

        // Verify the memory is now invalidated
        let invalidated_memory = store.get(&memory_id).unwrap();
        assert!(
            invalidated_memory.is_some(),
            "memory should still exist (not hard-deleted)"
        );
        let inv_mem = invalidated_memory.unwrap();
        assert!(
            inv_mem.invalidated_at.is_some(),
            "memory should be invalidated after concept deletion"
        );
        assert_eq!(
            inv_mem.invalidation_reason.as_deref(),
            Some("concept_deleted"),
            "invalidation reason should be set"
        );
    }
}
