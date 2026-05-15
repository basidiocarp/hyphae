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

        // Shared copy is a clean projection: raw_excerpt, source attribution,
        // branch/worktree, agent_id, and expires_at are intentionally dropped.
        // Provenance is recorded via related_ids (origin id) and a keyword tag.
        let mut keywords = original.keywords.clone();
        keywords.push(format!(
            "promoted_from:{}",
            original.project.as_deref().unwrap_or("unknown")
        ));

        let shared = Memory::builder(
            original.topic.clone(),
            original.summary.clone(),
            original.importance,
        )
        .keywords(keywords)
        .project(SHARED_PROJECT.to_string())
        // Link back to the origin so the shared copy can be traced to its source.
        .related_ids(vec![original.id.clone()])
        .build();

        let new_id = self.store(shared)?;
        Ok(new_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyphae_core::{Importance, Memory, MemoryStore};

    fn make_store() -> SqliteStore {
        SqliteStore::in_memory().unwrap()
    }

    fn make_memory(topic: &str, summary: &str, project: Option<&str>) -> Memory {
        let mut builder = Memory::builder(topic.into(), summary.into(), Importance::High);
        if let Some(p) = project {
            builder = builder.project(p.to_string());
        }
        builder.build()
    }

    #[test]
    fn recall_global_without_links_isolates_projects() {
        let store = make_store();

        // Store a memory in project "A"
        let mem_a = make_memory("topic_a", "memory from project A", Some("project_a"));
        store.store(mem_a).unwrap();

        // Store a memory in project "B"
        let mem_b = make_memory("topic_b", "memory from project B", Some("project_b"));
        store.store(mem_b).unwrap();

        // When searching from project B with no links, should NOT return project A's memory
        let results = store
            .search_related_projects("memory", &["_shared", "project_b"], 10)
            .unwrap();

        // Should only have the B memory, not the A memory
        let projects: Vec<&Option<String>> = results.iter().map(|m| &m.project).collect();
        assert!(projects.contains(&&Some("project_b".to_string())));
        assert!(!projects.contains(&&Some("project_a".to_string())));
    }

    #[test]
    fn recall_global_respects_linked_projects() {
        let store = make_store();

        // Store memories in three projects
        let mem_a = make_memory("topic_a", "memory from A", Some("project_a"));
        let mem_b = make_memory("topic_b", "memory from B", Some("project_b"));
        let mem_c = make_memory("topic_c", "memory from C", Some("project_c"));

        store.store(mem_a).unwrap();
        store.store(mem_b).unwrap();
        store.store(mem_c).unwrap();

        // Link project_b to project_c
        store.link_projects("project_b", "project_c").unwrap();

        // When searching from B, should find B and C (via link) but not A
        let results = store
            .search_related_projects("memory", &["_shared", "project_b", "project_c"], 10)
            .unwrap();

        let projects: Vec<&Option<String>> = results.iter().map(|m| &m.project).collect();
        assert!(projects.contains(&&Some("project_b".to_string())));
        assert!(projects.contains(&&Some("project_c".to_string())));
        assert!(!projects.contains(&&Some("project_a".to_string())));
    }

    #[test]
    fn get_linked_projects_returns_empty_when_no_links() {
        let store = make_store();
        let linked = store.get_linked_projects("project_a").unwrap();
        assert!(linked.is_empty());
    }

    #[test]
    fn get_linked_projects_returns_linked_targets() {
        let store = make_store();
        store.link_projects("project_a", "project_b").unwrap();
        store.link_projects("project_a", "project_c").unwrap();

        let linked = store.get_linked_projects("project_a").unwrap();
        assert_eq!(linked.len(), 2);
        assert!(linked.contains(&"project_b".to_string()));
        assert!(linked.contains(&"project_c".to_string()));
    }

    #[test]
    fn link_projects_is_bidirectional() {
        let store = make_store();
        store.link_projects("project_a", "project_b").unwrap();

        let from_a = store.get_linked_projects("project_a").unwrap();
        let from_b = store.get_linked_projects("project_b").unwrap();

        assert!(from_a.contains(&"project_b".to_string()));
        assert!(from_b.contains(&"project_a".to_string()));
    }

    #[test]
    fn promote_to_shared_records_origin_link_and_keyword() {
        let store = make_store();
        let mem = make_memory(
            "topic/provenance",
            "original content",
            Some("project_alpha"),
        );
        let origin_id = store.store(mem).unwrap();

        let shared_id = store.promote_to_shared(&origin_id).unwrap();

        let shared = store
            .get(&shared_id)
            .unwrap()
            .expect("shared memory should exist");

        // related_ids should point back to the origin
        assert!(
            shared.related_ids.contains(&origin_id),
            "promoted shared copy should have origin id in related_ids"
        );

        // keywords should include provenance tag
        assert!(
            shared
                .keywords
                .iter()
                .any(|k| k == "promoted_from:project_alpha"),
            "promoted shared copy should have promoted_from keyword, got: {:?}",
            shared.keywords
        );

        // project should be _shared
        assert_eq!(shared.project.as_deref(), Some("_shared"));
    }
}
