use chrono::Utc;
use rusqlite::params;

use hyphae_core::{HyphaeError, HyphaeResult, ReflexionConfidence, ReflexionErrorType, ReflexionRecord, ReflexionStore};

use super::SqliteStore;
use super::search::sanitize_fts_query;

impl ReflexionStore for SqliteStore {
    fn store_reflexion(&self, record: &ReflexionRecord) -> HyphaeResult<String> {
        let error_type = record.error_type.to_string();
        let confidence = record.confidence.to_string();
        let created_at = record.created_at.to_rfc3339();

        self.conn
            .execute(
                "INSERT INTO reflexion_records (id, error_type, root_cause, fix_applied, abstract_pattern, project, confidence, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    record.id,
                    error_type,
                    record.root_cause,
                    record.fix_applied,
                    record.abstract_pattern,
                    record.project,
                    confidence,
                    created_at
                ],
            )
            .map_err(|e| HyphaeError::Database(format!("failed to store reflexion record: {e}")))?;

        Ok(record.id.clone())
    }

    fn search_reflexions(
        &self,
        query: &str,
        error_type: Option<&ReflexionErrorType>,
        limit: usize,
    ) -> HyphaeResult<Vec<ReflexionRecord>> {
        // Check if FTS table exists; fall back to LIKE search if not
        let fts_exists: bool = self
            .conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='reflexion_fts'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(false);

        let query_lower = query.to_lowercase();
        let sanitized = sanitize_fts_query(query);
        let limit = limit.min(100);
        // Stringify once so the Option<String> lives long enough to borrow
        let error_type_str: Option<String> = error_type.map(|et| et.to_string());

        let records = if fts_exists {
            // FTS5 search — use sanitized query to prevent injection via special FTS operators.
            // Push the error_type filter into SQL so the LIMIT is applied to the right result set.
            let sql = if error_type_str.is_some() {
                "SELECT r.id, r.error_type, r.root_cause, r.fix_applied, r.abstract_pattern, r.project, r.confidence, r.created_at
                 FROM reflexion_records r
                 WHERE r.id IN (
                     SELECT id FROM reflexion_fts WHERE reflexion_fts MATCH ?1
                 )
                   AND r.error_type = ?2
                 ORDER BY
                   CASE r.confidence WHEN 'high' THEN 0 WHEN 'medium' THEN 1 ELSE 2 END,
                   r.created_at DESC
                 LIMIT ?3"
            } else {
                "SELECT r.id, r.error_type, r.root_cause, r.fix_applied, r.abstract_pattern, r.project, r.confidence, r.created_at
                 FROM reflexion_records r
                 WHERE r.id IN (
                     SELECT id FROM reflexion_fts WHERE reflexion_fts MATCH ?1
                 )
                 ORDER BY
                   CASE r.confidence WHEN 'high' THEN 0 WHEN 'medium' THEN 1 ELSE 2 END,
                   r.created_at DESC
                 LIMIT ?2"
            };

            let mut stmt = self.conn.prepare(sql).map_err(|e| HyphaeError::Database(e.to_string()))?;

            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> =
                vec![Box::new(sanitized)];
            if let Some(ref et) = error_type_str {
                param_values.push(Box::new(et.clone()));
            }
            param_values.push(Box::new(limit as i32));
            let params_ref: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(|b| b.as_ref()).collect();

            stmt.query_map(params_ref.as_slice(), |row| self.reflexion_from_row(row))
                .map_err(|e| HyphaeError::Database(e.to_string()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| HyphaeError::Database(e.to_string()))?
        } else {
            // Fallback: LIKE search across indexed columns.
            // Push the error_type filter into SQL so the LIMIT is applied to the right result set.
            let sql = if error_type_str.is_some() {
                "SELECT id, error_type, root_cause, fix_applied, abstract_pattern, project, confidence, created_at
                 FROM reflexion_records
                 WHERE (root_cause LIKE ?1 OR fix_applied LIKE ?1 OR abstract_pattern LIKE ?1)
                   AND error_type = ?2
                 ORDER BY
                   CASE confidence WHEN 'high' THEN 0 WHEN 'medium' THEN 1 ELSE 2 END,
                   created_at DESC
                 LIMIT ?3"
            } else {
                "SELECT id, error_type, root_cause, fix_applied, abstract_pattern, project, confidence, created_at
                 FROM reflexion_records
                 WHERE root_cause LIKE ?1 OR fix_applied LIKE ?1 OR abstract_pattern LIKE ?1
                 ORDER BY
                   CASE confidence WHEN 'high' THEN 0 WHEN 'medium' THEN 1 ELSE 2 END,
                   created_at DESC
                 LIMIT ?2"
            };

            let mut stmt = self.conn.prepare(sql).map_err(|e| HyphaeError::Database(e.to_string()))?;

            let like_pattern = format!("%{}%", query_lower);
            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> =
                vec![Box::new(like_pattern)];
            if let Some(ref et) = error_type_str {
                param_values.push(Box::new(et.clone()));
            }
            param_values.push(Box::new(limit as i32));
            let params_ref: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(|b| b.as_ref()).collect();

            stmt.query_map(params_ref.as_slice(), |row| self.reflexion_from_row(row))
                .map_err(|e| HyphaeError::Database(e.to_string()))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| HyphaeError::Database(e.to_string()))?
        };

        Ok(records)
    }

    fn list_reflexions_by_pattern(&self, limit: usize) -> HyphaeResult<Vec<ReflexionRecord>> {
        let limit = limit.min(100);

        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, error_type, root_cause, fix_applied, abstract_pattern, project, confidence, created_at
                 FROM reflexion_records
                 ORDER BY
                   CASE confidence WHEN 'high' THEN 0 WHEN 'medium' THEN 1 ELSE 2 END,
                   created_at DESC
                 LIMIT ?1",
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let records = stmt
            .query_map(params![limit as i32], |row| {
                self.reflexion_from_row(row)
            })
            .map_err(|e| HyphaeError::Database(e.to_string()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        Ok(records)
    }
}

impl SqliteStore {
    fn reflexion_from_row(
        &self,
        row: &rusqlite::Row,
    ) -> rusqlite::Result<ReflexionRecord> {
        let id: String = row.get(0)?;
        let error_type_str: String = row.get(1)?;
        let root_cause: String = row.get(2)?;
        let fix_applied: String = row.get(3)?;
        let abstract_pattern: String = row.get(4)?;
        let project: Option<String> = row.get(5)?;
        let confidence_str: String = row.get(6)?;
        let created_at_str: String = row.get(7)?;

        let error_type = error_type_str
            .parse::<ReflexionErrorType>()
            .unwrap_or(ReflexionErrorType::Other);

        let confidence = confidence_str
            .parse::<ReflexionConfidence>()
            .unwrap_or(ReflexionConfidence::Medium);

        let created_at = created_at_str
            .parse::<chrono::DateTime<chrono::Utc>>()
            .unwrap_or_else(|_| Utc::now());

        Ok(ReflexionRecord {
            id,
            error_type,
            root_cause,
            fix_applied,
            abstract_pattern,
            project,
            confidence,
            created_at,
        })
    }
}
