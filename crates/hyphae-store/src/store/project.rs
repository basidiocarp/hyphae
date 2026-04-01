// ─────────────────────────────────────────────────────────────────────────────
// Cross-project search and project management
// ─────────────────────────────────────────────────────────────────────────────

use chrono::Utc;
use rusqlite::params;

use hyphae_core::{HyphaeError, HyphaeResult, Memory, MemoryId, MemoryStore};

use super::SqliteStore;
use super::helpers;
use super::search;

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
             AND m.invalidated_at IS NULL
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
             AND m.invalidated_at IS NULL
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
                "SELECT COALESCE(project, '(none)'), COUNT(*)
                 FROM memories
                 WHERE invalidated_at IS NULL
                 GROUP BY project
                 ORDER BY project",
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1).map(|n| n as usize)?))
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
    pub fn promote_to_shared(&self, id: &MemoryId) -> HyphaeResult<MemoryId> {
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
}
