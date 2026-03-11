use std::collections::HashSet;

use chrono::Utc;
use rusqlite::{OptionalExtension, params};

use hyphae_core::{
    Concept, ConceptId, ConceptLink, HyphaeError, HyphaeResult, Label, LinkId, Memoir, MemoirId,
    MemoirStats, MemoirStore, MemoryId, Relation,
};

use super::SqliteStore;
use super::helpers::{
    CONCEPT_COLS, LINK_COLS, MEMOIR_COLS, row_to_concept, row_to_link, row_to_memoir,
};
use super::search::sanitize_fts_query;

impl MemoirStore for SqliteStore {
    fn create_memoir(&self, memoir: Memoir) -> HyphaeResult<MemoirId> {
        self.conn
            .execute(
                "INSERT INTO memoirs (id, name, description, created_at, updated_at, consolidation_threshold)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    memoir.id.as_ref(),
                    memoir.name,
                    memoir.description,
                    memoir.created_at.to_rfc3339(),
                    memoir.updated_at.to_rfc3339(),
                    memoir.consolidation_threshold,
                ],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
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
                 consolidation_threshold = ?5 WHERE id = ?1",
                params![
                    memoir.id.as_ref(),
                    memoir.name,
                    memoir.description,
                    memoir.updated_at.to_rfc3339(),
                    memoir.consolidation_threshold,
                ],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        if changed == 0 {
            return Err(HyphaeError::NotFound(memoir.id.to_string()));
        }
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

        self.conn
            .execute(
                "INSERT INTO concepts (id, memoir_id, name, definition, labels, confidence,
                 revision, created_at, updated_at, source_memory_ids)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
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

        let changed = self
            .conn
            .execute(
                "UPDATE concepts SET memoir_id = ?2, name = ?3, definition = ?4, labels = ?5,
                 confidence = ?6, revision = ?7, updated_at = ?8, source_memory_ids = ?9
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
                ],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        if changed == 0 {
            return Err(HyphaeError::NotFound(concept.id.to_string()));
        }
        Ok(())
    }

    fn delete_concept(&self, id: &ConceptId) -> HyphaeResult<()> {
        let changed = self
            .conn
            .execute("DELETE FROM concepts WHERE id = ?1", params![id.as_ref()])
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        if changed == 0 {
            return Err(HyphaeError::NotFound(id.to_string()));
        }
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

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| HyphaeError::Database(e.to_string()))?);
        }
        Ok(results)
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

        let sql = format!(
            "SELECT {CONCEPT_COLS} FROM concepts
             WHERE memoir_id = ?1
               AND id IN (SELECT id FROM concepts_fts WHERE concepts_fts MATCH ?2)
             ORDER BY confidence DESC
             LIMIT ?3"
        );

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

        let sql = format!(
            "SELECT {CONCEPT_COLS} FROM concepts
             WHERE id IN (SELECT id FROM concepts_fts WHERE concepts_fts MATCH ?1)
             ORDER BY confidence DESC
             LIMIT ?2"
        );

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
            .prepare(&sql)
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

    fn add_link(&self, link: ConceptLink) -> HyphaeResult<LinkId> {
        self.conn
            .execute(
                "INSERT INTO concept_links (id, source_id, target_id, relation, weight, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    link.id.as_ref(),
                    link.source_id.as_ref(),
                    link.target_id.as_ref(),
                    link.relation.to_string(),
                    link.weight.value(),
                    link.created_at.to_rfc3339(),
                ],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        Ok(link.id)
    }

    fn get_links_from(&self, concept_id: &ConceptId) -> HyphaeResult<Vec<ConceptLink>> {
        let sql = format!("SELECT {LINK_COLS} FROM concept_links WHERE source_id = ?1");
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
        let sql = format!("SELECT {LINK_COLS} FROM concept_links WHERE target_id = ?1");
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

    fn get_neighbors(
        &self,
        concept_id: &ConceptId,
        relation: Option<Relation>,
    ) -> HyphaeResult<Vec<Concept>> {
        let (sql, p_relation);

        let base = format!(
            "SELECT {CONCEPT_COLS} FROM concepts WHERE id IN (
                SELECT target_id FROM concept_links WHERE source_id = ?1 {{filter}}
                UNION
                SELECT source_id FROM concept_links WHERE target_id = ?1 {{filter}}
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
            if frontier.is_empty() {
                break;
            }

            let placeholders: String = (1..=frontier.len())
                .map(|i| format!("?{i}"))
                .collect::<Vec<_>>()
                .join(", ");

            // Batch-fetch outgoing links for all frontier nodes
            let outgoing_sql = format!(
                "SELECT {LINK_COLS} FROM concept_links WHERE source_id IN ({placeholders})"
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
                "SELECT {LINK_COLS} FROM concept_links WHERE target_id IN ({placeholders})"
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
                if visited.insert(link.target_id.to_string()) {
                    next_frontier.push(link.target_id.to_string());
                }
                all_links.push(link);
            }

            for link in incoming {
                if visited.insert(link.source_id.to_string()) {
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

    fn memoir_stats(&self, memoir_id: &MemoirId) -> HyphaeResult<MemoirStats> {
        let total_concepts: usize = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM concepts WHERE memoir_id = ?1",
                params![memoir_id.as_ref()],
                |row| row.get(0),
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let total_links: usize = self
            .conn
            .query_row(
                "SELECT COUNT(*) FROM concept_links
                 WHERE source_id IN (SELECT id FROM concepts WHERE memoir_id = ?1)",
                params![memoir_id.as_ref()],
                |row| row.get(0),
            )
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
                Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?))
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
}
