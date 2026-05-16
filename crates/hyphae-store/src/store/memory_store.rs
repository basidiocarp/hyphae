use std::collections::HashMap;

use chrono::Utc;
use rusqlite::{OptionalExtension, params};

use hyphae_core::{
    DEFAULT_CONSOLIDATION_THRESHOLD, HyphaeError, HyphaeResult, Memory, MemoryId, MemoryStore,
    SearchOrder, StoreStats, TopicHealth, TopicMemoryOrder,
};

use super::SqliteStore;
use super::helpers::{
    ACTIVE_MEMORY_CLAUSE, SELECT_COLS, embedding_to_blob, row_to_memory, source_data, source_type,
};
use super::search::sanitize_fts_query;

/// Hotness = sigmoid(ln(1 + access_count)) * exp(-age_days / half_life_days).
/// Returns a value in [0.0, 1.0].
fn hotness_score(
    access_count: u32,
    last_accessed: &chrono::DateTime<chrono::Utc>,
    half_life_days: f32,
) -> f32 {
    let age_days = (chrono::Utc::now() - *last_accessed).num_seconds().max(0) as f32 / 86_400.0;
    let frequency = (1.0 + access_count as f32).ln();
    let sig = 1.0 / (1.0 + (-frequency).exp());
    let decay = (-age_days / half_life_days).exp();
    (sig * decay).clamp(0.0, 1.0)
}

#[derive(Clone, Debug)]
struct HotnessConfig {
    alpha: f32,
    half_life_days: f32,
}

/// Weights for the three-signal hybrid retrieval fusion.
/// Configurable via environment variables; defaults sum to 1.0.
struct RetrievalWeights {
    fts: f32,
    cosine: f32,
    entity: f32,
}

impl Default for RetrievalWeights {
    fn default() -> Self {
        Self {
            fts: 0.25,
            cosine: 0.55,
            entity: 0.20,
        }
    }
}

impl RetrievalWeights {
    fn from_env() -> Self {
        let fts = std::env::var("HYPHAE_WEIGHT_FTS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.25_f32)
            .clamp(0.0, 1.0);
        let cosine = std::env::var("HYPHAE_WEIGHT_COSINE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.55_f32)
            .clamp(0.0, 1.0);
        let entity = std::env::var("HYPHAE_WEIGHT_ENTITY")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.20_f32)
            .clamp(0.0, 1.0);
        // Normalize so weights always sum to 1.0, preserving relative contributions.
        // Fall back to defaults if all weights are zero to avoid degenerate scoring.
        let total = fts + cosine + entity;
        if total > 0.0 {
            Self {
                fts: fts / total,
                cosine: cosine / total,
                entity: entity / total,
            }
        } else {
            Self::default()
        }
    }
}

/// Per-importance floor constants for memory decay.
/// Prevents high-importance memories from decaying to near-zero and becoming
/// indistinguishable from deliberately invalidated entries.
/// `critical` has no floor constant — the decay WHERE clause excludes it entirely.
const DECAY_FLOOR_HIGH: f64 = 0.30;
const DECAY_FLOOR_MEDIUM: f64 = 0.10;
const DECAY_FLOOR_LOW: f64 = 0.02;

/// Maximum number of results for unbounded queries to prevent memory/latency explosion.
const MAX_TOPIC_RESULTS: i64 = 500;
const MAX_LIST_TOPICS_RESULTS: i64 = 1000;

impl Default for HotnessConfig {
    fn default() -> Self {
        Self {
            alpha: 0.15,
            half_life_days: 7.0,
        }
    }
}

impl HotnessConfig {
    fn from_env() -> Self {
        let alpha = std::env::var("HYPHAE_HOTNESS_ALPHA")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.15_f32)
            .clamp(0.0, 1.0);
        let half_life_days = std::env::var("HYPHAE_HOTNESS_HALF_LIFE_DAYS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(7.0_f32)
            .max(0.1);
        Self {
            alpha,
            half_life_days,
        }
    }
}

impl SqliteStore {
    pub fn search_fts_scoped(
        &self,
        query: &str,
        limit: usize,
        offset: usize,
        project: Option<&str>,
        worktree: Option<&str>,
    ) -> HyphaeResult<Vec<Memory>> {
        let sanitized = sanitize_fts_query(query);
        if sanitized.is_empty() {
            return Ok(Vec::new());
        }

        let sql = format!(
            "SELECT {SELECT_COLS} FROM memories m
             WHERE m.id IN (
                 SELECT id FROM memories_fts
                 WHERE memories_fts MATCH ?1
                 AND (project = ?2 OR ?2 IS NULL)
             )
             AND (m.worktree = ?3 OR ?3 IS NULL)
             AND m.{ACTIVE_MEMORY_CLAUSE}
             ORDER BY m.weight DESC
             LIMIT ?4 OFFSET ?5"
        );

        let mut stmt = self
            .conn
            .prepare_cached(&sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(
                params![sanitized, project, worktree, limit as i64, offset as i64],
                row_to_memory,
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| HyphaeError::Database(e.to_string()))?);
        }

        let result_ids: Vec<&str> = results.iter().map(|m| m.id.as_ref()).collect();
        if let Err(e) = self.increment_access_counts(result_ids.as_slice()) {
            tracing::warn!(error = %e, "failed to update access counts; recall ranking may degrade");
        }

        Ok(results)
    }

    pub fn search_by_keywords_scoped(
        &self,
        keywords: &[&str],
        limit: usize,
        offset: usize,
        project: Option<&str>,
        worktree: Option<&str>,
    ) -> HyphaeResult<Vec<Memory>> {
        if keywords.is_empty() {
            return Ok(Vec::new());
        }

        // Scoped variant uses 4 extra bind params (limit, offset, project, worktree).
        // Cap keyword count to stay below SQLite's SQLITE_LIMIT_VARIABLE_NUMBER (default 999).
        const MAX_KEYWORD_PARAMS: usize = 989;
        let keywords = &keywords[..keywords.len().min(MAX_KEYWORD_PARAMS)];

        let where_parts: Vec<String> = (0..keywords.len())
            .map(|i| {
                let p = i + 1;
                format!("(keywords LIKE ?{p} OR summary LIKE ?{p} OR topic LIKE ?{p})")
            })
            .collect();
        let where_clause = where_parts.join(" OR ");
        let limit_pos = keywords.len() + 1;
        let offset_pos = keywords.len() + 2;
        let project_pos = keywords.len() + 3;
        let worktree_pos = keywords.len() + 4;

        let query = format!(
            "SELECT {SELECT_COLS}
             FROM memories
             WHERE ({where_clause})
               AND {ACTIVE_MEMORY_CLAUSE}
               AND (project = ?{project_pos} OR ?{project_pos} IS NULL)
               AND (worktree = ?{worktree_pos} OR ?{worktree_pos} IS NULL)
             ORDER BY weight DESC
             LIMIT ?{limit_pos} OFFSET ?{offset_pos}"
        );

        let mut stmt = self
            .conn
            .prepare(&query)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = keywords
            .iter()
            .map(|k| Box::new(format!("%{k}%")) as Box<dyn rusqlite::types::ToSql>)
            .collect();
        param_values.push(Box::new(limit as i64));
        param_values.push(Box::new(offset as i64));
        param_values.push(Box::new(project.map(|s| s.to_string())));
        param_values.push(Box::new(worktree.map(|s| s.to_string())));

        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();

        let rows = stmt
            .query_map(params_ref.as_slice(), row_to_memory)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| HyphaeError::Database(e.to_string()))?);
        }

        let result_ids: Vec<&str> = results.iter().map(|m| m.id.as_ref()).collect();
        if let Err(e) = self.increment_access_counts(result_ids.as_slice()) {
            tracing::warn!(error = %e, "failed to update access counts; recall ranking may degrade");
        }

        Ok(results)
    }

    pub fn get_by_topic_scoped(
        &self,
        topic: &str,
        project: Option<&str>,
        worktree: Option<&str>,
    ) -> HyphaeResult<Vec<Memory>> {
        let sql = format!(
            "SELECT {SELECT_COLS}
             FROM memories
             WHERE topic = ?1
               AND {ACTIVE_MEMORY_CLAUSE}
               AND (project = ?2 OR ?2 IS NULL)
               AND (worktree = ?3 OR ?3 IS NULL)
             ORDER BY weight DESC
             LIMIT ?4"
        );
        let mut stmt = self
            .conn
            .prepare_cached(&sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(
                params![topic, project, worktree, MAX_TOPIC_RESULTS],
                row_to_memory,
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| HyphaeError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    pub fn get_by_agent_id(
        &self,
        agent_id: &str,
        limit: usize,
        offset: usize,
    ) -> HyphaeResult<Vec<Memory>> {
        let sql = format!(
            "SELECT {SELECT_COLS} FROM memories
                          WHERE agent_id = ?1 AND {ACTIVE_MEMORY_CLAUSE}
                          ORDER BY created_at DESC
                          LIMIT ?2 OFFSET ?3"
        );
        let mut stmt = self
            .conn
            .prepare_cached(&sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(
                params![agent_id, limit as i64, offset as i64],
                row_to_memory,
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| HyphaeError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    pub fn search_hybrid_scoped(
        &self,
        query: &str,
        embedding: &[f32],
        limit: usize,
        offset: usize,
        project: Option<&str>,
        worktree: Option<&str>,
    ) -> HyphaeResult<Vec<(Memory, f32)>> {
        let pool_size = limit + offset;
        let sanitized = sanitize_fts_query(query);

        let fts_sql = "SELECT m.id, m.created_at, m.updated_at, m.last_accessed, m.access_count, m.weight, \
                    m.topic, m.summary, m.raw_excerpt, m.keywords, \
                    m.importance, m.source_type, m.source_data, m.related_ids, m.embedding, \
                    m.project, m.branch, m.worktree, m.agent_id, m.expires_at, m.invalidated_at, \
                    m.invalidation_reason, m.superseded_by, m.tier, m.entities, fts.rank \
             FROM memories_fts fts \
             JOIN memories m ON m.id = fts.id \
             WHERE memories_fts MATCH ?1 \
             AND m.invalidated_at IS NULL \
             AND (fts.project = ?3 OR ?3 IS NULL) \
             AND (m.worktree = ?4 OR ?4 IS NULL) \
             ORDER BY fts.rank \
             LIMIT ?2";

        let mut fts_scores: HashMap<String, f32> = HashMap::new();
        let mut all_memories: HashMap<String, Memory> = HashMap::new();

        if !sanitized.is_empty() {
            match self.conn.prepare_cached(fts_sql) {
                Ok(mut stmt) => {
                    match stmt.query_map(
                        params![sanitized, pool_size as i64, project, worktree],
                        |row| {
                            let memory = row_to_memory(row)?;
                            let rank: f32 = row.get(25)?;
                            Ok((memory, rank))
                        },
                    ) {
                        Ok(rows) => {
                            for row in rows.flatten() {
                                let (memory, rank) = row;
                                let score = 1.0 / (1.0 + rank.abs());
                                fts_scores.insert(memory.id.to_string(), score);
                                all_memories.insert(memory.id.to_string(), memory);
                            }
                        }
                        Err(e) => {
                            tracing::debug!(
                                fts_degraded = true,
                                error = %e,
                                "hybrid search FTS dropped"
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!(
                        fts_degraded = true,
                        error = %e,
                        "hybrid search FTS dropped"
                    );
                }
            }
        }

        let vec_results =
            self.search_by_embedding_scoped(embedding, pool_size, 0, project, worktree)?;
        let mut vec_scores: HashMap<String, f32> = HashMap::new();
        for (memory, similarity) in vec_results {
            vec_scores.insert(memory.id.to_string(), similarity);
            all_memories.entry(memory.id.to_string()).or_insert(memory);
        }

        let candidate_ids: Vec<String> = all_memories.keys().cloned().collect();
        let learned_scores = match self.recall_effectiveness_for_memory_ids(&candidate_ids) {
            Ok(scores) => scores,
            Err(e) => {
                tracing::warn!("recall_effectiveness lookup failed: {e}");
                HashMap::new()
            }
        };

        let cfg = HotnessConfig::from_env();
        let weights = RetrievalWeights::from_env();
        let query_entities = hyphae_core::extract_entities(query);

        let mut scored: Vec<(String, f32)> = Vec::new();
        for id in candidate_ids {
            let fts_score = fts_scores.get(&id).copied().unwrap_or(0.0);
            let vec_score = vec_scores.get(&id).copied().unwrap_or(0.0);
            let learned_score = learned_scores.get(&id).copied().unwrap_or(0.0);
            let static_weight_bias = all_memories
                .get(&id)
                .map(|memory| (memory.weight.value() - 0.5) * 0.05)
                .unwrap_or(0.0);
            let hot = all_memories
                .get(&id)
                .map(|m| hotness_score(m.access_count, &m.last_accessed, cfg.half_life_days))
                .unwrap_or(0.0);
            let entity_score = if query_entities.is_empty() {
                0.0
            } else {
                all_memories
                    .get(&id)
                    .map(|m| {
                        let shared = query_entities
                            .iter()
                            .filter(|e| m.entities.contains(*e))
                            .count();
                        shared as f32 / query_entities.len() as f32
                    })
                    .unwrap_or(0.0)
            };
            let similarity = weights.fts * fts_score
                + weights.cosine * vec_score
                + weights.entity * entity_score
                + static_weight_bias
                + 0.12 * learned_score;
            let combined = similarity * (1.0 - cfg.alpha) + hot * cfg.alpha;
            scored.push((id, combined.clamp(0.0, 1.0)));
        }

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let results: Vec<(Memory, f32)> = scored
            .into_iter()
            .skip(offset)
            .take(limit)
            .filter_map(|(id, score)| all_memories.remove(&id).map(|mem| (mem, score)))
            .collect();

        let result_ids: Vec<&str> = results.iter().map(|(m, _)| m.id.as_ref()).collect();
        if let Err(e) = self.increment_access_counts(result_ids.as_slice()) {
            tracing::warn!(error = %e, "failed to update access counts; recall ranking may degrade");
        }

        Ok(results)
    }

    fn search_by_embedding_scoped(
        &self,
        embedding: &[f32],
        limit: usize,
        offset: usize,
        project: Option<&str>,
        worktree: Option<&str>,
    ) -> HyphaeResult<Vec<(Memory, f32)>> {
        // Cap knn_limit to prevent the follow-up IN clause from exceeding SQLite's
        // SQLITE_MAX_VARIABLE_NUMBER (default 999). 900 gives comfortable headroom.
        const KNN_LIMIT_CAP: usize = 900;
        let query_blob = embedding_to_blob(embedding);
        let knn_limit = (limit + offset).min(KNN_LIMIT_CAP);

        let mut knn_stmt = self
            .conn
            .prepare_cached(
                "SELECT memory_id, distance
                 FROM vec_memories
                 WHERE embedding MATCH ?1
                 ORDER BY distance
                 LIMIT ?2",
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let knn_rows: Vec<(String, f32)> = knn_stmt
            .query_map(params![query_blob, knn_limit as i64], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, f32>(1)?))
            })
            .map_err(|e| HyphaeError::Database(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        if knn_rows.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders: Vec<String> = (1..=knn_rows.len()).map(|i| format!("?{i}")).collect();
        let in_clause = placeholders.join(",");
        let project_pos = knn_rows.len() + 1;
        let worktree_pos = knn_rows.len() + 2;
        let sql = format!(
            "SELECT {SELECT_COLS}
             FROM memories
             WHERE id IN ({in_clause})
               AND {ACTIVE_MEMORY_CLAUSE}
               AND (project = ?{project_pos} OR ?{project_pos} IS NULL)
               AND (worktree = ?{worktree_pos} OR ?{worktree_pos} IS NULL)"
        );

        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = knn_rows
            .iter()
            .map(|(id, _)| Box::new(id.clone()) as Box<dyn rusqlite::types::ToSql>)
            .collect();
        param_values.push(Box::new(project.map(|s| s.to_string())));
        param_values.push(Box::new(worktree.map(|s| s.to_string())));
        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();

        let memories: Vec<Memory> = stmt
            .query_map(params_ref.as_slice(), row_to_memory)
            .map_err(|e| HyphaeError::Database(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut memory_map: HashMap<String, Memory> = memories
            .into_iter()
            .map(|m| (m.id.to_string(), m))
            .collect();

        let mut results = Vec::new();
        for (id, distance) in &knn_rows {
            if let Some(memory) = memory_map.remove(id) {
                let similarity = 1.0 - distance;
                results.push((memory, similarity));
            }
        }

        Ok(results.into_iter().skip(offset).take(limit).collect())
    }

    /// Replace an existing memory record unconditionally, preserving `created_at`.
    ///
    /// Unlike [`MemoryStore::update`], this method deletes the existing row and
    /// re-inserts the provided `Memory`, which allows callers to change the
    /// `created_at` timestamp (e.g. for archive import merge).
    pub fn replace_memory(&self, memory: Memory) -> HyphaeResult<MemoryId> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        tx.execute(
            "DELETE FROM vec_memories WHERE memory_id = ?1",
            params![memory.id.as_ref()],
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

        tx.execute(
            "DELETE FROM memories WHERE id = ?1",
            params![memory.id.as_ref()],
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let keywords_json = serde_json::to_string(&memory.keywords)?;
        let related_json = serde_json::to_string(&memory.related_ids)?;
        let st = source_type(&memory.source);
        let sd = source_data(&memory.source);
        let emb_blob = memory.embedding.as_deref().map(embedding_to_blob);
        let entities = if memory.entities.is_empty() {
            let combined = format!("{} {}", memory.topic, memory.summary);
            hyphae_core::extract_entities(&combined)
        } else {
            memory.entities.clone()
        };
        let entities_json = serde_json::to_string(&entities)?;

        tx.execute(
            "INSERT INTO memories (id, created_at, updated_at, last_accessed, access_count, weight,
             topic, summary, raw_excerpt, keywords,
             importance, source_type, source_data, related_ids, embedding, project, branch, worktree,
             expires_at, invalidated_at, invalidation_reason, superseded_by, tier, entities)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)",
            params![
                memory.id.as_ref(),
                memory.created_at.to_rfc3339(),
                memory.updated_at.to_rfc3339(),
                memory.last_accessed.to_rfc3339(),
                memory.access_count,
                memory.weight.value(),
                memory.topic,
                memory.summary,
                memory.raw_excerpt,
                keywords_json,
                memory.importance.to_string(),
                st,
                sd,
                related_json,
                emb_blob,
                memory.project.as_deref(),
                memory.branch.as_deref(),
                memory.worktree.as_deref(),
                memory.expires_at.map(|dt| dt.to_rfc3339()),
                memory.invalidated_at.map(|dt| dt.to_rfc3339()),
                memory.invalidation_reason.as_deref(),
                memory.superseded_by.as_ref().map(MemoryId::as_ref),
                memory.tier.to_string(),
                entities_json,
            ],
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

        if let Some(ref emb) = memory.embedding {
            let blob = embedding_to_blob(emb);
            tx.execute(
                "INSERT INTO vec_memories (memory_id, embedding) VALUES (?1, ?2)",
                params![memory.id.as_ref(), blob],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        }

        let id = memory.id.clone();
        // Audit inside the transaction: only records if the commit succeeds.
        if let Err(e) = self.audit_memory(super::audit::AuditOperation::Update, &memory) {
            tracing::warn!("audit log write failed, replace proceeding: {e}");
        }
        tx.commit()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        Ok(id)
    }

    fn increment_access_counts(&self, ids: &[&str]) -> HyphaeResult<()> {
        if ids.is_empty() {
            return Ok(());
        }

        let now = Utc::now().to_rfc3339();
        let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{i}")).collect();
        let in_clause = placeholders.join(",");
        let sql = format!(
            "UPDATE memories SET access_count = access_count + 1, last_accessed = ?{} WHERE id IN ({})",
            ids.len() + 1,
            in_clause
        );

        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = ids
            .iter()
            .map(|id| Box::new(id.to_string()) as Box<dyn rusqlite::types::ToSql>)
            .collect();
        param_values.push(Box::new(now));
        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();

        self.conn
            .execute(sql.as_str(), params_ref.as_slice())
            .map_err(|e| {
                tracing::warn!("increment_access_counts failed (best-effort): {e}");
                HyphaeError::Database(e.to_string())
            })?;

        Ok(())
    }

    /// Hard-delete memories that were invalidated more than `cutoff_days` ago.
    /// This is a background cleanup operation, not called during consolidation.
    /// Returns the number of memories deleted.
    ///
    /// # Errors
    ///
    /// Returns `HyphaeError::Database` if the transaction, DELETE, or commit fails.
    pub fn purge_old_invalidated(&self, cutoff_days: u32) -> HyphaeResult<usize> {
        let cutoff_seconds = i64::from(cutoff_days) * 86_400;
        let now = Utc::now();
        let cutoff_time = (now - chrono::Duration::seconds(cutoff_seconds)).to_rfc3339();

        // SAFETY: No nested transactions — this method does not call other &self methods
        // that open transactions. The &self receiver is required by the MemoryStore trait.
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        // Delete from vec_memories first (foreign key constraint).
        tx.execute(
            "DELETE FROM vec_memories WHERE memory_id IN (
                SELECT id FROM memories WHERE invalidated_at IS NOT NULL AND invalidated_at < ?1
            )",
            params![cutoff_time],
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

        // Delete from memories.
        let changed = tx
            .execute(
                "DELETE FROM memories WHERE invalidated_at IS NOT NULL AND invalidated_at < ?1",
                params![cutoff_time],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        // Audit inside the transaction: only records if the commit succeeds.
        let meta = serde_json::json!({ "cutoff_days": cutoff_days });
        if let Err(e) = self.write_audit(
            super::audit::AuditOperation::PurgeInvalidated,
            "*",
            None,
            None,
            Some(&meta.to_string()),
        ) {
            tracing::warn!("audit log write failed, mutation proceeding: {e}");
        }

        tx.commit()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        Ok(changed)
    }
}

impl MemoryStore for SqliteStore {
    fn store(&self, memory: Memory) -> HyphaeResult<MemoryId> {
        let _span = tracing::info_span!("hyphae.memory.store").entered();

        // Auto-extract entities from topic + summary if the caller left them empty.
        let entities = if memory.entities.is_empty() {
            let combined = format!("{} {}", memory.topic, memory.summary);
            hyphae_core::extract_entities(&combined)
        } else {
            memory.entities.clone()
        };

        let keywords_json = serde_json::to_string(&memory.keywords)?;
        let related_json = serde_json::to_string(&memory.related_ids)?;
        let entities_json = serde_json::to_string(&entities)?;
        let st = source_type(&memory.source);
        let sd = source_data(&memory.source);
        let emb_blob = memory.embedding.as_deref().map(embedding_to_blob);

        // SAFETY: No nested transactions — this method does not call other &self methods
        // that open transactions. The &self receiver is required by the MemoryStore trait.
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        tx.execute(
                "INSERT INTO memories (id, created_at, updated_at, last_accessed, access_count, weight,
                 topic, summary, raw_excerpt, keywords,
                 importance, source_type, source_data, related_ids, embedding, project, branch, worktree, agent_id,
                 expires_at, invalidated_at, invalidation_reason, superseded_by, tier, entities)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25)",
                params![
                    memory.id.as_ref(),
                    memory.created_at.to_rfc3339(),
                    memory.updated_at.to_rfc3339(),
                    memory.last_accessed.to_rfc3339(),
                    memory.access_count,
                    memory.weight.value(),
                    memory.topic,
                    memory.summary,
                    memory.raw_excerpt,
                    keywords_json,
                    memory.importance.to_string(),
                    st,
                    sd,
                    related_json,
                    emb_blob,
                    memory.project.as_deref(),
                    memory.branch.as_deref(),
                    memory.worktree.as_deref(),
                    memory.agent_id.as_deref(),
                    memory.expires_at.map(|dt| dt.to_rfc3339()),
                    memory.invalidated_at.map(|dt| dt.to_rfc3339()),
                    memory.invalidation_reason.as_deref(),
                    memory.superseded_by.as_ref().map(MemoryId::as_ref),
                    memory.tier.to_string(),
                    entities_json,
                ],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        if let Some(ref emb) = memory.embedding {
            let blob = embedding_to_blob(emb);
            tx.execute(
                "INSERT INTO vec_memories (memory_id, embedding) VALUES (?1, ?2)",
                params![memory.id.as_ref(), blob],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        }

        tx.commit()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        // Audit after commit (outside the transaction). This differs from update()
        // and other methods that audit inside the transaction; the semantic here is
        // "record only what is durably written", and placing the audit after commit
        // makes that explicit. An audit failure does not roll back the committed store.
        if let Err(e) = self.audit_memory(super::audit::AuditOperation::Store, &memory) {
            tracing::warn!("audit log write failed, mutation succeeded: {e}");
        }

        Ok(memory.id)
    }

    fn get(&self, id: &MemoryId) -> HyphaeResult<Option<Memory>> {
        let sql = format!("SELECT {SELECT_COLS} FROM memories WHERE id = ?1");
        let mut stmt = self
            .conn
            .prepare_cached(&sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let result = stmt
            .query_row(params![id.as_ref()], row_to_memory)
            .optional()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        Ok(result)
    }

    fn get_by_ids(&self, ids: &[&str], project: Option<&str>) -> HyphaeResult<Vec<Memory>> {
        if ids.is_empty() {
            return Ok(vec![]);
        }

        let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{i}")).collect();
        let in_clause = placeholders.join(", ");
        let project_pos = ids.len() + 1;

        let sql = if project.is_some() {
            format!(
                "SELECT {SELECT_COLS} FROM memories WHERE id IN ({in_clause}) AND project = ?{project_pos} AND invalidated_at IS NULL ORDER BY updated_at DESC"
            )
        } else {
            format!(
                "SELECT {SELECT_COLS} FROM memories WHERE id IN ({in_clause}) AND invalidated_at IS NULL ORDER BY updated_at DESC"
            )
        };

        let mut stmt = self
            .conn
            .prepare_cached(&sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = ids
            .iter()
            .map(|id| Box::new(id.to_string()) as Box<dyn rusqlite::types::ToSql>)
            .collect();

        if let Some(proj) = project {
            param_values.push(Box::new(proj.to_string()));
        }

        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();

        let memories: Vec<Memory> = stmt
            .query_map(params_ref.as_slice(), row_to_memory)
            .map_err(|e| HyphaeError::Database(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        Ok(memories)
    }

    fn update(&self, memory: &Memory) -> HyphaeResult<()> {
        let _span = tracing::info_span!("hyphae.memory.update").entered();

        let keywords_json = serde_json::to_string(&memory.keywords)?;
        let related_json = serde_json::to_string(&memory.related_ids)?;
        let st = source_type(&memory.source);
        let sd = source_data(&memory.source);
        let emb_blob = memory.embedding.as_deref().map(embedding_to_blob);
        let entities = if memory.entities.is_empty() {
            let combined = format!("{} {}", memory.topic, memory.summary);
            hyphae_core::extract_entities(&combined)
        } else {
            memory.entities.clone()
        };
        let entities_json = serde_json::to_string(&entities)?;

        // SAFETY: No nested transactions — this method does not call other &self methods
        // that open transactions. The &self receiver is required by the MemoryStore trait.
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let changed = tx
            .execute(
                "UPDATE memories SET
                 updated_at = ?2, last_accessed = ?3, access_count = ?4, weight = ?5,
                 topic = ?6, summary = ?7, raw_excerpt = ?8, keywords = ?9,
                 importance = ?10, source_type = ?11, source_data = ?12, related_ids = ?13,
                 embedding = ?14, project = ?15, branch = ?16, worktree = ?17, agent_id = ?18, expires_at = ?19,
                 invalidated_at = ?20, invalidation_reason = ?21, superseded_by = ?22, entities = ?23
                 WHERE id = ?1",
                params![
                    memory.id.as_ref(),
                    memory.updated_at.to_rfc3339(),
                    memory.last_accessed.to_rfc3339(),
                    memory.access_count,
                    memory.weight.value(),
                    memory.topic,
                    memory.summary,
                    memory.raw_excerpt,
                    keywords_json,
                    memory.importance.to_string(),
                    st,
                    sd,
                    related_json,
                    emb_blob,
                    memory.project.as_deref(),
                    memory.branch.as_deref(),
                    memory.worktree.as_deref(),
                    memory.agent_id.as_deref(),
                    memory.expires_at.map(|dt| dt.to_rfc3339()),
                    memory.invalidated_at.map(|dt| dt.to_rfc3339()),
                    memory.invalidation_reason.as_deref(),
                    memory.superseded_by.as_ref().map(MemoryId::as_ref),
                    entities_json,
                ],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        if changed == 0 {
            return Err(HyphaeError::NotFound(memory.id.to_string()));
        }

        tx.execute(
            "DELETE FROM vec_memories WHERE memory_id = ?1",
            params![memory.id.as_ref()],
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

        if let Some(ref emb) = memory.embedding {
            let blob = embedding_to_blob(emb);
            tx.execute(
                "INSERT INTO vec_memories (memory_id, embedding) VALUES (?1, ?2)",
                params![memory.id.as_ref(), blob],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        }

        // Audit inside the transaction: only records if the commit succeeds.
        if let Err(e) = self.audit_memory(super::audit::AuditOperation::Update, memory) {
            tracing::warn!("audit log write failed, mutation proceeding: {e}");
        }

        tx.commit()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        Ok(())
    }

    fn invalidate(
        &self,
        id: &MemoryId,
        reason: Option<&str>,
        superseded_by: Option<&MemoryId>,
    ) -> HyphaeResult<()> {
        let _span = tracing::info_span!("hyphae.memory.invalidate").entered();
        // Write-ahead audit record before mutation
        let meta = serde_json::json!({
            "reason": reason,
            "superseded_by": superseded_by.map(|s| s.as_ref()),
        });
        if let Err(e) = self.write_audit(
            super::audit::AuditOperation::Invalidate,
            id.as_ref(),
            None,
            None,
            Some(&meta.to_string()),
        ) {
            tracing::warn!("audit log write failed, mutation proceeding: {e}");
        }

        let now = Utc::now().to_rfc3339();
        let changed = self
            .conn
            .execute(
                "UPDATE memories
                 SET invalidated_at = ?2,
                     invalidation_reason = ?3,
                     superseded_by = ?4,
                     updated_at = ?2
                 WHERE id = ?1",
                params![
                    id.as_ref(),
                    now,
                    reason,
                    superseded_by.map(MemoryId::as_ref),
                ],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        if changed == 0 {
            return Err(HyphaeError::NotFound(id.to_string()));
        }

        Ok(())
    }

    fn list_invalidated(
        &self,
        limit: usize,
        offset: usize,
        project: Option<&str>,
    ) -> HyphaeResult<Vec<Memory>> {
        let sql = format!(
            "SELECT {SELECT_COLS}
             FROM memories
             WHERE invalidated_at IS NOT NULL
               AND (project = ?1 OR ?1 IS NULL)
             ORDER BY invalidated_at DESC, updated_at DESC
             LIMIT ?2 OFFSET ?3"
        );
        let mut stmt = self
            .conn
            .prepare_cached(&sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![project, limit as i64, offset as i64], row_to_memory)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| HyphaeError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    fn delete(&self, id: &MemoryId) -> HyphaeResult<()> {
        // SAFETY: No nested transactions — this method does not call other &self methods
        // that open transactions. The &self receiver is required by the MemoryStore trait.
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        tx.execute(
            "DELETE FROM vec_memories WHERE memory_id = ?1",
            params![id.as_ref()],
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let changed = tx
            .execute("DELETE FROM memories WHERE id = ?1", params![id.as_ref()])
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        if changed == 0 {
            return Err(HyphaeError::NotFound(id.to_string()));
        }

        // Audit inside the transaction: only records if the commit succeeds.
        if let Err(e) = self.write_audit(
            super::audit::AuditOperation::Delete,
            id.as_ref(),
            None,
            None,
            None,
        ) {
            tracing::warn!("audit log write failed, mutation proceeding: {e}");
        }

        tx.commit()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        Ok(())
    }

    fn search_by_keywords(
        &self,
        keywords: &[&str],
        limit: usize,
        offset: usize,
        project: Option<&str>,
    ) -> HyphaeResult<Vec<Memory>> {
        if keywords.is_empty() {
            return Ok(Vec::new());
        }

        // Non-scoped variant uses 3 extra bind params (limit, offset, project).
        // Cap keyword count to stay below SQLite's SQLITE_LIMIT_VARIABLE_NUMBER (default 999).
        const MAX_KEYWORD_PARAMS: usize = 990;
        let keywords = &keywords[..keywords.len().min(MAX_KEYWORD_PARAMS)];

        let where_parts: Vec<String> = (0..keywords.len())
            .map(|i| {
                let p = i + 1;
                format!("(keywords LIKE ?{p} OR summary LIKE ?{p} OR topic LIKE ?{p})")
            })
            .collect();
        let where_clause = where_parts.join(" OR ");
        let limit_pos = keywords.len() + 1;
        let offset_pos = keywords.len() + 2;
        let project_pos = keywords.len() + 3;

        let query = format!(
            "SELECT {SELECT_COLS}
             FROM memories
             WHERE ({where_clause})
               AND {ACTIVE_MEMORY_CLAUSE}
               AND (project = ?{project_pos} OR ?{project_pos} IS NULL)
             ORDER BY weight DESC
             LIMIT ?{limit_pos} OFFSET ?{offset_pos}"
        );

        let mut stmt = self
            .conn
            .prepare(&query)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = keywords
            .iter()
            .map(|k| Box::new(format!("%{k}%")) as Box<dyn rusqlite::types::ToSql>)
            .collect();
        param_values.push(Box::new(limit as i64));
        param_values.push(Box::new(offset as i64));
        param_values.push(Box::new(project.map(|s| s.to_string())));

        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();

        let rows = stmt
            .query_map(params_ref.as_slice(), row_to_memory)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| HyphaeError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    fn search_fts(
        &self,
        query: &str,
        limit: usize,
        offset: usize,
        project: Option<&str>,
    ) -> HyphaeResult<Vec<Memory>> {
        let sanitized = sanitize_fts_query(query);
        if sanitized.is_empty() {
            return Ok(Vec::new());
        }

        // ─────────────────────────────────────────────────────────────────────
        // FTS5 search with project filter using UNINDEXED column
        // ─────────────────────────────────────────────────────────────────────
        let sql = format!(
            "SELECT {SELECT_COLS} FROM memories m
             WHERE m.id IN (
                 SELECT id FROM memories_fts
                 WHERE memories_fts MATCH ?1
                 AND (project = ?3 OR ?3 IS NULL)
             )
             AND m.{ACTIVE_MEMORY_CLAUSE}
             ORDER BY m.weight DESC
             LIMIT ?2 OFFSET ?4"
        );

        let mut stmt = self
            .conn
            .prepare_cached(&sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(
                params![sanitized, limit as i64, project, offset as i64],
                row_to_memory,
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| HyphaeError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    fn search_fts_in_topic(
        &self,
        query: &str,
        topic: &str,
        limit: usize,
        offset: usize,
        project: Option<&str>,
    ) -> HyphaeResult<Vec<Memory>> {
        let sanitized = sanitize_fts_query(query);
        if sanitized.is_empty() {
            return Ok(Vec::new());
        }

        let sql = format!(
            "SELECT {SELECT_COLS} FROM memories m
             WHERE m.id IN (
                 SELECT id FROM memories_fts
                 WHERE memories_fts MATCH ?1
                 AND topic = ?2
                 AND (project = ?4 OR ?4 IS NULL)
             )
             AND m.topic = ?2
             AND m.{ACTIVE_MEMORY_CLAUSE}
             ORDER BY m.weight DESC
             LIMIT ?3 OFFSET ?5"
        );

        let mut stmt = self
            .conn
            .prepare_cached(&sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(
                params![sanitized, topic, limit as i64, project, offset as i64],
                row_to_memory,
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| HyphaeError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    fn search_by_embedding(
        &self,
        embedding: &[f32],
        limit: usize,
        offset: usize,
        project: Option<&str>,
    ) -> HyphaeResult<Vec<(Memory, f32)>> {
        let query_blob = embedding_to_blob(embedding);
        // Fetch enough from KNN to apply offset on final results
        let knn_limit = limit + offset;

        let mut knn_stmt = self
            .conn
            .prepare_cached(
                "SELECT memory_id, distance
                 FROM vec_memories
                 WHERE embedding MATCH ?1
                 ORDER BY distance
                 LIMIT ?2",
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let knn_rows: Vec<(String, f32)> = knn_stmt
            .query_map(params![query_blob, knn_limit as i64], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, f32>(1)?))
            })
            .map_err(|e| HyphaeError::Database(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        if knn_rows.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders: Vec<String> = (1..=knn_rows.len()).map(|i| format!("?{i}")).collect();
        let in_clause = placeholders.join(",");
        let project_pos = knn_rows.len() + 1;
        let sql = format!(
            "SELECT {SELECT_COLS}
             FROM memories
             WHERE id IN ({in_clause})
               AND {ACTIVE_MEMORY_CLAUSE}
               AND (project = ?{project_pos} OR ?{project_pos} IS NULL)"
        );

        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = knn_rows
            .iter()
            .map(|(id, _)| Box::new(id.clone()) as Box<dyn rusqlite::types::ToSql>)
            .collect();
        param_values.push(Box::new(project.map(|s| s.to_string())));
        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();

        let memories: Vec<Memory> = stmt
            .query_map(params_ref.as_slice(), row_to_memory)
            .map_err(|e| HyphaeError::Database(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut memory_map: HashMap<String, Memory> = memories
            .into_iter()
            .map(|m| (m.id.to_string(), m))
            .collect();

        let mut results = Vec::new();
        for (id, distance) in &knn_rows {
            if let Some(memory) = memory_map.remove(id) {
                let similarity = 1.0 - distance;
                results.push((memory, similarity));
            }
        }
        // Apply offset on final results
        let results = results.into_iter().skip(offset).take(limit).collect();
        Ok(results)
    }

    fn search_hybrid(
        &self,
        query: &str,
        embedding: &[f32],
        limit: usize,
        offset: usize,
        project: Option<&str>,
    ) -> HyphaeResult<Vec<(Memory, f32)>> {
        // Reduced multiplier from 4x to 1.5x for ~50% memory reduction
        // Provides sufficient headroom for RRF ranking and dedup
        let pool_size = ((limit + offset) as f32 * 1.5).ceil() as usize;
        let sanitized = sanitize_fts_query(query);

        // ─────────────────────────────────────────────────────────────────────
        // FTS5 search with project filter using UNINDEXED column
        // ─────────────────────────────────────────────────────────────────────
        let fts_sql = "SELECT m.id, m.created_at, m.updated_at, m.last_accessed, m.access_count, m.weight, \
                    m.topic, m.summary, m.raw_excerpt, m.keywords, \
                    m.importance, m.source_type, m.source_data, m.related_ids, m.embedding, \
                    m.project, m.branch, m.worktree, m.agent_id, m.expires_at, m.invalidated_at, \
                    m.invalidation_reason, m.superseded_by, m.tier, m.entities, fts.rank \
             FROM memories_fts fts \
             JOIN memories m ON m.id = fts.id \
             WHERE memories_fts MATCH ?1 \
             AND m.invalidated_at IS NULL \
             AND (fts.project = ?3 OR ?3 IS NULL) \
             ORDER BY fts.rank \
             LIMIT ?2";

        let mut fts_scores: HashMap<String, f32> = HashMap::new();
        let mut all_memories: HashMap<String, Memory> = HashMap::new();

        if !sanitized.is_empty() {
            match self.conn.prepare_cached(fts_sql) {
                Ok(mut stmt) => {
                    match stmt.query_map(params![sanitized, pool_size as i64, project], |row| {
                        let memory = row_to_memory(row)?;
                        let rank: f32 = row.get(25)?;
                        Ok((memory, rank))
                    }) {
                        Ok(rows) => {
                            for row in rows.flatten() {
                                let (memory, rank) = row;
                                let score = 1.0 / (1.0 + rank.abs());
                                fts_scores.insert(memory.id.to_string(), score);
                                all_memories.insert(memory.id.to_string(), memory);
                            }
                        }
                        Err(e) => {
                            tracing::debug!(
                                fts_degraded = true,
                                error = %e,
                                "hybrid search FTS dropped"
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::debug!(
                        fts_degraded = true,
                        error = %e,
                        "hybrid search FTS dropped"
                    );
                }
            }
        }

        let vec_results = self.search_by_embedding(embedding, pool_size, 0, project)?;
        let mut vec_scores: HashMap<String, f32> = HashMap::new();
        for (memory, similarity) in vec_results {
            vec_scores.insert(memory.id.to_string(), similarity);
            all_memories.entry(memory.id.to_string()).or_insert(memory);
        }

        let candidate_ids: Vec<String> = all_memories.keys().cloned().collect();
        let learned_scores = match self.recall_effectiveness_for_memory_ids(&candidate_ids) {
            Ok(scores) => scores,
            Err(e) => {
                tracing::warn!("recall_effectiveness lookup failed: {e}");
                HashMap::new()
            }
        };

        let cfg = HotnessConfig::from_env();
        let weights = RetrievalWeights::from_env();
        let query_entities = hyphae_core::extract_entities(query);

        let mut scored: Vec<(String, f32)> = Vec::new();
        for id in candidate_ids {
            let fts_score = fts_scores.get(&id).copied().unwrap_or(0.0);
            let vec_score = vec_scores.get(&id).copied().unwrap_or(0.0);
            let learned_score = learned_scores.get(&id).copied().unwrap_or(0.0);
            let static_weight_bias = all_memories
                .get(&id)
                .map(|memory| (memory.weight.value() - 0.5) * 0.05)
                .unwrap_or(0.0);
            let hot = all_memories
                .get(&id)
                .map(|m| hotness_score(m.access_count, &m.last_accessed, cfg.half_life_days))
                .unwrap_or(0.0);
            let entity_score = if query_entities.is_empty() {
                0.0
            } else {
                all_memories
                    .get(&id)
                    .map(|m| {
                        let shared = query_entities
                            .iter()
                            .filter(|e| m.entities.contains(*e))
                            .count();
                        shared as f32 / query_entities.len() as f32
                    })
                    .unwrap_or(0.0)
            };
            let similarity = weights.fts * fts_score
                + weights.cosine * vec_score
                + weights.entity * entity_score
                + static_weight_bias
                + 0.12 * learned_score;
            let combined = similarity * (1.0 - cfg.alpha) + hot * cfg.alpha;
            scored.push((id, combined.clamp(0.0, 1.0)));
        }

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let results: Vec<(Memory, f32)> = scored
            .into_iter()
            .skip(offset)
            .take(limit)
            .filter_map(|(id, score)| all_memories.remove(&id).map(|mem| (mem, score)))
            .collect();

        let result_ids: Vec<&str> = results.iter().map(|(m, _)| m.id.as_ref()).collect();
        if let Err(e) = self.increment_access_counts(result_ids.as_slice()) {
            tracing::warn!(error = %e, "failed to update access counts; recall ranking may degrade");
        }

        Ok(results)
    }

    #[allow(clippy::too_many_arguments)]
    fn search_fts_with_options(
        &self,
        query: &str,
        topic: Option<&str>,
        limit: usize,
        offset: usize,
        project: Option<&str>,
        include_invalidated: bool,
        order: SearchOrder,
    ) -> HyphaeResult<Vec<Memory>> {
        let sanitized = sanitize_fts_query(query);
        if sanitized.is_empty() {
            return Ok(Vec::new());
        }

        let topic_clause = if topic.is_some() {
            "AND m.topic = ?3"
        } else {
            ""
        };
        let active_clause = if include_invalidated {
            String::new()
        } else {
            format!("AND m.{ACTIVE_MEMORY_CLAUSE}")
        };
        let qualified_select_cols = format!("m.{}", SELECT_COLS.replace(", ", ", m."));
        let order_clause = match order {
            SearchOrder::RankAsc => "bm25(memories_fts) ASC, m.weight DESC, m.created_at DESC",
            SearchOrder::WeightDesc => "m.weight DESC, m.created_at DESC",
        };
        let sql = format!(
            "SELECT {qualified_select_cols} FROM memories m
             JOIN memories_fts ON memories_fts.id = m.id
             WHERE memories_fts MATCH ?1
               AND (m.project = ?2 OR ?2 IS NULL)
               {topic_clause}
               {active_clause}
             ORDER BY {order_clause}
             LIMIT ?4 OFFSET ?5"
        );

        let mut stmt = self
            .conn
            .prepare_cached(&sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(
                params![sanitized, project, topic, limit as i64, offset as i64],
                row_to_memory,
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| HyphaeError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    fn search_fts_count_with_options(
        &self,
        query: &str,
        topic: Option<&str>,
        project: Option<&str>,
        include_invalidated: bool,
    ) -> HyphaeResult<usize> {
        let sanitized = sanitize_fts_query(query);
        if sanitized.is_empty() {
            return Ok(0);
        }

        let active_clause = if include_invalidated {
            String::new()
        } else {
            format!("AND m.{ACTIVE_MEMORY_CLAUSE}")
        };
        let sql = format!(
            "SELECT COUNT(*)
             FROM memories m
             JOIN memories_fts ON memories_fts.id = m.id
             WHERE memories_fts MATCH ?1
               AND (m.project = ?2 OR ?2 IS NULL)
               AND (?3 IS NULL OR m.topic = ?3)
               {active_clause}"
        );

        self.conn
            .query_row(&sql, params![sanitized, project, topic], |row| {
                row.get::<_, i64>(0)
            })
            .map(|n| n as usize)
            .map_err(|e| HyphaeError::Database(e.to_string()))
    }

    fn get_by_topic_with_options(
        &self,
        topic: &str,
        project: Option<&str>,
        include_invalidated: bool,
        order: TopicMemoryOrder,
    ) -> HyphaeResult<Vec<Memory>> {
        let active_clause = if include_invalidated {
            String::new()
        } else {
            format!("AND {ACTIVE_MEMORY_CLAUSE}")
        };
        let order_clause = match order {
            TopicMemoryOrder::CreatedAtDesc => "created_at DESC, weight DESC",
            TopicMemoryOrder::WeightDesc => "weight DESC, created_at DESC",
        };
        let sql = format!(
            "SELECT {SELECT_COLS}
             FROM memories
             WHERE topic = ?1
               {active_clause}
               AND (project = ?2 OR ?2 IS NULL)
             ORDER BY {order_clause}"
        );
        let mut stmt = self
            .conn
            .prepare_cached(&sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![topic, project], row_to_memory)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| HyphaeError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    fn list_topics_with_options(
        &self,
        project: Option<&str>,
        include_invalidated: bool,
    ) -> HyphaeResult<Vec<(String, usize)>> {
        let active_clause = if include_invalidated {
            String::new()
        } else {
            format!("WHERE {ACTIVE_MEMORY_CLAUSE}")
        };
        let project_clause = if include_invalidated {
            "WHERE (project = ?1 OR ?1 IS NULL)"
        } else {
            "AND (project = ?1 OR ?1 IS NULL)"
        };
        let sql = format!(
            "SELECT topic, COUNT(*)
             FROM memories
             {active_clause}
             {project_clause}
             GROUP BY topic
             ORDER BY topic"
        );
        let mut stmt = self
            .conn
            .prepare_cached(&sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![project], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1).map(|n| n as usize)?,
                ))
            })
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| HyphaeError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    fn topic_health_with_options(
        &self,
        topic: &str,
        project: Option<&str>,
        include_invalidated: bool,
    ) -> HyphaeResult<TopicHealth> {
        type HealthRow = (
            i64,
            Option<f32>,
            Option<f32>,
            Option<String>,
            Option<String>,
            Option<String>,
            i64,
        );
        let active_clause = if include_invalidated {
            String::new()
        } else {
            " AND invalidated_at IS NULL".to_string()
        };
        let sql = format!(
            "SELECT
                COUNT(*),
                AVG(weight),
                AVG(CAST(access_count AS REAL)),
                MIN(created_at),
                MAX(created_at),
                MAX(last_accessed),
                COUNT(CASE WHEN weight < 0.5
                    AND julianday('now') - julianday(last_accessed) > 14
                    THEN 1 END)
             FROM memories
             WHERE topic = ?1
               {active_clause}
               AND (project = ?2 OR ?2 IS NULL)"
        );
        let (
            entry_count_raw,
            avg_weight,
            avg_access,
            oldest_str,
            newest_str,
            last_accessed_str,
            stale_count_raw,
        ): HealthRow = self
            .conn
            .query_row(&sql, params![topic, project], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                ))
            })
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let entry_count = entry_count_raw as usize;
        let stale_count = stale_count_raw as usize;

        if entry_count == 0 {
            return Err(HyphaeError::NotFound(format!("topic: {topic}")));
        }

        let parse_opt_dt = |s: Option<String>| -> Option<chrono::DateTime<Utc>> {
            s.and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                .map(|d| d.with_timezone(&Utc))
        };

        Ok(TopicHealth {
            topic: topic.to_string(),
            entry_count,
            avg_weight: avg_weight.unwrap_or(0.0),
            avg_access_count: avg_access.unwrap_or(0.0),
            oldest: parse_opt_dt(oldest_str),
            newest: parse_opt_dt(newest_str),
            last_accessed: parse_opt_dt(last_accessed_str),
            needs_consolidation: entry_count >= DEFAULT_CONSOLIDATION_THRESHOLD,
            stale_count,
        })
    }

    fn stats_with_options(
        &self,
        project: Option<&str>,
        include_invalidated: bool,
    ) -> HyphaeResult<StoreStats> {
        let active_clause = if include_invalidated {
            String::new()
        } else {
            "invalidated_at IS NULL AND ".to_string()
        };
        let sql = format!(
            "SELECT
                COUNT(*),
                COUNT(DISTINCT topic),
                COALESCE(AVG(weight), 0.0),
                MIN(created_at),
                MAX(created_at)
             FROM memories
             WHERE {active_clause}(project = ?1 OR ?1 IS NULL)"
        );
        let (total_memories_raw, total_topics_raw, avg_weight, oldest_str, newest_str): (
            i64,
            i64,
            f32,
            Option<String>,
            Option<String>,
        ) = self
            .conn
            .query_row(&sql, params![project], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let parse_opt_dt = |s: Option<String>| -> Option<chrono::DateTime<Utc>> {
            s.and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                .map(|d| d.with_timezone(&Utc))
        };

        Ok(StoreStats {
            total_memories: total_memories_raw as usize,
            total_topics: total_topics_raw as usize,
            avg_weight,
            oldest_memory: parse_opt_dt(oldest_str),
            newest_memory: parse_opt_dt(newest_str),
        })
    }

    fn update_access(&self, id: &MemoryId) -> HyphaeResult<()> {
        let now = Utc::now().to_rfc3339();
        let changed = self
            .conn
            .execute(
                "UPDATE memories SET last_accessed = ?1, access_count = access_count + 1 WHERE id = ?2",
                params![now, id.as_ref()],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        if changed == 0 {
            return Err(HyphaeError::NotFound(id.to_string()));
        }
        Ok(())
    }

    fn apply_decay(&self, decay_factor: f32) -> HyphaeResult<usize> {
        let changed = self
            .conn
            .execute(
                "UPDATE memories SET weight = MAX(
                    CASE importance
                        WHEN 'high'   THEN ?2
                        WHEN 'medium' THEN ?3
                        WHEN 'low'    THEN ?4
                        ELSE 0.0
                    END,
                    weight * (
                        1.0 - (1.0 - ?1) *
                        CASE importance
                            WHEN 'high' THEN 0.5
                            WHEN 'low' THEN 2.0
                            ELSE 1.0
                        END
                        / (1.0 + access_count * 0.1)
                    )
                )
                WHERE importance NOT IN ('critical', 'constitution') AND invalidated_at IS NULL",
                params![
                    decay_factor,
                    DECAY_FLOOR_HIGH,
                    DECAY_FLOOR_MEDIUM,
                    DECAY_FLOOR_LOW
                ],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        // Audit after successful execute: only records what actually ran.
        let meta = serde_json::json!({ "decay_factor": decay_factor });
        if let Err(e) = self.write_audit(
            super::audit::AuditOperation::Decay,
            "*",
            None,
            None,
            Some(&meta.to_string()),
        ) {
            tracing::warn!("audit log write failed, mutation succeeded: {e}");
        }

        Ok(changed)
    }

    fn prune(&self, weight_threshold: f32) -> HyphaeResult<usize> {
        // SAFETY: No nested transactions — this method does not call other &self methods
        // that open transactions. The &self receiver is required by the MemoryStore trait.
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        tx.execute(
            "DELETE FROM vec_memories WHERE memory_id IN (
                SELECT id FROM memories
                WHERE weight < ?1
                  AND importance NOT IN ('critical', 'high', 'constitution')
                  AND invalidated_at IS NULL
            )",
            params![weight_threshold],
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let changed = tx
            .execute(
                "DELETE FROM memories
                 WHERE weight < ?1
                   AND importance NOT IN ('critical', 'high', 'constitution')
                   AND invalidated_at IS NULL",
                params![weight_threshold],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        // Audit inside the transaction: only records if the commit succeeds.
        let meta = serde_json::json!({ "weight_threshold": weight_threshold });
        if let Err(e) = self.write_audit(
            super::audit::AuditOperation::Prune,
            "*",
            None,
            None,
            Some(&meta.to_string()),
        ) {
            tracing::warn!("audit log write failed, mutation proceeding: {e}");
        }

        tx.commit()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        Ok(changed)
    }

    fn get_by_topic(&self, topic: &str, project: Option<&str>) -> HyphaeResult<Vec<Memory>> {
        self.get_by_topic_limited(topic, project, None)
    }

    fn list_topics(&self, project: Option<&str>) -> HyphaeResult<Vec<(String, usize)>> {
        let mut stmt = self
            .conn
            .prepare_cached(
                "SELECT topic, COUNT(*)
                 FROM memories
                 WHERE invalidated_at IS NULL
                   AND (project = ?1 OR ?1 IS NULL)
                 GROUP BY topic
                 ORDER BY topic
                 LIMIT ?2",
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![project, MAX_LIST_TOPICS_RESULTS], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1).map(|n| n as usize)?,
                ))
            })
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| HyphaeError::Database(e.to_string()))?);
        }

        if results.len() as i64 >= MAX_LIST_TOPICS_RESULTS {
            tracing::warn!(
                limit = MAX_LIST_TOPICS_RESULTS,
                "list_topics result truncated"
            );
        }

        Ok(results)
    }

    fn consolidate_topic(&self, topic: &str, consolidated: Memory) -> HyphaeResult<()> {
        // SAFETY: No nested transactions — this method does not call other &self methods
        // that open transactions. The &self receiver is required by the MemoryStore trait.
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let project = consolidated.project.as_deref();
        let now = Utc::now().to_rfc3339();
        let new_id = consolidated.id.as_ref();

        // Inline the INSERT (instead of self.store()) to stay within the transaction.
        // Insert the consolidated memory first, then invalidate the sources.
        let keywords_json = serde_json::to_string(&consolidated.keywords)?;
        let related_json = serde_json::to_string(&consolidated.related_ids)?;
        let st = source_type(&consolidated.source);
        let sd = source_data(&consolidated.source);
        let emb_blob = consolidated.embedding.as_deref().map(embedding_to_blob);
        let entities = if consolidated.entities.is_empty() {
            let combined = format!("{} {}", consolidated.topic, consolidated.summary);
            hyphae_core::extract_entities(&combined)
        } else {
            consolidated.entities.clone()
        };
        let entities_json = serde_json::to_string(&entities)?;

        tx.execute(
            "INSERT INTO memories (id, created_at, updated_at, last_accessed, access_count, weight,
             topic, summary, raw_excerpt, keywords,
             importance, source_type, source_data, related_ids, embedding, project, branch, worktree,
             expires_at, invalidated_at, invalidation_reason, superseded_by, tier, entities)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)",
            params![
                new_id,
                consolidated.created_at.to_rfc3339(),
                consolidated.updated_at.to_rfc3339(),
                consolidated.last_accessed.to_rfc3339(),
                consolidated.access_count,
                consolidated.weight.value(),
                consolidated.topic,
                consolidated.summary,
                consolidated.raw_excerpt,
                keywords_json,
                consolidated.importance.to_string(),
                st,
                sd,
                related_json,
                emb_blob,
                consolidated.project.as_deref(),
                consolidated.branch.as_deref(),
                consolidated.worktree.as_deref(),
                consolidated.expires_at.map(|dt| dt.to_rfc3339()),
                consolidated.invalidated_at.map(|dt| dt.to_rfc3339()),
                consolidated.invalidation_reason.as_deref(),
                consolidated.superseded_by.as_ref().map(MemoryId::as_ref),
                consolidated.tier.to_string(),
                entities_json,
            ],
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

        if let Some(ref emb) = consolidated.embedding {
            let blob = embedding_to_blob(emb);
            tx.execute(
                "INSERT INTO vec_memories (memory_id, embedding) VALUES (?1, ?2)",
                params![new_id, blob],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        }

        // Invalidate source memories: set invalidated_at, invalidation_reason, and superseded_by.
        // Guard with id != ?3 to avoid self-invalidation if the new memory ID matches a source.
        tx.execute(
            "UPDATE memories SET
                invalidated_at = ?1,
                invalidation_reason = 'consolidated',
                superseded_by = ?2,
                updated_at = ?1
             WHERE topic = ?3
               AND (project = ?4 OR (?4 IS NULL AND project IS NULL))
               AND invalidated_at IS NULL
               AND id != ?2",
            params![now, new_id, topic, project],
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

        // Audit inside the transaction: only records if the commit succeeds.
        let meta = serde_json::json!({
            "topic": topic,
            "new_memory_id": new_id,
        });
        if let Err(e) = self.write_audit(
            super::audit::AuditOperation::Consolidate,
            new_id,
            Some(topic),
            None,
            Some(&meta.to_string()),
        ) {
            tracing::warn!("audit log write failed, mutation proceeding: {e}");
        }

        tx.commit()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        Ok(())
    }

    fn count(&self, project: Option<&str>) -> HyphaeResult<usize> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE invalidated_at IS NULL AND (project = ?1 OR ?1 IS NULL)",
                params![project],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n as usize)
            .map_err(|e| HyphaeError::Database(e.to_string()))
    }

    fn count_by_topic(&self, topic: &str, project: Option<&str>) -> HyphaeResult<usize> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE topic = ?1 AND invalidated_at IS NULL AND (project = ?2 OR ?2 IS NULL)",
                params![topic, project],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n as usize)
            .map_err(|e| HyphaeError::Database(e.to_string()))
    }

    fn topic_health(&self, topic: &str, project: Option<&str>) -> HyphaeResult<TopicHealth> {
        type HealthRow = (
            i64,
            Option<f32>,
            Option<f32>,
            Option<String>,
            Option<String>,
            Option<String>,
            i64,
        );
        let (
            entry_count_raw,
            avg_weight,
            avg_access,
            oldest_str,
            newest_str,
            last_accessed_str,
            stale_count_raw,
        ): HealthRow = self
            .conn
            .query_row(
                "SELECT
                    COUNT(*),
                    AVG(weight),
                    AVG(CAST(access_count AS REAL)),
                    MIN(created_at),
                    MAX(created_at),
                    MAX(last_accessed),
                    COUNT(CASE WHEN weight < 0.5
                        AND julianday('now') - julianday(last_accessed) > 14
                        THEN 1 END)
                 FROM memories WHERE topic = ?1 AND invalidated_at IS NULL AND (project = ?2 OR ?2 IS NULL)",
                params![topic, project],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                    ))
                },
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let entry_count = entry_count_raw as usize;
        let stale_count = stale_count_raw as usize;

        if entry_count == 0 {
            return Err(HyphaeError::NotFound(format!("topic: {topic}")));
        }

        let parse_opt_dt = |s: Option<String>| -> Option<chrono::DateTime<Utc>> {
            s.and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                .map(|d| d.with_timezone(&Utc))
        };

        Ok(TopicHealth {
            topic: topic.to_string(),
            entry_count,
            avg_weight: avg_weight.unwrap_or(0.0),
            avg_access_count: avg_access.unwrap_or(0.0),
            oldest: parse_opt_dt(oldest_str),
            newest: parse_opt_dt(newest_str),
            last_accessed: parse_opt_dt(last_accessed_str),
            needs_consolidation: entry_count >= DEFAULT_CONSOLIDATION_THRESHOLD,
            stale_count,
        })
    }

    fn stats(&self, project: Option<&str>) -> HyphaeResult<StoreStats> {
        let (total_memories_raw, total_topics_raw, avg_weight, oldest_str, newest_str): (
            i64,
            i64,
            f32,
            Option<String>,
            Option<String>,
        ) = self
            .conn
            .query_row(
                "SELECT
                    COUNT(*),
                    COUNT(DISTINCT topic),
                    COALESCE(AVG(weight), 0.0),
                    MIN(created_at),
                    MAX(created_at)
                 FROM memories WHERE invalidated_at IS NULL AND (project = ?1 OR ?1 IS NULL)",
                params![project],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let parse_opt_dt = |s: Option<String>| -> Option<chrono::DateTime<Utc>> {
            s.and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                .map(|d| d.with_timezone(&Utc))
        };

        Ok(StoreStats {
            total_memories: total_memories_raw as usize,
            total_topics: total_topics_raw as usize,
            avg_weight,
            oldest_memory: parse_opt_dt(oldest_str),
            newest_memory: parse_opt_dt(newest_str),
        })
    }

    fn prune_expired(&self) -> HyphaeResult<usize> {
        let now = Utc::now().to_rfc3339();

        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        tx.execute(
            "DELETE FROM vec_memories WHERE memory_id IN (
                SELECT id FROM memories
                WHERE invalidated_at IS NULL
                  AND expires_at IS NOT NULL
                  AND expires_at < ?1
            )",
            params![now],
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let changed = tx
            .execute(
                "DELETE FROM memories
                 WHERE invalidated_at IS NULL
                   AND expires_at IS NOT NULL
                   AND expires_at < ?1",
                params![now],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        // Audit inside the transaction: only records if the commit succeeds.
        if let Err(e) = self.write_audit(
            super::audit::AuditOperation::PruneExpired,
            "*",
            None,
            None,
            None,
        ) {
            tracing::warn!("audit log write failed, mutation proceeding: {e}");
        }

        tx.commit()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        Ok(changed)
    }
}

/// Additional methods on SqliteStore not part of the MemoryStore trait
impl SqliteStore {
    /// Get memories by topic with optional limit.
    pub fn get_by_topic_limited(
        &self,
        topic: &str,
        project: Option<&str>,
        limit: Option<i64>,
    ) -> HyphaeResult<Vec<Memory>> {
        let effective_limit = limit.unwrap_or(MAX_TOPIC_RESULTS).min(MAX_TOPIC_RESULTS);
        let sql = format!(
            "SELECT {SELECT_COLS}
             FROM memories
             WHERE topic = ?1
               AND {ACTIVE_MEMORY_CLAUSE}
               AND (project = ?2 OR ?2 IS NULL)
             ORDER BY weight DESC
             LIMIT ?3"
        );
        let mut stmt = self
            .conn
            .prepare_cached(&sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![topic, project, effective_limit], row_to_memory)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| HyphaeError::Database(e.to_string()))?);
        }

        if results.len() as i64 >= effective_limit && effective_limit < MAX_TOPIC_RESULTS {
            tracing::warn!(
                topic = %topic,
                truncated_at = effective_limit,
                "get_by_topic result truncated"
            );
        }

        Ok(results)
    }
}

#[cfg(test)]
mod hotness_tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn hotness_zero_access_count_never_recalled() {
        let score = hotness_score(0, &Utc::now(), 7.0);
        assert!(score > 0.0 && score < 1.0, "score={score}");
    }

    #[test]
    fn hotness_high_count_recent_scores_high() {
        let score = hotness_score(50, &Utc::now(), 7.0);
        assert!(score > 0.9, "score={score}");
    }

    #[test]
    fn hotness_decays_with_age() {
        let recent = hotness_score(10, &Utc::now(), 7.0);
        let old = hotness_score(10, &(Utc::now() - chrono::Duration::days(30)), 7.0);
        assert!(recent > old, "recent={recent} old={old}");
    }

    #[test]
    fn hotness_config_defaults() {
        let cfg = HotnessConfig::default();
        assert_eq!(cfg.alpha, 0.15);
        assert_eq!(cfg.half_life_days, 7.0);
    }
}

#[cfg(test)]
mod hybrid_search_fts_tests {
    use super::*;
    use hyphae_core::{Importance, Memory, MemoryId, MemorySource, Weight};

    fn make_store() -> SqliteStore {
        SqliteStore::in_memory().unwrap()
    }

    fn make_memory(topic: &str, summary: &str) -> Memory {
        Memory {
            id: MemoryId::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            last_accessed: chrono::Utc::now(),
            access_count: 0,
            weight: Weight::new(1.0).unwrap(),
            topic: topic.to_string(),
            summary: summary.to_string(),
            raw_excerpt: None,
            keywords: vec![],
            importance: Importance::Medium,
            source: MemorySource::Manual,
            related_ids: vec![],
            embedding: None,
            project: None,
            branch: None,
            worktree: None,
            agent_id: None,
            expires_at: None,
            invalidated_at: None,
            invalidation_reason: None,
            superseded_by: None,
            tier: Default::default(),
            entities: vec![],
        }
    }

    #[test]
    fn fts_candidates_surface_in_hybrid_search_without_embedding() {
        let store = make_store();
        let mem = make_memory(
            "decisions/hyphae",
            "hyphae session initialization changed to lazy mode",
        );
        let stored_id = mem.id.clone();
        store.store(mem).unwrap();

        // No embedding stored — vector arm returns nothing.
        // FTS arm must surface the memory for the query to return a result.
        // sqlite-vec rejects zero-length embeddings even on empty tables, so
        // use a dummy non-zero embedding that will simply produce no KNN hits.
        let dummy_embedding = vec![0.0f32; 384];
        let results = store
            .search_hybrid(
                "hyphae session initialization",
                &dummy_embedding,
                10,
                0,
                None,
            )
            .unwrap();

        assert!(
            !results.is_empty(),
            "FTS arm should surface the stored memory when no embedding is available"
        );
        assert!(
            results.iter().any(|(m, _)| m.id == stored_id),
            "stored memory must appear in hybrid search results"
        );
    }

    #[test]
    fn fts_candidates_surface_in_hybrid_search_scoped() {
        let store = make_store();
        let mem = make_memory(
            "errors/resolved",
            "canopy task dispatch failed due to missing agent_id field",
        );
        let stored_id = mem.id.clone();
        store.store(mem).unwrap();

        let dummy_embedding = vec![0.0f32; 384];
        let results = store
            .search_hybrid_scoped(
                "canopy dispatch agent_id",
                &dummy_embedding,
                10,
                0,
                None,
                None,
            )
            .unwrap();

        assert!(
            !results.is_empty(),
            "FTS arm should surface the stored memory in scoped hybrid search"
        );
        assert!(
            results.iter().any(|(m, _)| m.id == stored_id),
            "stored memory must appear in scoped hybrid search results"
        );
    }
}

#[cfg(test)]
mod consolidate_topic_tests {
    use super::*;
    use hyphae_core::{Importance, Memory, MemoryId, MemorySource, Weight};

    fn make_store() -> SqliteStore {
        SqliteStore::in_memory().unwrap()
    }

    fn make_memory(topic: &str, summary: &str) -> Memory {
        Memory {
            id: MemoryId::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            last_accessed: chrono::Utc::now(),
            access_count: 0,
            weight: Weight::new(1.0).unwrap(),
            topic: topic.to_string(),
            summary: summary.to_string(),
            raw_excerpt: None,
            keywords: vec![],
            importance: Importance::Medium,
            source: MemorySource::Manual,
            related_ids: vec![],
            embedding: None,
            project: None,
            branch: None,
            worktree: None,
            agent_id: None,
            expires_at: None,
            invalidated_at: None,
            invalidation_reason: None,
            superseded_by: None,
            tier: Default::default(),
            entities: vec![],
        }
    }

    #[test]
    fn consolidate_invalidates_source_memories() {
        let store = make_store();
        let topic = "test/consolidation";

        // Store 3 source memories
        let mut source_ids = Vec::new();
        for i in 0..3 {
            let mem = make_memory(topic, &format!("source memory {}", i));
            let id = mem.id.clone();
            store.store(mem).unwrap();
            source_ids.push(id);
        }

        // Create a consolidated memory
        let consolidated = make_memory(topic, "consolidated summary of sources");
        let consolidated_id = consolidated.id.clone();

        // Perform consolidation
        store.consolidate_topic(topic, consolidated).unwrap();

        // Verify: consolidated memory exists and is active
        let retrieved = store.get(&consolidated_id).unwrap();
        assert!(retrieved.is_some(), "consolidated memory should exist");
        let consolidated_mem = retrieved.unwrap();
        assert!(
            consolidated_mem.invalidated_at.is_none(),
            "consolidated memory should not be invalidated"
        );

        // Verify: source memories are invalidated with correct metadata
        for source_id in source_ids {
            let retrieved = store.get(&source_id).unwrap();
            assert!(retrieved.is_some(), "source memory should still exist");
            let source_mem = retrieved.unwrap();
            assert!(
                source_mem.invalidated_at.is_some(),
                "source memory should be invalidated"
            );
            assert_eq!(
                source_mem.invalidation_reason,
                Some("consolidated".to_string()),
                "invalidation reason should be 'consolidated'"
            );
            assert_eq!(
                source_mem.superseded_by,
                Some(consolidated_id.clone()),
                "superseded_by should point to consolidated memory"
            );
        }
    }
}

#[cfg(test)]
mod decay_floor_tests {
    use super::*;
    use hyphae_core::{Importance, Memory, MemoryId, MemorySource, Weight};

    fn make_store() -> SqliteStore {
        SqliteStore::in_memory().unwrap()
    }

    fn make_memory(topic: &str, summary: &str, importance: Importance) -> Memory {
        Memory {
            id: MemoryId::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            last_accessed: chrono::Utc::now(),
            access_count: 0,
            weight: Weight::new(1.0).unwrap(),
            topic: topic.to_string(),
            summary: summary.to_string(),
            raw_excerpt: None,
            keywords: vec![],
            importance,
            source: MemorySource::Manual,
            related_ids: vec![],
            embedding: None,
            project: None,
            branch: None,
            worktree: None,
            agent_id: None,
            expires_at: None,
            invalidated_at: None,
            invalidation_reason: None,
            superseded_by: None,
            tier: Default::default(),
            entities: vec![],
        }
    }

    #[test]
    fn test_decay_floor_low_importance() {
        let store = make_store();
        let mem = make_memory("test/decay", "low importance memory", Importance::Low);
        store.store(mem).unwrap();

        // Apply decay 1000 times with factor 0.95
        for _ in 0..1000 {
            store.apply_decay(0.95).unwrap();
        }

        // Retrieve and check that weight is >= DECAY_FLOOR_LOW (0.02)
        let retrieved = store.get_by_topic("test/decay", None).unwrap();
        assert!(
            !retrieved.is_empty(),
            "memory should still exist after decay"
        );
        let weight = retrieved[0].weight.value();
        assert!(
            weight >= 0.02,
            "Low-importance memory weight after 1000 decay cycles should be >= 0.02, got {weight}"
        );
    }

    #[test]
    fn test_decay_floor_high_importance() {
        let store = make_store();
        let mem = make_memory("test/decay", "high importance memory", Importance::High);
        store.store(mem).unwrap();

        // Apply decay 100 times with factor 0.95
        for _ in 0..100 {
            store.apply_decay(0.95).unwrap();
        }

        // Retrieve and check that weight is >= DECAY_FLOOR_HIGH (0.30)
        let retrieved = store.get_by_topic("test/decay", None).unwrap();
        assert!(
            !retrieved.is_empty(),
            "memory should still exist after decay"
        );
        let weight = retrieved[0].weight.value();
        assert!(
            weight >= 0.30,
            "High-importance memory weight after 100 decay cycles should be >= 0.30, got {weight}"
        );
    }

    #[test]
    fn test_decay_floor_medium_importance() {
        let store = make_store();
        let mem = make_memory("test/decay", "medium importance memory", Importance::Medium);
        store.store(mem).unwrap();

        // Apply decay 200 times with factor 0.95
        for _ in 0..200 {
            store.apply_decay(0.95).unwrap();
        }

        // Retrieve and check that weight is >= DECAY_FLOOR_MEDIUM (0.10)
        let retrieved = store.get_by_topic("test/decay", None).unwrap();
        assert!(
            !retrieved.is_empty(),
            "memory should still exist after decay"
        );
        let weight = retrieved[0].weight.value();
        assert!(
            weight >= 0.10,
            "Medium-importance memory weight after 200 decay cycles should be >= 0.10, got {weight}"
        );
    }

    #[test]
    fn test_decay_floor_sql_literals_match_rust_constants() {
        // This test ensures decay floor SQL string literals match the Rust constants.
        // If this test fails, it means the SQL WHERE clause drift from the Rust enum values.
        let store = make_store();

        // Insert memories with each importance level
        let critical = make_memory("test/drift/critical", "critical", Importance::Critical);
        let high = make_memory("test/drift/high", "high", Importance::High);
        let medium = make_memory("test/drift/medium", "medium", Importance::Medium);
        let low = make_memory("test/drift/low", "low", Importance::Low);

        store.store(critical).unwrap();
        store.store(high).unwrap();
        store.store(medium).unwrap();
        store.store(low).unwrap();

        // Apply decay many times to force decay floors to take effect
        for _ in 0..300 {
            store.apply_decay(0.95).unwrap();
        }

        // Check high importance floor
        let high_mem = store.get_by_topic("test/drift/high", None).unwrap();
        assert!(
            !high_mem.is_empty(),
            "high memory should not be decayed away"
        );
        assert!(
            high_mem[0].weight.value() >= DECAY_FLOOR_HIGH as f32,
            "high importance should be >= DECAY_FLOOR_HIGH ({})",
            DECAY_FLOOR_HIGH
        );

        // Check medium importance floor
        let medium_mem = store.get_by_topic("test/drift/medium", None).unwrap();
        assert!(
            !medium_mem.is_empty(),
            "medium memory should not be decayed away"
        );
        assert!(
            medium_mem[0].weight.value() >= DECAY_FLOOR_MEDIUM as f32,
            "medium importance should be >= DECAY_FLOOR_MEDIUM ({})",
            DECAY_FLOOR_MEDIUM
        );

        // Check low importance floor
        let low_mem = store.get_by_topic("test/drift/low", None).unwrap();
        assert!(!low_mem.is_empty(), "low memory should not be decayed away");
        assert!(
            low_mem[0].weight.value() >= DECAY_FLOOR_LOW as f32,
            "low importance should be >= DECAY_FLOOR_LOW ({})",
            DECAY_FLOOR_LOW
        );

        // Critical memories should never be decayed (they exclude critical in the WHERE clause)
        let critical_mem = store.get_by_topic("test/drift/critical", None).unwrap();
        assert!(
            !critical_mem.is_empty(),
            "critical memory should never be decayed"
        );
    }
}
