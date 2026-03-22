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
     c.source_type, c.language, c.heading, c.line_start, c.line_end, c.created_at";

impl ChunkStore for SqliteStore {
    fn store_document(&self, doc: Document) -> HyphaeResult<DocumentId> {
        let id = doc.id.clone();
        self.conn
            .prepare_cached(&format!(
                "INSERT OR REPLACE INTO documents ({DOCUMENT_COLS}) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"
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
            ])
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        Ok(id)
    }

    fn store_chunks(&self, chunks: Vec<Chunk>) -> HyphaeResult<usize> {
        if chunks.is_empty() {
            return Ok(0);
        }

        // SAFETY: No nested transactions — this method does not call other &self methods
        // that open transactions. The &self receiver is required by the ChunkStore trait.
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let count = chunks.len();
        for chunk in chunks {
            let now = chunk.created_at.to_rfc3339();
            tx.prepare_cached(&format!(
                "INSERT OR REPLACE INTO chunks ({CHUNK_COLS}) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)"
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
            ])
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

            tx.prepare_cached(
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
                tx.prepare_cached(
                    "INSERT OR REPLACE INTO vec_chunks (chunk_id, embedding) VALUES (?1, ?2)",
                )
                .map_err(|e| HyphaeError::Database(e.to_string()))?
                .execute(params![chunk.id.to_string(), blob])
                .map_err(|e| HyphaeError::Database(e.to_string()))?;
            }
        }

        tx.commit()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        Ok(count)
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
                "SELECT {DOCUMENT_COLS} FROM documents WHERE source_path = ?1 AND (project = ?2 OR ?2 IS NULL)"
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
                    let rank: f32 = row.get(11)?;
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
                        let rank: f32 = row.get(11)?;
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
            },
            embedding: None,
            created_at: Utc::now(),
        }
    }
}
