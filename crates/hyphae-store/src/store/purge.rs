// ─────────────────────────────────────────────────────────────────────────────
// Purge Operations (GDPR/Retention Compliance)
// ─────────────────────────────────────────────────────────────────────────────

use rusqlite::params;

use hyphae_core::{HyphaeError, HyphaeResult};

use super::SqliteStore;

impl SqliteStore {
    /// Count memories for a given project.
    pub fn count_memories_by_project(&self, project: &str) -> HyphaeResult<usize> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE project = ?1",
                params![project],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n as usize)
            .map_err(|e| HyphaeError::Database(e.to_string()))
    }

    /// Count memories created before a given date.
    pub fn count_memories_before_date(&self, before_dt: &str) -> HyphaeResult<usize> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE created_at < ?1",
                params![before_dt],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n as usize)
            .map_err(|e| HyphaeError::Database(e.to_string()))
    }

    /// Count sessions for a given project.
    pub fn count_sessions_by_project(&self, project: &str) -> HyphaeResult<usize> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE project = ?1",
                params![project],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n as usize)
            .map_err(|e| HyphaeError::Database(e.to_string()))
    }

    /// Count sessions started before a given date.
    pub fn count_sessions_before_date(&self, before_dt: &str) -> HyphaeResult<usize> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE started_at < ?1",
                params![before_dt],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n as usize)
            .map_err(|e| HyphaeError::Database(e.to_string()))
    }

    /// Count chunks in documents for a given project.
    pub fn count_chunks_by_project(&self, project: &str) -> HyphaeResult<usize> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM chunks WHERE document_id IN (
                    SELECT id FROM documents WHERE project = ?1
                )",
                params![project],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n as usize)
            .map_err(|e| HyphaeError::Database(e.to_string()))
    }

    /// Count documents for a given project.
    pub fn count_documents_by_project(&self, project: &str) -> HyphaeResult<usize> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM documents WHERE project = ?1",
                params![project],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n as usize)
            .map_err(|e| HyphaeError::Database(e.to_string()))
    }

    /// Count documents created before a given date.
    pub fn count_documents_before_date(&self, before_dt: &str) -> HyphaeResult<usize> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM documents WHERE created_at < ?1",
                params![before_dt],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n as usize)
            .map_err(|e| HyphaeError::Database(e.to_string()))
    }

    /// Count chunks in documents created before a given date.
    pub fn count_chunks_before_date(&self, before_dt: &str) -> HyphaeResult<usize> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM chunks WHERE document_id IN (
                    SELECT id FROM documents WHERE created_at < ?1
                )",
                params![before_dt],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n as usize)
            .map_err(|e| HyphaeError::Database(e.to_string()))
    }

    /// Delete all data for a specific project.
    /// Returns (memories_deleted, sessions_deleted, chunks_deleted, documents_deleted).
    pub fn purge_project(&self, project: &str) -> HyphaeResult<(usize, usize, usize, usize)> {
        // SAFETY: No nested transactions — this method does not call other &self methods
        // that open transactions. The &self receiver is required by SqliteStore.
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        // Delete vector embeddings for memories
        tx.execute(
            "DELETE FROM vec_memories WHERE memory_id IN (
                SELECT id FROM memories WHERE project = ?1
            )",
            params![project],
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

        tx.execute(
            "DELETE FROM vec_chunks WHERE chunk_id IN (
                SELECT id FROM chunks WHERE document_id IN (
                    SELECT id FROM documents WHERE project = ?1
                )
            )",
            params![project],
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

        tx.execute(
            "DELETE FROM chunks_fts WHERE id IN (
                SELECT id FROM chunks WHERE document_id IN (
                    SELECT id FROM documents WHERE project = ?1
                )
            )",
            params![project],
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

        // Delete memories
        let memories_deleted = tx
            .execute("DELETE FROM memories WHERE project = ?1", params![project])
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        // Delete sessions
        let sessions_deleted = tx
            .execute("DELETE FROM sessions WHERE project = ?1", params![project])
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        // Delete chunks (cascades from documents deletion)
        let chunks_deleted = tx
            .execute(
                "DELETE FROM chunks WHERE document_id IN (
                    SELECT id FROM documents WHERE project = ?1
                )",
                params![project],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        // Delete documents
        let documents_deleted = tx
            .execute("DELETE FROM documents WHERE project = ?1", params![project])
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        tx.commit()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        Ok((
            memories_deleted,
            sessions_deleted,
            chunks_deleted,
            documents_deleted,
        ))
    }

    /// Delete all data created before a specific date (ISO 8601 format).
    /// Returns (memories_deleted, sessions_deleted, chunks_deleted, documents_deleted).
    pub fn purge_before_date(&self, before_dt: &str) -> HyphaeResult<(usize, usize, usize, usize)> {
        // SAFETY: No nested transactions — this method does not call other &self methods
        // that open transactions. The &self receiver is required by SqliteStore.
        let tx = self
            .conn
            .unchecked_transaction()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        // Delete vector embeddings for memories
        tx.execute(
            "DELETE FROM vec_memories WHERE memory_id IN (
                SELECT id FROM memories WHERE created_at < ?1
            )",
            params![before_dt],
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

        tx.execute(
            "DELETE FROM vec_chunks WHERE chunk_id IN (
                SELECT id FROM chunks WHERE document_id IN (
                    SELECT id FROM documents WHERE created_at < ?1
                )
            )",
            params![before_dt],
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

        tx.execute(
            "DELETE FROM chunks_fts WHERE id IN (
                SELECT id FROM chunks WHERE document_id IN (
                    SELECT id FROM documents WHERE created_at < ?1
                )
            )",
            params![before_dt],
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

        // Delete memories
        let memories_deleted = tx
            .execute(
                "DELETE FROM memories WHERE created_at < ?1",
                params![before_dt],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        // Delete sessions
        let sessions_deleted = tx
            .execute(
                "DELETE FROM sessions WHERE started_at < ?1",
                params![before_dt],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        // Delete chunks
        let chunks_deleted = tx
            .execute(
                "DELETE FROM chunks WHERE document_id IN (
                    SELECT id FROM documents WHERE created_at < ?1
                )",
                params![before_dt],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        // Delete documents
        let documents_deleted = tx
            .execute(
                "DELETE FROM documents WHERE created_at < ?1",
                params![before_dt],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        tx.commit()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        Ok((
            memories_deleted,
            sessions_deleted,
            chunks_deleted,
            documents_deleted,
        ))
    }
}
