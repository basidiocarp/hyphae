mod chunk_store;
pub mod context;
mod helpers;
mod memoir_store;
mod memory_store;
mod search;
pub mod session;

pub use search::UnifiedSearchResult;

use std::path::Path;
use std::sync::Once;

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, ffi::sqlite3_auto_extension, params};

use hyphae_core::{HyphaeError, HyphaeResult, Memory, MemoryStore};

use crate::schema::{init_db, init_db_with_dims};

static SQLITE_VEC_INIT: Once = Once::new();

fn ensure_sqlite_vec() {
    SQLITE_VEC_INIT.call_once(|| unsafe {
        #[allow(clippy::missing_transmute_annotations)]
        sqlite3_auto_extension(Some(std::mem::transmute(
            sqlite_vec::sqlite3_vec_init as *const (),
        )));
    });
}

pub struct SqliteStore {
    pub(crate) conn: Connection,
}

impl SqliteStore {
    pub fn new(path: &Path) -> HyphaeResult<Self> {
        Self::with_dims(path, 384)
    }

    /// Open or create a store with a specific embedding dimension.
    pub fn with_dims(path: &Path, embedding_dims: usize) -> HyphaeResult<Self> {
        ensure_sqlite_vec();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| HyphaeError::Database(format!("cannot create db directory: {e}")))?;
        }
        let conn = Connection::open(path)
            .map_err(|e| HyphaeError::Database(format!("cannot open database: {e}")))?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        init_db_with_dims(&conn, embedding_dims)?;
        Ok(Self { conn })
    }

    /// Apply decay if more than 24 hours since last decay.
    /// Called automatically on recall to avoid manual `hyphae decay` cron.
    pub fn maybe_auto_decay(&self) -> HyphaeResult<()> {
        let now = Utc::now();

        let last: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM hyphae_metadata WHERE key = 'last_decay_at'",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let should_decay = match last {
            Some(ts) => {
                let last_dt = DateTime::parse_from_rfc3339(&ts)
                    .map(|d| d.with_timezone(&Utc))
                    .unwrap_or_else(|_| now - chrono::Duration::hours(25));
                (now - last_dt).num_hours() >= 24
            }
            None => true,
        };

        if should_decay {
            self.apply_decay(0.95)?;
            self.conn
                .execute(
                    "INSERT INTO hyphae_metadata (key, value) VALUES ('last_decay_at', ?1)
                     ON CONFLICT(key) DO UPDATE SET value = ?1",
                    params![now.to_rfc3339()],
                )
                .map_err(|e| HyphaeError::Database(e.to_string()))?;
        }

        Ok(())
    }

    /// Count expired ephemeral memories without deleting them.
    pub fn count_expired(&self) -> HyphaeResult<usize> {
        let now = Utc::now().to_rfc3339();
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE expires_at IS NOT NULL AND expires_at < ?1",
                params![now],
                |row| row.get::<_, usize>(0),
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))
    }

    /// Count low-weight memories that would be pruned at the given threshold.
    pub fn count_low_weight(&self, weight_threshold: f32) -> HyphaeResult<usize> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE weight < ?1 AND importance NOT IN ('critical', 'high')",
                params![weight_threshold],
                |row| row.get::<_, usize>(0),
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))
    }

    /// Check if a memory exists with a specific hash keyword (for deduplication).
    pub fn memory_exists_with_keyword(&self, keyword: &str) -> HyphaeResult<bool> {
        let pattern = format!("%\"hash:{keyword}\"%");
        let exists: bool = self
            .conn
            .query_row(
                "SELECT 1 FROM memories WHERE keywords LIKE ?1 LIMIT 1",
                params![pattern],
                |_row| Ok(true),
            )
            .optional()
            .map_err(|e| HyphaeError::Database(e.to_string()))?
            .unwrap_or(false);
        Ok(exists)
    }

    pub fn in_memory() -> HyphaeResult<Self> {
        ensure_sqlite_vec();
        let conn = Connection::open_in_memory()
            .map_err(|e| HyphaeError::Database(format!("cannot open in-memory db: {e}")))?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        init_db(&conn)?;
        Ok(Self { conn })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Cross-project search and project management
// ─────────────────────────────────────────────────────────────────────────────

/// Name of the special shared knowledge pool project.
pub const SHARED_PROJECT: &str = "_shared";

impl SqliteStore {
    /// FTS search across all projects (no project filter).
    /// Results include the `project` field so the caller knows the source.
    pub fn search_all_projects(&self, query: &str, limit: usize) -> HyphaeResult<Vec<Memory>> {
        let sanitized = search::sanitize_fts_query(query);
        if sanitized.is_empty() {
            return Ok(Vec::new());
        }

        let sql = format!(
            "SELECT {cols} FROM memories m
             WHERE m.id IN (
                 SELECT id FROM memories_fts WHERE memories_fts MATCH ?1
             )
             ORDER BY m.weight DESC
             LIMIT ?2",
            cols = helpers::SELECT_COLS,
        );

        let mut stmt = self
            .conn
            .prepare_cached(&sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![sanitized, limit as i64], helpers::row_to_memory)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| HyphaeError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    /// FTS search across a specific set of projects.
    /// Results ranked by relevance (FTS score via weight), not project affinity.
    pub fn search_related_projects(
        &self,
        query: &str,
        projects: &[&str],
        limit: usize,
    ) -> HyphaeResult<Vec<Memory>> {
        if projects.is_empty() {
            return Ok(Vec::new());
        }

        let sanitized = search::sanitize_fts_query(query);
        if sanitized.is_empty() {
            return Ok(Vec::new());
        }

        let placeholders: Vec<String> =
            (0..projects.len()).map(|i| format!("?{}", i + 3)).collect();
        let in_clause = placeholders.join(",");

        // ─────────────────────────────────────────────────────────────────────
        // FTS5 search with project filter using UNINDEXED column
        // ─────────────────────────────────────────────────────────────────────
        let sql = format!(
            "SELECT {cols} FROM memories m
             WHERE m.id IN (
                 SELECT id FROM memories_fts
                 WHERE memories_fts MATCH ?1
                 AND project IN ({in_clause})
             )
             ORDER BY m.weight DESC
             LIMIT ?2",
            cols = helpers::SELECT_COLS,
        );

        let mut stmt = self
            .conn
            .prepare(&sql)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        param_values.push(Box::new(sanitized));
        param_values.push(Box::new(limit as i64));
        for p in projects {
            param_values.push(Box::new(p.to_string()));
        }

        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|p| p.as_ref()).collect();

        let rows = stmt
            .query_map(params_ref.as_slice(), helpers::row_to_memory)
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| HyphaeError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    /// List all distinct projects with their memory counts.
    pub fn list_projects(&self) -> HyphaeResult<Vec<(String, usize)>> {
        let mut stmt = self
            .conn
            .prepare_cached(
                "SELECT COALESCE(project, '(none)'), COUNT(*) FROM memories GROUP BY project ORDER BY project",
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, usize>(1)?))
            })
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| HyphaeError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    /// Link two projects together (bidirectional).
    pub fn link_projects(&self, source: &str, target: &str) -> HyphaeResult<()> {
        let now = Utc::now().to_rfc3339();
        self.conn
            .execute(
                "INSERT OR IGNORE INTO project_links (source_project, target_project, created_at) VALUES (?1, ?2, ?3)",
                params![source, target, now],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        self.conn
            .execute(
                "INSERT OR IGNORE INTO project_links (source_project, target_project, created_at) VALUES (?1, ?2, ?3)",
                params![target, source, now],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        Ok(())
    }

    /// Get linked projects for a given project.
    pub fn get_linked_projects(&self, project: &str) -> HyphaeResult<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare_cached(
                "SELECT target_project FROM project_links WHERE source_project = ?1 ORDER BY target_project",
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let rows = stmt
            .query_map(params![project], |row| row.get::<_, String>(0))
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut results = Vec::new();
        for row in rows {
            results.push(row.map_err(|e| HyphaeError::Database(e.to_string()))?);
        }
        Ok(results)
    }

    /// Promote (copy) a memory to the `_shared` project.
    /// Returns the new memory ID.
    pub fn promote_to_shared(
        &self,
        id: &hyphae_core::MemoryId,
    ) -> HyphaeResult<hyphae_core::MemoryId> {
        let original = self
            .get(id)?
            .ok_or_else(|| HyphaeError::NotFound(id.to_string()))?;

        let shared = Memory::builder(
            original.topic.clone(),
            original.summary.clone(),
            original.importance,
        )
        .keywords(original.keywords.clone())
        .project(SHARED_PROJECT.to_string())
        .build();

        let new_id = self.store(shared)?;
        Ok(new_id)
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Purge Operations (GDPR/Retention Compliance)
    // ─────────────────────────────────────────────────────────────────────────

    /// Count memories for a given project.
    pub fn count_memories_by_project(&self, project: &str) -> HyphaeResult<usize> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE project = ?1",
                params![project],
                |row| row.get(0),
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))
    }

    /// Count memories created before a given date.
    pub fn count_memories_before_date(&self, before_dt: &str) -> HyphaeResult<usize> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM memories WHERE created_at < ?1",
                params![before_dt],
                |row| row.get(0),
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))
    }

    /// Count sessions for a given project.
    pub fn count_sessions_by_project(&self, project: &str) -> HyphaeResult<usize> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE project = ?1",
                params![project],
                |row| row.get(0),
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))
    }

    /// Count sessions started before a given date.
    pub fn count_sessions_before_date(&self, before_dt: &str) -> HyphaeResult<usize> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM sessions WHERE started_at < ?1",
                params![before_dt],
                |row| row.get(0),
            )
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
                |row| row.get(0),
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))
    }

    /// Count chunks created before a given date.
    pub fn count_chunks_before_date(&self, before_dt: &str) -> HyphaeResult<usize> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM chunks WHERE created_at < ?1",
                params![before_dt],
                |row| row.get(0),
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))
    }

    /// Count documents for a given project.
    pub fn count_documents_by_project(&self, project: &str) -> HyphaeResult<usize> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM documents WHERE project = ?1",
                params![project],
                |row| row.get(0),
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))
    }

    /// Count documents created before a given date.
    pub fn count_documents_before_date(&self, before_dt: &str) -> HyphaeResult<usize> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM documents WHERE created_at < ?1",
                params![before_dt],
                |row| row.get(0),
            )
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
                "DELETE FROM chunks WHERE created_at < ?1",
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

#[cfg(test)]
pub(crate) mod test_helpers {
    use super::ensure_sqlite_vec;

    pub fn ensure_vec_init() {
        ensure_sqlite_vec();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyphae_core::{
        Concept, ConceptLink, Importance, Memoir, MemoirId, MemoirStore, Memory, MemoryId,
        Relation, Weight,
    };

    fn test_store() -> SqliteStore {
        SqliteStore::in_memory().unwrap()
    }

    fn make_memory(topic: &str, summary: &str) -> Memory {
        Memory::new(topic.into(), summary.into(), Importance::Medium)
    }

    fn make_memoir(name: &str) -> Memoir {
        Memoir::new(name.into(), format!("Description for {name}"))
    }

    fn make_concept(memoir_id: &MemoirId, name: &str, definition: &str) -> Concept {
        Concept::new(memoir_id.clone(), name.into(), definition.into())
    }

    // === MemoryStore tests ===

    #[test]
    fn test_store_and_get() {
        let store = test_store();
        let mem = make_memory("test", "hello world");
        let id = mem.id.clone();

        store.store(mem).unwrap();
        let retrieved = store.get(&id).unwrap().unwrap();
        assert_eq!(retrieved.summary, "hello world");
        assert_eq!(retrieved.topic, "test");
    }

    #[test]
    fn test_get_not_found() {
        let store = test_store();
        let result = store.get(&MemoryId::from("nonexistent")).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_update() {
        let store = test_store();
        let mut mem = make_memory("test", "original");
        let id = mem.id.clone();
        store.store(mem.clone()).unwrap();

        mem.summary = "updated".into();
        store.update(&mem).unwrap();

        let retrieved = store.get(&id).unwrap().unwrap();
        assert_eq!(retrieved.summary, "updated");
    }

    #[test]
    fn test_delete() {
        let store = test_store();
        let mem = make_memory("test", "to delete");
        let id = mem.id.clone();
        store.store(mem).unwrap();

        store.delete(&id).unwrap();
        assert!(store.get(&id).unwrap().is_none());
    }

    #[test]
    fn test_delete_not_found() {
        let store = test_store();
        let result = store.delete(&MemoryId::from("nonexistent"));
        assert!(result.is_err());
    }

    #[test]
    fn test_search_by_keywords() {
        let store = test_store();
        store
            .store(make_memory("rust", "ownership and borrowing"))
            .unwrap();
        store
            .store(make_memory("rust", "lifetimes in rust"))
            .unwrap();
        store
            .store(make_memory("python", "python decorators"))
            .unwrap();

        let results = store.search_by_keywords(&["rust"], 10, 0, None).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_search_by_keywords_empty() {
        let store = test_store();
        let results = store.search_by_keywords(&[], 10, 0, None).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_fts() {
        let store = test_store();
        store
            .store(make_memory("rust", "ownership and borrowing in Rust"))
            .unwrap();
        store
            .store(make_memory("rust", "lifetimes are important"))
            .unwrap();
        store
            .store(make_memory("python", "python decorators are cool"))
            .unwrap();

        let results = store
            .search_fts("ownership borrowing", 10, 0, None)
            .unwrap();
        assert!(!results.is_empty());
    }

    #[test]
    fn test_search_fts_empty_query() {
        let store = test_store();
        let results = store.search_fts("", 10, 0, None).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_fts_special_chars() {
        let store = test_store();
        store
            .store(make_memory("deps", "sqlite-vec is a vector extension"))
            .unwrap();

        let results = store.search_fts("sqlite-vec", 10, 0, None).unwrap();
        assert!(!results.is_empty());
    }

    #[test]
    fn test_update_access() {
        let store = test_store();
        let mem = make_memory("test", "access test");
        let id = mem.id.clone();
        store.store(mem).unwrap();

        store.update_access(&id).unwrap();
        let retrieved = store.get(&id).unwrap().unwrap();
        assert_eq!(retrieved.access_count, 1);
    }

    #[test]
    fn test_apply_decay() {
        let store = test_store();
        store.store(make_memory("test", "decay test")).unwrap();

        let affected = store.apply_decay(0.95).unwrap();
        assert!(affected > 0);
    }

    #[test]
    fn test_prune() {
        let store = test_store();
        let mut mem = make_memory("test", "low weight");
        mem.weight = Weight::new_clamped(0.01);
        store.store(mem).unwrap();

        let pruned = store.prune(0.1).unwrap();
        assert_eq!(pruned, 1);
    }

    #[test]
    fn test_prune_respects_importance() {
        let store = test_store();

        let mut critical = make_memory("test", "critical memory");
        critical.weight = Weight::new_clamped(0.01);
        critical.importance = Importance::Critical;
        store.store(critical).unwrap();

        let mut high = make_memory("test", "high memory");
        high.weight = Weight::new_clamped(0.01);
        high.importance = Importance::High;
        store.store(high).unwrap();

        let pruned = store.prune(0.1).unwrap();
        assert_eq!(pruned, 0);
    }

    #[test]
    fn test_get_by_topic() {
        let store = test_store();
        store.store(make_memory("rust", "rust memory 1")).unwrap();
        store.store(make_memory("rust", "rust memory 2")).unwrap();
        store.store(make_memory("python", "python memory")).unwrap();

        let results = store.get_by_topic("rust", None).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn test_list_topics() {
        let store = test_store();
        store.store(make_memory("rust", "rust 1")).unwrap();
        store.store(make_memory("rust", "rust 2")).unwrap();
        store.store(make_memory("python", "python 1")).unwrap();

        let topics = store.list_topics(None).unwrap();
        assert_eq!(topics.len(), 2);
    }

    #[test]
    fn test_consolidate_topic() {
        let store = test_store();
        store.store(make_memory("rust", "rust 1")).unwrap();
        store.store(make_memory("rust", "rust 2")).unwrap();

        let consolidated = make_memory("rust", "consolidated rust knowledge");
        store.consolidate_topic("rust", consolidated).unwrap();

        let results = store.get_by_topic("rust", None).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].summary, "consolidated rust knowledge");
    }

    #[test]
    fn test_count() {
        let store = test_store();
        assert_eq!(store.count(None).unwrap(), 0);

        store.store(make_memory("test", "one")).unwrap();
        store.store(make_memory("test", "two")).unwrap();
        assert_eq!(store.count(None).unwrap(), 2);
    }

    #[test]
    fn test_count_by_topic() {
        let store = test_store();
        store.store(make_memory("rust", "rust 1")).unwrap();
        store.store(make_memory("rust", "rust 2")).unwrap();
        store.store(make_memory("python", "python 1")).unwrap();

        assert_eq!(store.count_by_topic("rust", None).unwrap(), 2);
        assert_eq!(store.count_by_topic("python", None).unwrap(), 1);
        assert_eq!(store.count_by_topic("go", None).unwrap(), 0);
    }

    #[test]
    fn test_topic_health() {
        let store = test_store();
        store.store(make_memory("rust", "rust 1")).unwrap();
        store.store(make_memory("rust", "rust 2")).unwrap();

        let health = store.topic_health("rust", None).unwrap();
        assert_eq!(health.entry_count, 2);
        assert!(health.avg_weight > 0.0);
    }

    #[test]
    fn test_topic_health_not_found() {
        let store = test_store();
        let result = store.topic_health("nonexistent", None);
        assert!(result.is_err());
    }

    #[test]
    fn test_stats() {
        let store = test_store();
        store.store(make_memory("rust", "rust 1")).unwrap();
        store.store(make_memory("python", "python 1")).unwrap();

        let stats = store.stats(None).unwrap();
        assert_eq!(stats.total_memories, 2);
        assert_eq!(stats.total_topics, 2);
    }

    #[test]
    fn test_stats_empty() {
        let store = test_store();
        let stats = store.stats(None).unwrap();
        assert_eq!(stats.total_memories, 0);
        assert_eq!(stats.total_topics, 0);
        assert_eq!(stats.avg_weight, 0.0);
    }

    // === Embedding tests ===

    #[test]
    fn test_store_with_embedding() {
        let store = test_store();
        let mut mem = make_memory("test", "embedding test");
        mem.embedding = Some(vec![0.1; 384]);
        let id = mem.id.clone();

        store.store(mem).unwrap();
        let retrieved = store.get(&id).unwrap().unwrap();
        assert!(retrieved.embedding.is_some());
        assert_eq!(retrieved.embedding.unwrap().len(), 384);
    }

    #[test]
    fn test_search_by_embedding() {
        let store = test_store();
        let mut mem = make_memory("test", "vector search test");
        mem.embedding = Some(vec![0.1; 384]);
        store.store(mem).unwrap();

        let query_emb = vec![0.1; 384];
        let results = store.search_by_embedding(&query_emb, 5, 0, None).unwrap();
        assert!(!results.is_empty());
    }

    #[test]
    fn test_hybrid_search() {
        let store = test_store();
        let mut mem = make_memory("test", "hybrid search with vectors and text");
        mem.embedding = Some(vec![0.1; 384]);
        store.store(mem).unwrap();

        let query_emb = vec![0.1; 384];
        let results = store
            .search_hybrid("hybrid search", &query_emb, 5, 0, None)
            .unwrap();
        assert!(!results.is_empty());
    }

    #[test]
    fn test_update_with_embedding() {
        let store = test_store();
        let mut mem = make_memory("test", "update embedding");
        mem.embedding = Some(vec![0.1; 384]);
        let id = mem.id.clone();
        store.store(mem.clone()).unwrap();

        mem.embedding = Some(vec![0.2; 384]);
        store.update(&mem).unwrap();

        let retrieved = store.get(&id).unwrap().unwrap();
        let emb = retrieved.embedding.unwrap();
        assert!((emb[0] - 0.2).abs() < 0.001);
    }

    #[test]
    fn test_delete_with_embedding() {
        let store = test_store();
        let mut mem = make_memory("test", "delete embedding");
        mem.embedding = Some(vec![0.1; 384]);
        let id = mem.id.clone();
        store.store(mem).unwrap();

        store.delete(&id).unwrap();
        assert!(store.get(&id).unwrap().is_none());
    }

    // === MemoirStore tests ===

    #[test]
    fn test_create_and_get_memoir() {
        let store = test_store();
        let memoir = make_memoir("Rust Knowledge");
        let id = memoir.id.clone();

        store.create_memoir(memoir).unwrap();
        let retrieved = store.get_memoir(&id).unwrap().unwrap();
        assert_eq!(retrieved.name, "Rust Knowledge");
    }

    #[test]
    fn test_get_memoir_not_found() {
        let store = test_store();
        let result = store.get_memoir(&MemoirId::from("nonexistent")).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_get_memoir_by_name() {
        let store = test_store();
        let memoir = make_memoir("Rust Knowledge");
        store.create_memoir(memoir).unwrap();

        let retrieved = store.get_memoir_by_name("Rust Knowledge").unwrap().unwrap();
        assert_eq!(retrieved.name, "Rust Knowledge");
    }

    #[test]
    fn test_update_memoir() {
        let store = test_store();
        let mut memoir = make_memoir("Original");
        let id = memoir.id.clone();
        store.create_memoir(memoir.clone()).unwrap();

        memoir.name = "Updated".into();
        store.update_memoir(&memoir).unwrap();

        let retrieved = store.get_memoir(&id).unwrap().unwrap();
        assert_eq!(retrieved.name, "Updated");
    }

    #[test]
    fn test_delete_memoir() {
        let store = test_store();
        let memoir = make_memoir("To Delete");
        let id = memoir.id.clone();
        store.create_memoir(memoir).unwrap();

        store.delete_memoir(&id).unwrap();
        assert!(store.get_memoir(&id).unwrap().is_none());
    }

    #[test]
    fn test_list_memoirs() {
        let store = test_store();
        store.create_memoir(make_memoir("Alpha")).unwrap();
        store.create_memoir(make_memoir("Beta")).unwrap();

        let memoirs = store.list_memoirs().unwrap();
        assert_eq!(memoirs.len(), 2);
        assert_eq!(memoirs[0].name, "Alpha");
        assert_eq!(memoirs[1].name, "Beta");
    }

    // === Concept tests ===

    #[test]
    fn test_add_and_get_concept() {
        let store = test_store();
        let memoir = make_memoir("Test");
        let memoir_id = memoir.id.clone();
        store.create_memoir(memoir).unwrap();

        let concept = make_concept(&memoir_id, "Ownership", "Rust ownership model");
        let id = concept.id.clone();
        store.add_concept(concept).unwrap();

        let retrieved = store.get_concept(&id).unwrap().unwrap();
        assert_eq!(retrieved.name, "Ownership");
    }

    #[test]
    fn test_get_concept_by_name() {
        let store = test_store();
        let memoir = make_memoir("Test");
        let memoir_id = memoir.id.clone();
        store.create_memoir(memoir).unwrap();

        let concept = make_concept(&memoir_id, "Borrowing", "Rust borrowing rules");
        store.add_concept(concept).unwrap();

        let retrieved = store
            .get_concept_by_name(&memoir_id, "Borrowing")
            .unwrap()
            .unwrap();
        assert_eq!(retrieved.definition, "Rust borrowing rules");
    }

    #[test]
    fn test_update_concept() {
        let store = test_store();
        let memoir = make_memoir("Test");
        let memoir_id = memoir.id.clone();
        store.create_memoir(memoir).unwrap();

        let mut concept = make_concept(&memoir_id, "Lifetimes", "original definition");
        let id = concept.id.clone();
        store.add_concept(concept.clone()).unwrap();

        concept.definition = "updated definition".into();
        store.update_concept(&concept).unwrap();

        let retrieved = store.get_concept(&id).unwrap().unwrap();
        assert_eq!(retrieved.definition, "updated definition");
    }

    #[test]
    fn test_delete_concept() {
        let store = test_store();
        let memoir = make_memoir("Test");
        let memoir_id = memoir.id.clone();
        store.create_memoir(memoir).unwrap();

        let concept = make_concept(&memoir_id, "Temp", "temporary concept");
        let id = concept.id.clone();
        store.add_concept(concept).unwrap();

        store.delete_concept(&id).unwrap();
        assert!(store.get_concept(&id).unwrap().is_none());
    }

    #[test]
    fn test_list_concepts() {
        let store = test_store();
        let memoir = make_memoir("Test");
        let memoir_id = memoir.id.clone();
        store.create_memoir(memoir).unwrap();

        store
            .add_concept(make_concept(&memoir_id, "Alpha", "first"))
            .unwrap();
        store
            .add_concept(make_concept(&memoir_id, "Beta", "second"))
            .unwrap();

        let concepts = store.list_concepts(&memoir_id).unwrap();
        assert_eq!(concepts.len(), 2);
        assert_eq!(concepts[0].name, "Alpha");
        assert_eq!(concepts[1].name, "Beta");
    }

    #[test]
    fn test_search_concepts_fts() {
        let store = test_store();
        let memoir = make_memoir("Test");
        let memoir_id = memoir.id.clone();
        store.create_memoir(memoir).unwrap();

        store
            .add_concept(make_concept(
                &memoir_id,
                "Ownership",
                "Rust ownership model for memory safety",
            ))
            .unwrap();
        store
            .add_concept(make_concept(
                &memoir_id,
                "Borrowing",
                "References and borrowing rules",
            ))
            .unwrap();

        let results = store
            .search_concepts_fts(&memoir_id, "ownership memory", 10)
            .unwrap();
        assert!(!results.is_empty());
    }

    #[test]
    fn test_search_all_concepts_fts() {
        let store = test_store();
        let m1 = make_memoir("Memoir 1");
        let m1_id = m1.id.clone();
        store.create_memoir(m1).unwrap();

        let m2 = make_memoir("Memoir 2");
        let m2_id = m2.id.clone();
        store.create_memoir(m2).unwrap();

        store
            .add_concept(make_concept(&m1_id, "Ownership", "Rust ownership"))
            .unwrap();
        store
            .add_concept(make_concept(&m2_id, "Ownership2", "Go ownership model"))
            .unwrap();

        let results = store.search_all_concepts_fts("ownership", 10).unwrap();
        assert!(results.len() >= 2);
    }

    #[test]
    fn test_refine_concept() {
        let store = test_store();
        let memoir = make_memoir("Test");
        let memoir_id = memoir.id.clone();
        store.create_memoir(memoir).unwrap();

        let concept = make_concept(&memoir_id, "Ownership", "basic definition");
        let id = concept.id.clone();
        store.add_concept(concept).unwrap();

        store
            .refine_concept(
                &id,
                "refined definition with more detail",
                &[MemoryId::from("src1")],
            )
            .unwrap();

        let refined = store.get_concept(&id).unwrap().unwrap();
        assert_eq!(refined.definition, "refined definition with more detail");
        assert_eq!(refined.revision, 2);
        assert!(refined.confidence.value() > 0.5);
        assert!(refined.source_memory_ids.contains(&MemoryId::from("src1")));
    }

    // === ConceptLink tests ===

    #[test]
    fn test_add_and_get_links() {
        let store = test_store();
        let memoir = make_memoir("Test");
        let memoir_id = memoir.id.clone();
        store.create_memoir(memoir).unwrap();

        let c1 = make_concept(&memoir_id, "Ownership", "ownership model");
        let c1_id = c1.id.clone();
        store.add_concept(c1).unwrap();

        let c2 = make_concept(&memoir_id, "Borrowing", "borrowing rules");
        let c2_id = c2.id.clone();
        store.add_concept(c2).unwrap();

        let link = ConceptLink::new(c1_id.clone(), c2_id.clone(), Relation::DependsOn);
        store.add_link(link).unwrap();

        let from_links = store.get_links_from(&c1_id).unwrap();
        assert_eq!(from_links.len(), 1);
        assert_eq!(from_links[0].target_id, c2_id);

        let to_links = store.get_links_to(&c2_id).unwrap();
        assert_eq!(to_links.len(), 1);
        assert_eq!(to_links[0].source_id, c1_id);
    }

    #[test]
    fn test_delete_link() {
        let store = test_store();
        let memoir = make_memoir("Test");
        let memoir_id = memoir.id.clone();
        store.create_memoir(memoir).unwrap();

        let c1 = make_concept(&memoir_id, "A", "concept a");
        let c1_id = c1.id.clone();
        store.add_concept(c1).unwrap();

        let c2 = make_concept(&memoir_id, "B", "concept b");
        let c2_id = c2.id.clone();
        store.add_concept(c2).unwrap();

        let link = ConceptLink::new(c1_id.clone(), c2_id, Relation::RelatedTo);
        let link_id = link.id.clone();
        store.add_link(link).unwrap();

        store.delete_link(&link_id).unwrap();
        let from_links = store.get_links_from(&c1_id).unwrap();
        assert!(from_links.is_empty());
    }

    #[test]
    fn test_get_neighbors() {
        let store = test_store();
        let memoir = make_memoir("Test");
        let memoir_id = memoir.id.clone();
        store.create_memoir(memoir).unwrap();

        let c1 = make_concept(&memoir_id, "Center", "center concept");
        let c1_id = c1.id.clone();
        store.add_concept(c1).unwrap();

        let c2 = make_concept(&memoir_id, "Left", "left concept");
        let c2_id = c2.id.clone();
        store.add_concept(c2).unwrap();

        let c3 = make_concept(&memoir_id, "Right", "right concept");
        let c3_id = c3.id.clone();
        store.add_concept(c3).unwrap();

        store
            .add_link(ConceptLink::new(c1_id.clone(), c2_id, Relation::DependsOn))
            .unwrap();
        store
            .add_link(ConceptLink::new(c3_id, c1_id.clone(), Relation::RelatedTo))
            .unwrap();

        let neighbors = store.get_neighbors(&c1_id, None).unwrap();
        assert_eq!(neighbors.len(), 2);

        let depends_neighbors = store
            .get_neighbors(&c1_id, Some(Relation::DependsOn))
            .unwrap();
        assert_eq!(depends_neighbors.len(), 1);
    }

    #[test]
    fn test_get_neighborhood() {
        let store = test_store();
        let memoir = make_memoir("Test");
        let memoir_id = memoir.id.clone();
        store.create_memoir(memoir).unwrap();

        let c1 = make_concept(&memoir_id, "Root", "root");
        let c1_id = c1.id.clone();
        store.add_concept(c1).unwrap();

        let c2 = make_concept(&memoir_id, "Child", "child");
        let c2_id = c2.id.clone();
        store.add_concept(c2).unwrap();

        let c3 = make_concept(&memoir_id, "Grandchild", "grandchild");
        let c3_id = c3.id.clone();
        store.add_concept(c3).unwrap();

        store
            .add_link(ConceptLink::new(
                c1_id.clone(),
                c2_id.clone(),
                Relation::PartOf,
            ))
            .unwrap();
        store
            .add_link(ConceptLink::new(c2_id, c3_id, Relation::PartOf))
            .unwrap();

        let (concepts, links) = store.get_neighborhood(&c1_id, 1).unwrap();
        assert_eq!(concepts.len(), 2);
        assert!(!links.is_empty());

        let (concepts_deep, _) = store.get_neighborhood(&c1_id, 2).unwrap();
        assert_eq!(concepts_deep.len(), 3);
    }

    #[test]
    fn test_memoir_stats() {
        let store = test_store();
        let memoir = make_memoir("Test");
        let memoir_id = memoir.id.clone();
        store.create_memoir(memoir).unwrap();

        store
            .add_concept(make_concept(&memoir_id, "A", "concept a"))
            .unwrap();
        store
            .add_concept(make_concept(&memoir_id, "B", "concept b"))
            .unwrap();

        let stats = store.memoir_stats(&memoir_id).unwrap();
        assert_eq!(stats.total_concepts, 2);
        assert_eq!(stats.total_links, 0);
    }

    #[test]
    fn test_auto_decay() {
        let store = test_store();
        store.store(make_memory("test", "decay test")).unwrap();
        store.maybe_auto_decay().unwrap();
        store.maybe_auto_decay().unwrap();
    }

    // === Decay formula tests ===

    #[test]
    fn test_decay_critical_never_decays() {
        let store = test_store();
        let mut mem = make_memory("test", "critical memory");
        mem.importance = Importance::Critical;
        let id = mem.id.clone();
        store.store(mem).unwrap();

        store.apply_decay(0.5).unwrap();

        let retrieved = store.get(&id).unwrap().unwrap();
        assert!(
            (retrieved.weight.value() - 1.0).abs() < 0.001,
            "Critical memory weight should remain 1.0 after decay, got {}",
            retrieved.weight.value()
        );
    }

    #[test]
    fn test_decay_rate_ordering_low_medium_high() {
        let store = test_store();

        let mut high_mem = make_memory("test", "high importance");
        high_mem.importance = Importance::High;
        let high_id = high_mem.id.clone();
        store.store(high_mem).unwrap();

        let mut medium_mem = make_memory("test", "medium importance");
        medium_mem.importance = Importance::Medium;
        let medium_id = medium_mem.id.clone();
        store.store(medium_mem).unwrap();

        let mut low_mem = make_memory("test", "low importance");
        low_mem.importance = Importance::Low;
        let low_id = low_mem.id.clone();
        store.store(low_mem).unwrap();

        store.apply_decay(0.9).unwrap();

        let high = store.get(&high_id).unwrap().unwrap();
        let medium = store.get(&medium_id).unwrap().unwrap();
        let low = store.get(&low_id).unwrap().unwrap();

        // High decays least, Low decays most
        assert!(
            high.weight.value() > medium.weight.value(),
            "High ({}) should decay less than Medium ({})",
            high.weight.value(),
            medium.weight.value()
        );
        assert!(
            medium.weight.value() > low.weight.value(),
            "Medium ({}) should decay less than Low ({})",
            medium.weight.value(),
            low.weight.value()
        );
    }

    #[test]
    fn test_decay_higher_access_count_slows_decay() {
        let store = test_store();

        let mem_no_access = make_memory("test", "no access");
        let id_no_access = mem_no_access.id.clone();
        store.store(mem_no_access).unwrap();

        let mem_high_access = make_memory("test", "high access");
        let id_high_access = mem_high_access.id.clone();
        store.store(mem_high_access).unwrap();

        // Bump access count to 10 for the high-access memory
        for _ in 0..10 {
            store.update_access(&id_high_access).unwrap();
        }

        store.apply_decay(0.9).unwrap();

        let no_access = store.get(&id_no_access).unwrap().unwrap();
        let high_access = store.get(&id_high_access).unwrap().unwrap();

        assert!(
            high_access.weight.value() > no_access.weight.value(),
            "High-access memory ({}) should decay less than no-access memory ({})",
            high_access.weight.value(),
            no_access.weight.value()
        );
    }

    #[test]
    fn test_decay_weight_never_goes_below_zero() {
        let store = test_store();
        let mem = make_memory("test", "decay to zero");
        let id = mem.id.clone();
        store.store(mem).unwrap();

        // Apply many aggressive decay passes
        for _ in 0..100 {
            store.apply_decay(0.0).unwrap();
        }

        let retrieved = store.get(&id).unwrap().unwrap();
        assert!(
            retrieved.weight.value() >= 0.0,
            "Weight should never go below 0.0, got {}",
            retrieved.weight.value()
        );
    }

    // === Consolidation tests ===

    #[test]
    fn test_consolidate_topic_merges_and_deletes_originals() {
        let store = test_store();
        store
            .store(make_memory("algorithms", "binary search basics"))
            .unwrap();
        store
            .store(make_memory("algorithms", "merge sort algorithm"))
            .unwrap();
        store
            .store(make_memory("algorithms", "quicksort partitioning"))
            .unwrap();

        assert_eq!(store.count_by_topic("algorithms", None).unwrap(), 3);

        let consolidated = make_memory("algorithms", "comprehensive algorithms knowledge");
        store.consolidate_topic("algorithms", consolidated).unwrap();

        let results = store.get_by_topic("algorithms", None).unwrap();
        assert_eq!(
            results.len(),
            1,
            "Should have exactly one memory after consolidation"
        );
        assert_eq!(results[0].summary, "comprehensive algorithms knowledge");
        assert_eq!(store.count_by_topic("algorithms", None).unwrap(), 1);
    }

    // === Prune tests ===

    #[test]
    fn test_prune_removes_only_below_threshold_medium_low() {
        let store = test_store();

        let mut above_threshold = make_memory("test", "above threshold");
        above_threshold.weight = Weight::new_clamped(0.5);
        store.store(above_threshold).unwrap();

        let mut below_medium = make_memory("test", "below threshold medium");
        below_medium.weight = Weight::new_clamped(0.05);
        below_medium.importance = Importance::Medium;
        store.store(below_medium).unwrap();

        let mut below_low = make_memory("test", "below threshold low");
        below_low.weight = Weight::new_clamped(0.05);
        below_low.importance = Importance::Low;
        store.store(below_low).unwrap();

        let pruned = store.prune(0.1).unwrap();
        assert_eq!(pruned, 2, "Should prune exactly 2 memories below threshold");
        assert_eq!(
            store.count(None).unwrap(),
            1,
            "Should have 1 memory remaining"
        );
    }

    #[test]
    fn test_prune_does_not_remove_critical_or_high() {
        let store = test_store();

        let mut critical = make_memory("test", "critical below threshold");
        critical.weight = Weight::new_clamped(0.01);
        critical.importance = Importance::Critical;
        store.store(critical).unwrap();

        let mut high = make_memory("test", "high below threshold");
        high.weight = Weight::new_clamped(0.01);
        high.importance = Importance::High;
        store.store(high).unwrap();

        // Also add a low-importance one that should be pruned
        let mut low = make_memory("test", "low below threshold");
        low.weight = Weight::new_clamped(0.01);
        low.importance = Importance::Low;
        store.store(low).unwrap();

        let pruned = store.prune(0.1).unwrap();
        assert_eq!(pruned, 1, "Should only prune the Low importance memory");

        let remaining = store.count(None).unwrap();
        assert_eq!(remaining, 2, "Critical and High memories should remain");
    }

    // === ChunkStore tests ===

    #[test]
    fn test_chunk_store_document() {
        use chunk_store::test_helpers::make_document;
        use hyphae_core::ChunkStore;

        let store = test_store();
        let doc = make_document("docs/readme.md");
        let doc_id = doc.id.clone();

        let returned_id = store.store_document(doc).unwrap();
        assert_eq!(returned_id, doc_id);

        let fetched = store.get_document(&doc_id).unwrap();
        assert!(fetched.is_some());
        assert_eq!(fetched.unwrap().source_path, "docs/readme.md");
    }

    #[test]
    fn test_chunk_store_chunks() {
        use chunk_store::test_helpers::{make_chunk, make_document};
        use hyphae_core::ChunkStore;

        let store = test_store();
        let doc = make_document("src/lib.rs");
        let doc_id = doc.id.clone();
        store.store_document(doc).unwrap();

        let chunks = vec![
            make_chunk(&doc_id, 0, "first chunk content"),
            make_chunk(&doc_id, 1, "second chunk content"),
        ];
        let stored = store.store_chunks(chunks).unwrap();
        assert_eq!(stored, 2);

        let fetched = store.get_chunks(&doc_id).unwrap();
        assert_eq!(fetched.len(), 2);
        assert_eq!(fetched[0].chunk_index, 0);
        assert_eq!(fetched[1].chunk_index, 1);
    }

    #[test]
    fn test_chunk_store_fts_search() {
        use chunk_store::test_helpers::{make_chunk, make_document};
        use hyphae_core::ChunkStore;

        let store = test_store();
        let doc = make_document("src/main.rs");
        let doc_id = doc.id.clone();
        store.store_document(doc).unwrap();

        let chunks = vec![
            make_chunk(&doc_id, 0, "Rust ownership and borrowing rules"),
            make_chunk(&doc_id, 1, "Python decorators are useful"),
        ];
        store.store_chunks(chunks).unwrap();

        let results = store
            .search_chunks_fts("ownership borrowing", 10, 0, None)
            .unwrap();
        assert!(!results.is_empty());
        assert!(results[0].chunk.content.contains("ownership"));
    }

    #[test]
    fn test_chunk_store_delete_cascades() {
        use chunk_store::test_helpers::{make_chunk, make_document};
        use hyphae_core::ChunkStore;

        let store = test_store();
        let doc = make_document("to_delete.txt");
        let doc_id = doc.id.clone();
        store.store_document(doc).unwrap();

        store
            .store_chunks(vec![make_chunk(&doc_id, 0, "some content")])
            .unwrap();
        assert_eq!(store.count_chunks(None).unwrap(), 1);

        store.delete_document(&doc_id).unwrap();

        assert!(store.get_document(&doc_id).unwrap().is_none());
        assert_eq!(store.count_chunks(None).unwrap(), 0);
    }

    #[test]
    fn test_chunk_store_list_documents() {
        use chunk_store::test_helpers::make_document;
        use hyphae_core::ChunkStore;

        let store = test_store();
        assert_eq!(store.list_documents(None).unwrap().len(), 0);

        store.store_document(make_document("a.txt")).unwrap();
        store.store_document(make_document("b.txt")).unwrap();

        let docs = store.list_documents(None).unwrap();
        assert_eq!(docs.len(), 2);
    }

    #[test]
    fn test_chunk_store_count() {
        use chunk_store::test_helpers::{make_chunk, make_document};
        use hyphae_core::ChunkStore;

        let store = test_store();
        assert_eq!(store.count_documents(None).unwrap(), 0);
        assert_eq!(store.count_chunks(None).unwrap(), 0);

        let doc = make_document("counts.txt");
        let doc_id = doc.id.clone();
        store.store_document(doc).unwrap();
        store
            .store_chunks(vec![
                make_chunk(&doc_id, 0, "chunk one"),
                make_chunk(&doc_id, 1, "chunk two"),
                make_chunk(&doc_id, 2, "chunk three"),
            ])
            .unwrap();

        assert_eq!(store.count_documents(None).unwrap(), 1);
        assert_eq!(store.count_chunks(None).unwrap(), 3);
    }

    #[test]
    fn test_chunk_store_get_by_path() {
        use chunk_store::test_helpers::make_document;
        use hyphae_core::ChunkStore;

        let store = test_store();
        let doc = make_document("unique/path/file.txt");
        let doc_id = doc.id.clone();
        store.store_document(doc).unwrap();

        let found = store
            .get_document_by_path("unique/path/file.txt", None)
            .unwrap();
        assert!(found.is_some());
        assert_eq!(found.unwrap().id, doc_id);

        let not_found = store.get_document_by_path("nonexistent.txt", None).unwrap();
        assert!(not_found.is_none());
    }

    // === FTS sanitization tests ===

    #[test]
    fn test_fts_special_chars_no_panic() {
        let store = test_store();
        store
            .store(make_memory("test", "testing special characters"))
            .unwrap();

        let special_queries = vec![
            "sqlite-vec",
            "hello*world",
            "test\"query",
            "col:value",
            "(grouped)",
            "a + b",
            "~prefix",
            "hat^trick",
            "back\\slash",
            "---",
            "***",
            "",
            "   ",
        ];

        for q in special_queries {
            let _ = store.search_fts(q, 10, 0, None);
        }
    }

    // === ChunkStore tests ===

    use super::chunk_store::test_helpers::{make_chunk, make_document};
    use hyphae_core::ChunkStore;

    #[test]
    fn test_chunk_store_and_retrieve_document() {
        let store = test_store();
        let doc = make_document("src/main.rs");
        let id = doc.id.clone();
        store.store_document(doc).unwrap();

        let retrieved = store.get_document(&id).unwrap();
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().source_path, "src/main.rs");
    }

    #[test]
    fn test_chunk_store_and_retrieve_chunks() {
        let store = test_store();
        let doc = make_document("src/lib.rs");
        let doc_id = doc.id.clone();
        store.store_document(doc).unwrap();

        let chunks = vec![
            make_chunk(&doc_id, 0, "fn main() { println!(\"hello\"); }"),
            make_chunk(&doc_id, 1, "fn helper() { return 42; }"),
        ];
        let count = store.store_chunks(chunks).unwrap();
        assert_eq!(count, 2);

        let retrieved = store.get_chunks(&doc_id).unwrap();
        assert_eq!(retrieved.len(), 2);
        assert_eq!(retrieved[0].chunk_index, 0);
        assert_eq!(retrieved[1].chunk_index, 1);
    }

    #[test]
    fn test_chunk_delete_document_cascades() {
        let store = test_store();
        let doc = make_document("test/cascade.rs");
        let doc_id = doc.id.clone();
        store.store_document(doc).unwrap();

        let chunks = vec![
            make_chunk(&doc_id, 0, "chunk one content"),
            make_chunk(&doc_id, 1, "chunk two content"),
        ];
        store.store_chunks(chunks).unwrap();
        assert_eq!(store.count_chunks(None).unwrap(), 2);

        store.delete_document(&doc_id).unwrap();
        assert_eq!(store.count_chunks(None).unwrap(), 0);
        assert_eq!(store.count_documents(None).unwrap(), 0);
    }

    #[test]
    fn test_chunk_search_fts() {
        let store = test_store();
        let doc = make_document("docs/guide.md");
        let doc_id = doc.id.clone();
        store.store_document(doc).unwrap();

        let chunks = vec![
            make_chunk(&doc_id, 0, "authentication using JWT tokens"),
            make_chunk(&doc_id, 1, "database connection pooling with sqlx"),
            make_chunk(&doc_id, 2, "JWT token expiration and refresh flow"),
        ];
        store.store_chunks(chunks).unwrap();

        let results = store.search_chunks_fts("JWT token", 10, 0, None).unwrap();
        assert!(!results.is_empty(), "FTS should find JWT-related chunks");
        // First result should be most relevant
        assert!(
            results[0].chunk.content.contains("JWT"),
            "Top result should contain JWT"
        );
    }

    #[test]
    fn test_chunk_list_documents() {
        let store = test_store();
        store.store_document(make_document("a.rs")).unwrap();
        store.store_document(make_document("b.rs")).unwrap();
        store.store_document(make_document("c.rs")).unwrap();

        let docs = store.list_documents(None).unwrap();
        assert_eq!(docs.len(), 3);
    }

    #[test]
    fn test_chunk_get_document_by_path() {
        let store = test_store();
        let doc = make_document("unique/path.rs");
        store.store_document(doc).unwrap();

        let found = store.get_document_by_path("unique/path.rs", None).unwrap();
        assert!(found.is_some());

        let not_found = store.get_document_by_path("nonexistent.rs", None).unwrap();
        assert!(not_found.is_none());
    }

    #[test]
    fn test_chunk_count() {
        let store = test_store();
        assert_eq!(store.count_documents(None).unwrap(), 0);
        assert_eq!(store.count_chunks(None).unwrap(), 0);

        let doc = make_document("count.rs");
        let doc_id = doc.id.clone();
        store.store_document(doc).unwrap();

        let chunks = vec![
            make_chunk(&doc_id, 0, "first"),
            make_chunk(&doc_id, 1, "second"),
            make_chunk(&doc_id, 2, "third"),
        ];
        store.store_chunks(chunks).unwrap();

        assert_eq!(store.count_documents(None).unwrap(), 1);
        assert_eq!(store.count_chunks(None).unwrap(), 3);
    }

    #[test]
    fn test_chunk_store_empty_batch() {
        let store = test_store();
        let count = store.store_chunks(vec![]).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_memory_exists_with_keyword_found() {
        let store = test_store();
        let mem = Memory::builder("test".into(), "summary".into(), Importance::Medium)
            .keywords(vec!["hash:abc123def456".into(), "other".into()])
            .build();
        store.store(mem).unwrap();

        assert!(store.memory_exists_with_keyword("abc123def456").unwrap());
    }

    #[test]
    fn test_memory_exists_with_keyword_not_found() {
        let store = test_store();
        assert!(!store.memory_exists_with_keyword("nonexistent").unwrap());
    }

    #[test]
    fn test_memory_exists_with_keyword_partial_no_match() {
        let store = test_store();
        let mem = Memory::builder("test".into(), "summary".into(), Importance::Medium)
            .keywords(vec!["hash:abc123def456".into()])
            .build();
        store.store(mem).unwrap();

        // Different hash should not match
        assert!(!store.memory_exists_with_keyword("xyz789").unwrap());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Cross-project search tests
    // ─────────────────────────────────────────────────────────────────────────

    fn make_project_memory(topic: &str, summary: &str, project: &str) -> Memory {
        Memory::builder(topic.into(), summary.into(), Importance::Medium)
            .project(project.to_string())
            .build()
    }

    #[test]
    fn test_search_all_projects() {
        let store = test_store();
        store
            .store(make_project_memory(
                "rust",
                "Use anyhow for application error handling",
                "alpha",
            ))
            .unwrap();
        store
            .store(make_project_memory(
                "rust",
                "Use thiserror for library error handling",
                "beta",
            ))
            .unwrap();
        store
            .store(make_project_memory(
                "rust",
                "Always derive Debug for error handling types",
                SHARED_PROJECT,
            ))
            .unwrap();

        // FTS matches on "error" token across all three
        let results = store.search_all_projects("error handling", 10).unwrap();
        assert_eq!(results.len(), 3, "should find memories across all projects");
    }

    #[test]
    fn test_search_all_projects_empty_query() {
        let store = test_store();
        store
            .store(make_project_memory("rust", "some content", "alpha"))
            .unwrap();

        let results = store.search_all_projects("", 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_related_projects() {
        let store = test_store();
        store
            .store(make_project_memory(
                "rust",
                "anyhow error handling patterns",
                "alpha",
            ))
            .unwrap();
        store
            .store(make_project_memory(
                "rust",
                "thiserror error handling patterns",
                "beta",
            ))
            .unwrap();
        store
            .store(make_project_memory(
                "rust",
                "error handling formatting tips",
                "gamma",
            ))
            .unwrap();

        let results = store
            .search_related_projects("error handling", &["alpha", "beta"], 10)
            .unwrap();
        assert_eq!(
            results.len(),
            2,
            "should only find memories in alpha and beta"
        );
        for mem in &results {
            let p = mem.project.as_deref().unwrap();
            assert!(p == "alpha" || p == "beta");
        }
    }

    #[test]
    fn test_search_related_projects_empty_list() {
        let store = test_store();
        let results = store.search_related_projects("anything", &[], 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_list_projects() {
        let store = test_store();
        store
            .store(make_project_memory("rust", "memory 1", "alpha"))
            .unwrap();
        store
            .store(make_project_memory("rust", "memory 2", "alpha"))
            .unwrap();
        store
            .store(make_project_memory("rust", "memory 3", "beta"))
            .unwrap();

        let projects = store.list_projects().unwrap();
        assert_eq!(projects.len(), 2);

        let alpha = projects.iter().find(|(name, _)| name == "alpha");
        assert!(alpha.is_some());
        assert_eq!(alpha.unwrap().1, 2);

        let beta = projects.iter().find(|(name, _)| name == "beta");
        assert!(beta.is_some());
        assert_eq!(beta.unwrap().1, 1);
    }

    #[test]
    fn test_link_projects() {
        let store = test_store();
        store.link_projects("alpha", "beta").unwrap();

        let linked_from_alpha = store.get_linked_projects("alpha").unwrap();
        assert_eq!(linked_from_alpha, vec!["beta"]);

        let linked_from_beta = store.get_linked_projects("beta").unwrap();
        assert_eq!(linked_from_beta, vec!["alpha"]);
    }

    #[test]
    fn test_link_projects_idempotent() {
        let store = test_store();
        store.link_projects("alpha", "beta").unwrap();
        store.link_projects("alpha", "beta").unwrap(); // should not error
        let linked = store.get_linked_projects("alpha").unwrap();
        assert_eq!(linked, vec!["beta"]);
    }

    #[test]
    fn test_promote_to_shared() {
        let store = test_store();
        let mem = make_project_memory("rust", "important pattern", "alpha");
        let id = mem.id.clone();
        store.store(mem).unwrap();

        let shared_id = store.promote_to_shared(&id).unwrap();
        let shared = store.get(&shared_id).unwrap().unwrap();
        assert_eq!(shared.project.as_deref(), Some(SHARED_PROJECT));
        assert_eq!(shared.summary, "important pattern");
        assert_eq!(shared.topic, "rust");
    }

    #[test]
    fn test_promote_to_shared_not_found() {
        let store = test_store();
        let result = store.promote_to_shared(&MemoryId::from("nonexistent"));
        assert!(result.is_err());
    }

    #[test]
    fn test_search_all_projects_includes_project_field() {
        let store = test_store();
        store
            .store(make_project_memory(
                "rust",
                "alpha content about rust",
                "alpha",
            ))
            .unwrap();

        let results = store.search_all_projects("rust", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].project.as_deref(), Some("alpha"));
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Purge Operations Tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_purge_by_project() {
        let store = test_store();

        // Create memories in two projects
        let mem1 = make_project_memory("topic1", "summary1", "proj_a");
        let mem2 = make_project_memory("topic1", "summary2", "proj_b");
        let mem3 = make_project_memory("topic1", "summary3", "proj_a");

        store.store(mem1).unwrap();
        store.store(mem2).unwrap();
        store.store(mem3).unwrap();

        assert_eq!(store.count_memories_by_project("proj_a").unwrap(), 2);
        assert_eq!(store.count_memories_by_project("proj_b").unwrap(), 1);

        // Purge proj_a
        let (mem_del, _ses_del, _chk_del, _doc_del) = store.purge_project("proj_a").unwrap();

        assert_eq!(mem_del, 2);
        assert_eq!(store.count_memories_by_project("proj_a").unwrap(), 0);
        assert_eq!(store.count_memories_by_project("proj_b").unwrap(), 1);
    }

    #[test]
    fn test_count_memories_by_project() {
        let store = test_store();

        let mem1 = make_project_memory("topic1", "summary1", "myproject");
        let mem2 = make_project_memory("topic1", "summary2", "myproject");
        let mem3 = make_project_memory("topic2", "summary3", "other");

        store.store(mem1).unwrap();
        store.store(mem2).unwrap();
        store.store(mem3).unwrap();

        assert_eq!(store.count_memories_by_project("myproject").unwrap(), 2);
        assert_eq!(store.count_memories_by_project("other").unwrap(), 1);
        assert_eq!(store.count_memories_by_project("nonexistent").unwrap(), 0);
    }

    #[test]
    fn test_count_memories_before_date() {
        let store = test_store();

        let now = Utc::now();
        let old_date = (now - chrono::Duration::days(10)).to_rfc3339();
        let future_date = (now + chrono::Duration::days(10)).to_rfc3339();

        // Create a memory with an old timestamp (manually since builder creates with now)
        let old_mem = Memory::builder("topic".into(), "summary".into(), Importance::Medium).build();

        // We can't easily set the created_at in Memory builder, so we'll test with current time
        // Just verify the method works
        let mem = Memory::builder("topic".into(), "summary".into(), Importance::Medium).build();
        store.store(mem).unwrap();

        // Should find the memory when checking before future date
        let count = store.count_memories_before_date(&future_date).unwrap();
        assert_eq!(count, 1);

        // Should find 0 when checking before past date
        let count = store.count_memories_before_date(&old_date).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_purge_project_empty() {
        let store = test_store();

        // Purging empty project should work without error
        let (mem_del, ses_del, chk_del, doc_del) = store.purge_project("nonexistent").unwrap();

        assert_eq!(mem_del, 0);
        assert_eq!(ses_del, 0);
        assert_eq!(chk_del, 0);
        assert_eq!(doc_del, 0);
    }

    #[test]
    fn test_count_sessions_by_project() {
        let store = test_store();

        // Create sessions using session_start
        let (_id1, _started1) = store.session_start("proj_a", Some("task1")).unwrap();
        let (_id2, _started2) = store.session_start("proj_a", Some("task2")).unwrap();
        let (_id3, _started3) = store.session_start("proj_b", Some("task3")).unwrap();

        assert_eq!(store.count_sessions_by_project("proj_a").unwrap(), 2);
        assert_eq!(store.count_sessions_by_project("proj_b").unwrap(), 1);
        assert_eq!(store.count_sessions_by_project("nonexistent").unwrap(), 0);
    }

    #[test]
    fn test_count_sessions_before_date() {
        let store = test_store();

        let now = Utc::now();
        let future_date = (now + chrono::Duration::days(10)).to_rfc3339();
        let past_date = (now - chrono::Duration::days(10)).to_rfc3339();

        let (_id, _started) = store.session_start("proj_a", Some("task")).unwrap();

        // Should find session before future date
        let count = store.count_sessions_before_date(&future_date).unwrap();
        assert_eq!(count, 1);

        // Should find 0 sessions before past date
        let count = store.count_sessions_before_date(&past_date).unwrap();
        assert_eq!(count, 0);
    }
}
