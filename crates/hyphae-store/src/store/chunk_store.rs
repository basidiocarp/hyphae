use std::collections::HashMap;

use rusqlite::{OptionalExtension, params};

use hyphae_core::{
    Chunk, ChunkSearchResult, ChunkStore, Document, DocumentId, HyphaeError, HyphaeResult,
};

use super::SqliteStore;
use super::helpers::{CHUNK_COLS, DOCUMENT_COLS, embedding_to_blob, row_to_chunk, row_to_document};
use super::search::sanitize_fts_query;

// Prefixed chunk columns for JOIN queries
const C_CHUNK_COLS: &str = "c.id, c.document_id, c.chunk_index, c.content, c.source_path, \
     c.source_type, c.language, c.heading, c.line_start, c.line_end, c.created_at, c.chunk_strategy";

impl ChunkStore for SqliteStore {
    fn store_document(&self, doc: Document) -> HyphaeResult<DocumentId> {
        let id = doc.id.clone();
        self.conn
            .prepare_cached(&format!(
                "INSERT OR REPLACE INTO documents ({DOCUMENT_COLS}) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"
            ))
            .map_err(|e| HyphaeError::Database(e.to_string()))?
            .execute(params![
                doc.id.to_string(),
                doc.source_path,
                doc.source_type.to_string(),
                doc.chunk_count as u32,
                doc.created_at.to_rfc3339(),
                doc.updated_at.to_rfc3339(),
                doc.project.as_deref(),
                doc.runtime_session_id.as_deref(),
                doc.content_hash.as_deref(),
            ])
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        Ok(id)
    }

    fn store_chunks(&self, chunks: Vec<Chunk>) -> HyphaeResult<usize> {
        if chunks.is_empty() {
            return Ok(0);
        }

        const CHUNK_TX_BATCH: usize = 64;

        // SAFETY: No nested transactions — this method does not call other &self methods
        // that open transactions. The &self receiver is required by the ChunkStore trait.
        let mut tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut successfully_stored = 0;

        for (chunk_idx, chunk) in chunks.into_iter().enumerate() {
            // Begin a new transaction batch if needed
            if chunk_idx > 0 && chunk_idx % CHUNK_TX_BATCH == 0 {
                tx.commit()
                    .map_err(|e| HyphaeError::Database(e.to_string()))?;
                tx = self
                    .conn
                    .unchecked_transaction()
                    .map_err(|e| HyphaeError::Database(e.to_string()))?;
            }

            // Create a SAVEPOINT for this chunk
            let savepoint_name = format!("chunk_{}", chunk_idx);
            tx.execute(&format!("SAVEPOINT {}", savepoint_name), [])
                .map_err(|e| HyphaeError::Database(e.to_string()))?;

            let now = chunk.created_at.to_rfc3339();
            let mut chunk_succeeded = true;

            // INSERT into chunks table
            if let Err(e) = tx
                .prepare_cached(&format!(
                    "INSERT OR REPLACE INTO chunks ({CHUNK_COLS}) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"
                ))
                .and_then(|mut stmt| {
                    stmt.execute(params![
                        chunk.id.to_string(),
                        chunk.document_id.to_string(),
                        chunk.chunk_index,
                        chunk.content.clone(),
                        chunk.metadata.source_path.clone(),
                        chunk.metadata.source_type.to_string(),
                        chunk.metadata.language.clone(),
                        chunk.metadata.heading.clone(),
                        chunk.metadata.line_start,
                        chunk.metadata.line_end,
                        now.clone(),
                        chunk.metadata.chunk_strategy.clone(),
                    ])
                    .map(|_| ())
                })
            {
                tracing::warn!("Failed to insert chunk {}: {}", chunk.id, e);
                chunk_succeeded = false;
            }

            // INSERT into chunks_fts table
            if chunk_succeeded {
                if let Err(e) = tx
                    .prepare_cached(
                        "INSERT OR REPLACE INTO chunks_fts (id, content, source_path, heading) \
                         VALUES (?1, ?2, ?3, ?4)",
                    )
                    .and_then(|mut stmt| {
                        stmt.execute(params![
                            chunk.id.to_string(),
                            chunk.content.clone(),
                            chunk.metadata.source_path.clone(),
                            chunk.metadata.heading.clone(),
                        ])
                        .map(|_| ())
                    })
                {
                    tracing::warn!(
                        "Failed to insert into chunks_fts for chunk {}: {}",
                        chunk.id,
                        e
                    );
                    chunk_succeeded = false;
                }
            }

            // INSERT into vec_chunks if embedding is present
            if chunk_succeeded {
                if let Some(embedding) = &chunk.embedding {
                    let blob = embedding_to_blob(embedding);
                    if let Err(e) = tx
                        .prepare_cached(
                            "INSERT OR REPLACE INTO vec_chunks (chunk_id, embedding) VALUES (?1, ?2)",
                        )
                        .and_then(|mut stmt| {
                            stmt.execute(params![chunk.id.to_string(), blob])
                                .map(|_| ())
                        })
                    {
                        tracing::warn!("Failed to insert into vec_chunks for chunk {}: {}", chunk.id, e);
                        chunk_succeeded = false;
                    }
                }
            }

            // ROLLBACK or RELEASE the savepoint
            if chunk_succeeded {
                tx.execute(&format!("RELEASE {}", savepoint_name), [])
                    .map_err(|e| HyphaeError::Database(e.to_string()))?;
                successfully_stored += 1;
            } else {
                // ROLLBACK TO undoes the chunk's writes but leaves the savepoint
                // on the stack; RELEASE pops it so the per-transaction savepoint
                // stack stays bounded across a 64-chunk batch.
                tx.execute(&format!("ROLLBACK TO {}", savepoint_name), [])
                    .map_err(|e| HyphaeError::Database(e.to_string()))?;
                tx.execute(&format!("RELEASE {}", savepoint_name), [])
                    .map_err(|e| HyphaeError::Database(e.to_string()))?;
            }
        }

        tx.commit()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        Ok(successfully_stored)
    }

    fn get_document(&self, id: &DocumentId) -> HyphaeResult<Option<Document>> {
        self.conn
            .prepare_cached(&format!(
                "SELECT {DOCUMENT_COLS} FROM documents WHERE id = ?1"
            ))
            .map_err(|e| HyphaeError::Database(e.to_string()))?
            .query_row(params![id.to_string()], row_to_document)
            .optional()
            .map_err(|e| HyphaeError::Database(e.to_string()))
    }

    fn get_document_by_path(
        &self,
        path: &str,
        project: Option<&str>,
    ) -> HyphaeResult<Option<Document>> {
        self.conn
            .prepare_cached(&format!(
                "SELECT {DOCUMENT_COLS} FROM documents WHERE source_path = ?1 AND project IS ?2"
            ))
            .map_err(|e| HyphaeError::Database(e.to_string()))?
            .query_row(params![path, project], row_to_document)
            .optional()
            .map_err(|e| HyphaeError::Database(e.to_string()))
    }

    fn get_chunks(&self, document_id: &DocumentId) -> HyphaeResult<Vec<Chunk>> {
        let mut stmt = self
            .conn
            .prepare_cached(&format!(
                "SELECT {CHUNK_COLS} FROM chunks WHERE document_id = ?1 ORDER BY chunk_index"
            ))
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![document_id.to_string()], row_to_chunk)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| HyphaeError::Database(e.to_string()))
    }

    fn delete_document(&self, id: &DocumentId) -> HyphaeResult<()> {
        // SAFETY: No nested transactions — this method does not call other &self methods
        // that open transactions. The &self receiver is required by the ChunkStore trait.
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let id_str = id.to_string();

        tx.execute(
            "DELETE FROM vec_chunks WHERE chunk_id IN \
             (SELECT id FROM chunks WHERE document_id = ?1)",
            params![id_str],
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

        tx.execute(
            "DELETE FROM chunks_fts WHERE id IN \
             (SELECT id FROM chunks WHERE document_id = ?1)",
            params![id_str],
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

        tx.execute("DELETE FROM documents WHERE id = ?1", params![id_str])
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        tx.commit()
            .map_err(|e| HyphaeError::Database(e.to_string()))
    }

    fn list_documents(&self, project: Option<&str>) -> HyphaeResult<Vec<Document>> {
        let mut stmt = self
            .conn
            .prepare_cached(&format!(
                "SELECT {DOCUMENT_COLS} FROM documents WHERE (project = ?1 OR ?1 IS NULL) ORDER BY created_at DESC"
            ))
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![project], row_to_document)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| HyphaeError::Database(e.to_string()))
    }

    fn search_chunks_fts(
        &self,
        query: &str,
        limit: usize,
        offset: usize,
        project: Option<&str>,
    ) -> HyphaeResult<Vec<ChunkSearchResult>> {
        let sanitized = sanitize_fts_query(query);
        if sanitized.is_empty() {
            return Ok(Vec::new());
        }

        let sql = format!(
            "SELECT {C_CHUNK_COLS}, fts.rank \
             FROM chunks_fts fts \
             JOIN chunks c ON c.id = fts.id \
             JOIN documents d ON d.id = c.document_id \
             WHERE chunks_fts MATCH ?1 \
             AND (d.project = ?3 OR ?3 IS NULL) \
             ORDER BY fts.rank \
             LIMIT ?2 OFFSET ?4"
        );

        let mut stmt = self
            .conn
            .prepare_cached(&sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(
                params![sanitized, limit as i64, project, offset as i64],
                |row| {
                    let chunk = row_to_chunk(row)?;
                    let rank: f32 = row.get(12)?;
                    Ok((chunk, rank))
                },
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows.flatten() {
            let (chunk, rank) = row;
            let score = 1.0 / (1.0 + rank.abs());
            results.push(ChunkSearchResult { chunk, score });
        }
        Ok(results)
    }

    fn search_chunks_by_embedding(
        &self,
        embedding: &[f32],
        limit: usize,
        offset: usize,
        project: Option<&str>,
    ) -> HyphaeResult<Vec<ChunkSearchResult>> {
        let query_blob = embedding_to_blob(embedding);
        // Fetch enough from KNN to apply offset on final results
        let knn_limit = limit + offset;

        let knn_rows: Vec<(String, f32)> = self
            .conn
            .prepare_cached(
                "SELECT chunk_id, distance FROM vec_chunks \
                 WHERE embedding MATCH ?1 \
                 ORDER BY distance \
                 LIMIT ?2",
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?
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
        let in_clause = placeholders.join(", ");
        let project_pos = knn_rows.len() + 1;
        let sql = format!(
            "SELECT {CHUNK_COLS} FROM chunks c \
             JOIN documents d ON d.id = c.document_id \
             WHERE c.id IN ({in_clause}) AND (d.project = ?{project_pos} OR ?{project_pos} IS NULL)"
        );

        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut id_params: Vec<Box<dyn rusqlite::ToSql>> = knn_rows
            .iter()
            .map(|(id, _)| Box::new(id.clone()) as Box<dyn rusqlite::ToSql>)
            .collect();
        id_params.push(Box::new(project.map(|s| s.to_string())));
        let params_ref: Vec<&dyn rusqlite::ToSql> = id_params.iter().map(|p| p.as_ref()).collect();

        let chunk_map: HashMap<String, Chunk> = stmt
            .query_map(params_ref.as_slice(), row_to_chunk)
            .map_err(|e| HyphaeError::Database(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| HyphaeError::Database(e.to_string()))?
            .into_iter()
            .map(|c| (c.id.to_string(), c))
            .collect();

        let results = knn_rows
            .into_iter()
            .filter_map(|(id, distance)| {
                chunk_map.get(&id).cloned().map(|chunk| ChunkSearchResult {
                    chunk,
                    score: 1.0 - distance,
                })
            })
            .skip(offset)
            .take(limit)
            .collect();

        Ok(results)
    }

    fn search_chunks_hybrid(
        &self,
        query: &str,
        embedding: &[f32],
        limit: usize,
        offset: usize,
        project: Option<&str>,
    ) -> HyphaeResult<Vec<ChunkSearchResult>> {
        // Reduced multiplier from 4x to 1.5x for ~50% memory reduction
        // Provides sufficient headroom for RRF ranking and dedup
        let pool_size = ((limit + offset) as f32 * 1.5).ceil() as usize;
        let sanitized = sanitize_fts_query(query);

        let mut fts_scores: HashMap<String, f32> = HashMap::new();
        let mut all_chunks: HashMap<String, Chunk> = HashMap::new();

        if !sanitized.is_empty() {
            let fts_sql = format!(
                "SELECT {C_CHUNK_COLS}, fts.rank \
                 FROM chunks_fts fts \
                 JOIN chunks c ON c.id = fts.id \
                 JOIN documents d ON d.id = c.document_id \
                 WHERE chunks_fts MATCH ?1 \
                 AND (d.project = ?3 OR ?3 IS NULL) \
                 ORDER BY fts.rank \
                 LIMIT ?2"
            );

            match self.conn.prepare_cached(&fts_sql) {
                Ok(mut stmt) => {
                    match stmt.query_map(params![sanitized, pool_size as i64, project], |row| {
                        let chunk = row_to_chunk(row)?;
                        let rank: f32 = row.get(12)?;
                        Ok((chunk, rank))
                    }) {
                        Ok(rows) => {
                            for row in rows.flatten() {
                                let (chunk, rank) = row;
                                let score = 1.0 / (1.0 + rank.abs());
                                fts_scores.insert(chunk.id.to_string(), score);
                                all_chunks.insert(chunk.id.to_string(), chunk);
                            }
                        }
                        Err(e) => {
                            tracing::warn!(
                                "chunk FTS search failed, falling back to embedding-only: {e}"
                            );
                        }
                    }
                }
                Err(e) => {
                    tracing::warn!("chunk FTS prepare failed, falling back to embedding-only: {e}");
                }
            }
        }

        let vec_results = self.search_chunks_by_embedding(embedding, pool_size, 0, project)?;
        let mut vec_scores: HashMap<String, f32> = HashMap::new();
        for result in vec_results {
            vec_scores.insert(result.chunk.id.to_string(), result.score);
            all_chunks
                .entry(result.chunk.id.to_string())
                .or_insert(result.chunk);
        }

        let mut scored: Vec<(String, f32)> = all_chunks
            .keys()
            .map(|id| {
                let fts = fts_scores.get(id).copied().unwrap_or(0.0);
                let vec = vec_scores.get(id).copied().unwrap_or(0.0);
                (id.clone(), 0.3 * fts + 0.7 * vec)
            })
            .collect();

        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        let results = scored
            .into_iter()
            .skip(offset)
            .take(limit)
            .filter_map(|(id, score)| {
                all_chunks
                    .remove(&id)
                    .map(|chunk| ChunkSearchResult { chunk, score })
            })
            .collect();

        Ok(results)
    }

    fn count_documents(&self, project: Option<&str>) -> HyphaeResult<usize> {
        self.conn
            .prepare_cached("SELECT COUNT(*) FROM documents WHERE (project = ?1 OR ?1 IS NULL)")
            .map_err(|e| HyphaeError::Database(e.to_string()))?
            .query_row(params![project], |row| row.get::<_, u32>(0))
            .map(|n| n as usize)
            .map_err(|e| HyphaeError::Database(e.to_string()))
    }

    fn count_chunks(&self, project: Option<&str>) -> HyphaeResult<usize> {
        self.conn
            .prepare_cached(
                "SELECT COUNT(*) FROM chunks c \
                 JOIN documents d ON d.id = c.document_id \
                 WHERE (d.project = ?1 OR ?1 IS NULL)",
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?
            .query_row(params![project], |row| row.get::<_, u32>(0))
            .map(|n| n as usize)
            .map_err(|e| HyphaeError::Database(e.to_string()))
    }
}

impl SqliteStore {
    /// Atomically delete an existing document and store a new one with its chunks in a single transaction.
    /// Guarantees all-or-nothing semantics: if any step fails, the entire operation is rolled back.
    pub fn ingest_atomic(
        &self,
        existing_id: &DocumentId,
        doc: Document,
        chunks: Vec<Chunk>,
    ) -> HyphaeResult<DocumentId> {
        self.with_transaction(|| {
            let id_str = existing_id.to_string();

            // Delete old document and all its associated data
            self.conn
                .execute(
                    "DELETE FROM vec_chunks WHERE chunk_id IN \
                 (SELECT id FROM chunks WHERE document_id = ?1)",
                    params![id_str],
                )
                .map_err(|e| HyphaeError::Database(e.to_string()))?;

            self.conn
                .execute(
                    "DELETE FROM chunks_fts WHERE id IN \
                 (SELECT id FROM chunks WHERE document_id = ?1)",
                    params![id_str],
                )
                .map_err(|e| HyphaeError::Database(e.to_string()))?;

            self.conn
                .execute("DELETE FROM documents WHERE id = ?1", params![id_str])
                .map_err(|e| HyphaeError::Database(e.to_string()))?;

            // Store new document
            let new_id = doc.id.clone();
            self.conn
                .prepare_cached(&format!(
                    "INSERT OR REPLACE INTO documents ({DOCUMENT_COLS}) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"
                ))
                .map_err(|e| HyphaeError::Database(e.to_string()))?
                .execute(params![
                    doc.id.to_string(),
                    doc.source_path,
                    doc.source_type.to_string(),
                    doc.chunk_count as u32,
                    doc.created_at.to_rfc3339(),
                    doc.updated_at.to_rfc3339(),
                    doc.project.as_deref(),
                    doc.runtime_session_id.as_deref(),
                    doc.content_hash.as_deref(),
                ])
                .map_err(|e| HyphaeError::Database(e.to_string()))?;

            // Store chunks
            for chunk in chunks {
                let now = chunk.created_at.to_rfc3339();
                self.conn
                    .prepare_cached(&format!(
                        "INSERT OR REPLACE INTO chunks ({CHUNK_COLS}) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)"
                    ))
                    .map_err(|e| HyphaeError::Database(e.to_string()))?
                    .execute(params![
                        chunk.id.to_string(),
                        chunk.document_id.to_string(),
                        chunk.chunk_index,
                        chunk.content,
                        chunk.metadata.source_path,
                        chunk.metadata.source_type.to_string(),
                        chunk.metadata.language,
                        chunk.metadata.heading,
                        chunk.metadata.line_start,
                        chunk.metadata.line_end,
                        now,
                        chunk.metadata.chunk_strategy,
                    ])
                    .map_err(|e| HyphaeError::Database(e.to_string()))?;

                self.conn
                    .prepare_cached(
                        "INSERT OR REPLACE INTO chunks_fts (id, content, source_path, heading) \
                         VALUES (?1, ?2, ?3, ?4)",
                    )
                    .map_err(|e| HyphaeError::Database(e.to_string()))?
                    .execute(params![
                        chunk.id.to_string(),
                        chunk.content,
                        chunk.metadata.source_path,
                        chunk.metadata.heading,
                    ])
                    .map_err(|e| HyphaeError::Database(e.to_string()))?;

                if let Some(embedding) = &chunk.embedding {
                    let blob = embedding_to_blob(embedding);
                    self.conn
                        .prepare_cached(
                            "INSERT OR REPLACE INTO vec_chunks (chunk_id, embedding) VALUES (?1, ?2)",
                        )
                        .map_err(|e| HyphaeError::Database(e.to_string()))?
                        .execute(params![chunk.id.to_string(), blob])
                        .map_err(|e| HyphaeError::Database(e.to_string()))?;
                }
            }

            Ok(new_id)
        })
    }
}

// ---------------------------------------------------------------------------
// Helper to create test data
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod test_helpers {
    use chrono::Utc;
    use hyphae_core::{Chunk, ChunkMetadata, Document, DocumentId, SourceType};

    pub fn make_document(path: &str) -> Document {
        let now = Utc::now();
        Document {
            id: DocumentId::new(),
            source_path: path.to_string(),
            source_type: SourceType::Text,
            chunk_count: 0,
            created_at: now,
            updated_at: now,
            project: None,
            runtime_session_id: None,
            content_hash: None,
        }
    }

    pub fn make_chunk(doc_id: &DocumentId, index: u32, content: &str) -> Chunk {
        Chunk {
            id: hyphae_core::ChunkId::new(),
            document_id: doc_id.clone(),
            chunk_index: index,
            content: content.to_string(),
            metadata: ChunkMetadata {
                source_path: "test.txt".to_string(),
                source_type: SourceType::Text,
                language: None,
                heading: None,
                line_start: None,
                line_end: None,
                chunk_strategy: None,
            },
            embedding: None,
            created_at: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_helpers::{make_chunk, make_document};
    use super::*;
    use crate::store::{SqliteStore, test_helpers::ensure_vec_init};

    fn make_store() -> SqliteStore {
        SqliteStore::in_memory().unwrap()
    }

    #[test]
    fn test_store_chunks_partial_failure_resilience() {
        // Test that when one chunk fails to insert (due to wrong embedding dimension),
        // the batch returns Ok(good_count) and good chunks are persisted while the bad one is skipped.
        ensure_vec_init();
        let store = make_store();

        let doc = make_document("test.md");
        store.store_document(doc.clone()).unwrap();

        // Create a batch of 5 chunks: 0, 1, 2(BAD), 3, 4
        let chunk_0 = make_chunk(&doc.id, 0, "good chunk 0");
        let chunk_1 = make_chunk(&doc.id, 1, "good chunk 1");

        let mut chunk_2_bad = make_chunk(&doc.id, 2, "bad chunk with wrong embedding dimension");
        // Create an embedding with wrong dimension (100 instead of expected 384)
        chunk_2_bad.embedding = Some(vec![0.5; 100]);
        let chunk_2_bad_id = chunk_2_bad.id.clone();

        let chunk_3 = make_chunk(&doc.id, 3, "good chunk 3");
        let chunk_4 = make_chunk(&doc.id, 4, "good chunk 4");

        let chunks = vec![
            chunk_0.clone(),
            chunk_1.clone(),
            chunk_2_bad,
            chunk_3.clone(),
            chunk_4.clone(),
        ];

        // store_chunks should return Ok(4) since one chunk fails
        let result = store.store_chunks(chunks).unwrap();
        assert_eq!(
            result, 4,
            "Expected 4 chunks stored (1 skipped due to embedding dimension mismatch)"
        );

        // Verify that the good chunks were persisted
        let persisted = store.get_chunks(&doc.id).unwrap();
        assert_eq!(persisted.len(), 4, "Expected 4 persisted chunks");

        // Check that chunk 0, 1, 3, 4 are present and chunk 2 is absent
        let persisted_ids: Vec<String> = persisted.iter().map(|c| c.id.to_string()).collect();
        assert!(persisted_ids.contains(&chunk_0.id.to_string()));
        assert!(persisted_ids.contains(&chunk_1.id.to_string()));
        assert!(
            !persisted_ids.contains(&chunk_2_bad_id.to_string()),
            "Bad chunk should not be persisted"
        );
        assert!(persisted_ids.contains(&chunk_3.id.to_string()));
        assert!(persisted_ids.contains(&chunk_4.id.to_string()));

        // Verify the content is correct
        let persisted_contents: Vec<String> = persisted.iter().map(|c| c.content.clone()).collect();
        assert!(persisted_contents.contains(&"good chunk 0".to_string()));
        assert!(persisted_contents.contains(&"good chunk 1".to_string()));
        assert!(persisted_contents.contains(&"good chunk 3".to_string()));
        assert!(persisted_contents.contains(&"good chunk 4".to_string()));
        assert!(
            !persisted_contents.contains(&"bad chunk with wrong embedding dimension".to_string())
        );
    }

    #[test]
    fn test_store_chunks_all_success_returns_full_count() {
        ensure_vec_init();
        let store = make_store();

        let doc = make_document("test.md");
        store.store_document(doc.clone()).unwrap();

        let chunks = vec![
            make_chunk(&doc.id, 0, "chunk 0"),
            make_chunk(&doc.id, 1, "chunk 1"),
            make_chunk(&doc.id, 2, "chunk 2"),
        ];
        let expected_count = chunks.len();

        let result = store.store_chunks(chunks).unwrap();
        assert_eq!(
            result, expected_count,
            "All chunks should be stored successfully"
        );

        let persisted = store.get_chunks(&doc.id).unwrap();
        assert_eq!(persisted.len(), expected_count);
    }

    #[test]
    fn test_store_chunks_empty_batch() {
        ensure_vec_init();
        let store = make_store();

        let result = store.store_chunks(vec![]).unwrap();
        assert_eq!(result, 0, "Empty batch should return 0");
    }

    #[test]
    fn test_store_chunks_crosses_commit_boundary_with_failures() {
        // CHUNK_TX_BATCH is 64, so a 130-chunk batch spans three transactions
        // (commits at indices 64 and 128). Plant failures in each transaction —
        // before the first boundary (10), just after it (70), and in the tail
        // transaction (129) — to prove the savepoint-skip path survives the
        // commit/reopen cycle and the success count is correct across boundaries.
        ensure_vec_init();
        let store = make_store();

        let doc = make_document("boundary.md");
        store.store_document(doc.clone()).unwrap();

        let bad_indices = [10usize, 70, 129];
        let total = 130usize;
        let mut bad_ids = Vec::new();
        let mut chunks = Vec::with_capacity(total);
        for idx in 0..total {
            let mut chunk = make_chunk(&doc.id, idx as u32, &format!("chunk {idx}"));
            if bad_indices.contains(&idx) {
                // Wrong embedding dimension → deterministic vec_chunks INSERT failure.
                chunk.embedding = Some(vec![0.5; 100]);
                bad_ids.push(chunk.id.to_string());
            }
            chunks.push(chunk);
        }

        let expected_good = total - bad_indices.len();
        let result = store.store_chunks(chunks).unwrap();
        assert_eq!(
            result, expected_good,
            "Expected {expected_good} chunks stored across the commit boundary (3 skipped)"
        );

        let persisted = store.get_chunks(&doc.id).unwrap();
        assert_eq!(
            persisted.len(),
            expected_good,
            "Persisted count must match success count"
        );

        let persisted_ids: Vec<String> = persisted.iter().map(|c| c.id.to_string()).collect();
        for bad_id in &bad_ids {
            assert!(
                !persisted_ids.contains(bad_id),
                "Bad chunk {bad_id} must not be persisted"
            );
        }
    }
}
