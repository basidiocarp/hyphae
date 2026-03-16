use std::collections::HashMap;

use chrono::Utc;
use rusqlite::{OptionalExtension, params};

use hyphae_core::{
    HyphaeError, HyphaeResult, Memory, MemoryId, MemoryStore, StoreStats, TopicHealth,
};

use super::SqliteStore;
use super::helpers::{SELECT_COLS, embedding_to_blob, row_to_memory, source_data, source_type};
use super::search::sanitize_fts_query;

impl MemoryStore for SqliteStore {
    fn store(&self, memory: Memory) -> HyphaeResult<MemoryId> {
        let keywords_json = serde_json::to_string(&memory.keywords)?;
        let related_json = serde_json::to_string(&memory.related_ids)?;
        let st = source_type(&memory.source);
        let sd = source_data(&memory.source);
        let emb_blob = memory.embedding.as_deref().map(embedding_to_blob);

        self.conn
            .execute(
                "INSERT INTO memories (id, created_at, updated_at, last_accessed, access_count, weight,
                 topic, summary, raw_excerpt, keywords,
                 importance, source_type, source_data, related_ids, embedding, project, expires_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
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
                    memory.expires_at.map(|dt| dt.to_rfc3339()),
                ],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        if let Some(ref emb) = memory.embedding {
            let blob = embedding_to_blob(emb);
            self.conn
                .execute(
                    "INSERT INTO vec_memories (memory_id, embedding) VALUES (?1, ?2)",
                    params![memory.id.as_ref(), blob],
                )
                .map_err(|e| HyphaeError::Database(e.to_string()))?;
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

    fn update(&self, memory: &Memory) -> HyphaeResult<()> {
        let keywords_json = serde_json::to_string(&memory.keywords)?;
        let related_json = serde_json::to_string(&memory.related_ids)?;
        let st = source_type(&memory.source);
        let sd = source_data(&memory.source);
        let emb_blob = memory.embedding.as_deref().map(embedding_to_blob);

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
                 embedding = ?14, expires_at = ?15
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
                    memory.expires_at.map(|dt| dt.to_rfc3339()),
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

        tx.commit()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        Ok(())
    }

    fn delete(&self, id: &MemoryId) -> HyphaeResult<()> {
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
            "SELECT {SELECT_COLS} FROM memories WHERE ({where_clause}) AND (project = ?{project_pos} OR ?{project_pos} IS NULL) ORDER BY weight DESC LIMIT ?{limit_pos} OFFSET ?{offset_pos}"
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

        let sql = format!(
            "SELECT {SELECT_COLS} FROM memories m
             WHERE m.id IN (
                 SELECT id FROM memories_fts WHERE memories_fts MATCH ?1
             )
             AND (m.project = ?3 OR ?3 IS NULL)
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
            .filter_map(|r| r.ok())
            .collect();

        if knn_rows.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders: Vec<String> = (1..=knn_rows.len()).map(|i| format!("?{i}")).collect();
        let in_clause = placeholders.join(",");
        let project_pos = knn_rows.len() + 1;
        let sql = format!(
            "SELECT {SELECT_COLS} FROM memories WHERE id IN ({in_clause}) AND (project = ?{project_pos} OR ?{project_pos} IS NULL)"
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
            .filter_map(|r| r.ok())
            .collect();

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
        let pool_size = (limit + offset) * 4;
        let sanitized = sanitize_fts_query(query);

        let fts_sql = "SELECT m.id, m.created_at, m.updated_at, m.last_accessed, m.access_count, m.weight, \
                    m.topic, m.summary, m.raw_excerpt, m.keywords, \
                    m.importance, m.source_type, m.source_data, m.related_ids, m.embedding, \
                    m.project, m.expires_at, fts.rank \
             FROM memories_fts fts \
             JOIN memories m ON m.id = fts.id \
             WHERE memories_fts MATCH ?1 \
             AND (m.project = ?3 OR ?3 IS NULL) \
             ORDER BY fts.rank \
             LIMIT ?2";

        let mut fts_scores: HashMap<String, f32> = HashMap::new();
        let mut all_memories: HashMap<String, Memory> = HashMap::new();

        if !sanitized.is_empty() {
            match self.conn.prepare_cached(fts_sql) {
                Ok(mut stmt) => {
                    match stmt.query_map(params![sanitized, pool_size as i64, project], |row| {
                        let memory = row_to_memory(row)?;
                        let rank: f32 = row.get(17)?;
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
                        Err(e) => tracing::warn!("hybrid search FTS query failed: {e}"),
                    }
                }
                Err(e) => tracing::warn!("hybrid search FTS prepare failed: {e}"),
            }
        }

        let vec_results = self.search_by_embedding(embedding, pool_size, 0, project)?;
        let mut vec_scores: HashMap<String, f32> = HashMap::new();
        for (memory, similarity) in vec_results {
            vec_scores.insert(memory.id.to_string(), similarity);
            all_memories.entry(memory.id.to_string()).or_insert(memory);
        }

        let mut scored: Vec<(String, f32)> = Vec::new();
        for id in all_memories.keys() {
            let fts_score = fts_scores.get(id).copied().unwrap_or(0.0);
            let vec_score = vec_scores.get(id).copied().unwrap_or(0.0);
            let combined = 0.3 * fts_score + 0.7 * vec_score;
            scored.push((id.clone(), combined));
        }

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let results: Vec<(Memory, f32)> = scored
            .into_iter()
            .skip(offset)
            .take(limit)
            .filter_map(|(id, score)| all_memories.remove(&id).map(|mem| (mem, score)))
            .collect();

        Ok(results)
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
                "UPDATE memories SET weight = weight * (
                    1.0 - (1.0 - ?1) *
                    CASE importance
                        WHEN 'high' THEN 0.5
                        WHEN 'low' THEN 2.0
                        ELSE 1.0
                    END
                    / (1.0 + access_count * 0.1)
                )
                WHERE importance != 'critical'",
                params![decay_factor],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        Ok(changed)
    }

    fn prune(&self, weight_threshold: f32) -> HyphaeResult<usize> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        tx.execute(
            "DELETE FROM vec_memories WHERE memory_id IN (
                SELECT id FROM memories WHERE weight < ?1 AND importance NOT IN ('critical', 'high')
            )",
            params![weight_threshold],
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let changed = tx
            .execute(
                "DELETE FROM memories WHERE weight < ?1 AND importance NOT IN ('critical', 'high')",
                params![weight_threshold],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        tx.commit()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        Ok(changed)
    }

    fn get_by_topic(&self, topic: &str, project: Option<&str>) -> HyphaeResult<Vec<Memory>> {
        let sql = format!(
            "SELECT {SELECT_COLS} FROM memories WHERE topic = ?1 AND (project = ?2 OR ?2 IS NULL) ORDER BY weight DESC"
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

    fn list_topics(&self, project: Option<&str>) -> HyphaeResult<Vec<(String, usize)>> {
        let mut stmt = self
            .conn
            .prepare_cached("SELECT topic, COUNT(*) FROM memories WHERE (project = ?1 OR ?1 IS NULL) GROUP BY topic ORDER BY topic")
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![project], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?))
            })
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| HyphaeError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    fn consolidate_topic(&self, topic: &str, consolidated: Memory) -> HyphaeResult<()> {
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        tx.execute(
            "DELETE FROM vec_memories WHERE memory_id IN (
                SELECT id FROM memories WHERE topic = ?1
            )",
            params![topic],
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

        tx.execute("DELETE FROM memories WHERE topic = ?1", params![topic])
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        // Inline the INSERT (instead of self.store()) to stay within the transaction
        let keywords_json = serde_json::to_string(&consolidated.keywords)?;
        let related_json = serde_json::to_string(&consolidated.related_ids)?;
        let st = source_type(&consolidated.source);
        let sd = source_data(&consolidated.source);
        let emb_blob = consolidated.embedding.as_deref().map(embedding_to_blob);

        tx.execute(
            "INSERT INTO memories (id, created_at, updated_at, last_accessed, access_count, weight,
             topic, summary, raw_excerpt, keywords,
             importance, source_type, source_data, related_ids, embedding, project, expires_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            params![
                consolidated.id.as_ref(),
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
                consolidated.expires_at.map(|dt| dt.to_rfc3339()),
            ],
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

        if let Some(ref emb) = consolidated.embedding {
            let blob = embedding_to_blob(emb);
            tx.execute(
                "INSERT INTO vec_memories (memory_id, embedding) VALUES (?1, ?2)",
                params![consolidated.id.as_ref(), blob],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        }

        tx.commit()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        Ok(())
    }

    fn count(&self, project: Option<&str>) -> HyphaeResult<usize> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE (project = ?1 OR ?1 IS NULL)",
                params![project],
                |row| row.get::<_, usize>(0),
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))
    }

    fn count_by_topic(&self, topic: &str, project: Option<&str>) -> HyphaeResult<usize> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE topic = ?1 AND (project = ?2 OR ?2 IS NULL)",
                params![topic, project],
                |row| row.get::<_, usize>(0),
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))
    }

    fn topic_health(&self, topic: &str, project: Option<&str>) -> HyphaeResult<TopicHealth> {
        type HealthRow = (
            usize,
            Option<f32>,
            Option<f32>,
            Option<String>,
            Option<String>,
            Option<String>,
            usize,
        );
        let (
            entry_count,
            avg_weight,
            avg_access,
            oldest_str,
            newest_str,
            last_accessed_str,
            stale_count,
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
                 FROM memories WHERE topic = ?1 AND (project = ?2 OR ?2 IS NULL)",
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
            needs_consolidation: entry_count > 5,
            stale_count,
        })
    }

    fn stats(&self, project: Option<&str>) -> HyphaeResult<StoreStats> {
        let (total_memories, total_topics, avg_weight, oldest_str, newest_str): (
            usize,
            usize,
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
                 FROM memories WHERE (project = ?1 OR ?1 IS NULL)",
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
            total_memories,
            total_topics,
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
                SELECT id FROM memories WHERE expires_at IS NOT NULL AND expires_at < ?1
            )",
            params![now],
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let changed = tx
            .execute(
                "DELETE FROM memories WHERE expires_at IS NOT NULL AND expires_at < ?1",
                params![now],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        tx.commit()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        Ok(changed)
    }
}
