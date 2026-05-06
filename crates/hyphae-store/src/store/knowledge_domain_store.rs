use chrono::Utc;
use rusqlite::{OptionalExtension, params};
use std::str::FromStr;

use hyphae_core::{
    ApplicabilityRule, Authority, HyphaeError, HyphaeResult, InputSpec, KnowledgeDomain,
};

use super::SqliteStore;

impl SqliteStore {
    pub fn upsert_knowledge_domain(&self, domain: &KnowledgeDomain) -> HyphaeResult<()> {
        let applies_when_json = serde_json::to_string(&domain.applies_when)?;
        let required_inputs_json = serde_json::to_string(&domain.required_inputs)?;

        let authority_str = match domain.authority {
            Authority::Primary => "primary",
            Authority::Derived => "derived",
            Authority::Historical => "historical",
        };

        let now = Utc::now().to_rfc3339();

        self.conn
            .execute(
                "INSERT INTO knowledge_domains
                 (id, description, applies_when, required_inputs, query_template, authority, freshness_ttl_secs, boundary_note, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, COALESCE((SELECT created_at FROM knowledge_domains WHERE id = ?1), ?9), ?10)
                 ON CONFLICT(id) DO UPDATE SET
                   description = ?2,
                   applies_when = ?3,
                   required_inputs = ?4,
                   query_template = ?5,
                   authority = ?6,
                   freshness_ttl_secs = ?7,
                   boundary_note = ?8,
                   updated_at = ?10",
                params![
                    &domain.id,
                    &domain.description,
                    applies_when_json,
                    required_inputs_json,
                    &domain.query_template,
                    authority_str,
                    domain.freshness_ttl_secs.map(|n| n as i64),
                    &domain.boundary_note,
                    &now,
                    &now,
                ],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        Ok(())
    }

    pub fn get_knowledge_domain(&self, id: &str) -> HyphaeResult<Option<KnowledgeDomain>> {
        let mut stmt = self
            .conn
            .prepare_cached(
                "SELECT id, description, applies_when, required_inputs, query_template, authority, freshness_ttl_secs, boundary_note
                 FROM knowledge_domains WHERE id = ?1"
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let result = stmt
            .query_row([id], |row| {
                let applies_when_json: String = row.get(2)?;
                let required_inputs_json: String = row.get(3)?;
                let authority_str: String = row.get(5)?;
                let freshness_ttl_secs: Option<i64> = row.get(6)?;

                let applies_when: Vec<ApplicabilityRule> =
                    serde_json::from_str(&applies_when_json).unwrap_or_default();
                let required_inputs: Vec<InputSpec> =
                    serde_json::from_str(&required_inputs_json).unwrap_or_default();
                let authority = Authority::from_str(&authority_str).unwrap_or_default();

                Ok(KnowledgeDomain {
                    id: row.get(0)?,
                    description: row.get(1)?,
                    applies_when,
                    required_inputs,
                    query_template: row.get(4)?,
                    authority,
                    freshness_ttl_secs: freshness_ttl_secs.map(|n| n as u64),
                    boundary_note: row.get(7)?,
                })
            })
            .optional()
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        Ok(result)
    }

    pub fn list_knowledge_domains(&self) -> HyphaeResult<Vec<KnowledgeDomain>> {
        let mut stmt = self
            .conn
            .prepare_cached(
                "SELECT id, description, applies_when, required_inputs, query_template, authority, freshness_ttl_secs, boundary_note
                 FROM knowledge_domains ORDER BY updated_at DESC"
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                let applies_when_json: String = row.get(2)?;
                let required_inputs_json: String = row.get(3)?;
                let authority_str: String = row.get(5)?;
                let freshness_ttl_secs: Option<i64> = row.get(6)?;

                let applies_when: Vec<ApplicabilityRule> =
                    serde_json::from_str(&applies_when_json).unwrap_or_default();
                let required_inputs: Vec<InputSpec> =
                    serde_json::from_str(&required_inputs_json).unwrap_or_default();
                let authority = Authority::from_str(&authority_str).unwrap_or_default();

                Ok(KnowledgeDomain {
                    id: row.get(0)?,
                    description: row.get(1)?,
                    applies_when,
                    required_inputs,
                    query_template: row.get(4)?,
                    authority,
                    freshness_ttl_secs: freshness_ttl_secs.map(|n| n as u64),
                    boundary_note: row.get(7)?,
                })
            })
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        let mut domains = Vec::new();
        for row in rows {
            domains.push(row.map_err(|e| HyphaeError::Database(e.to_string()))?);
        }
        Ok(domains)
    }

    pub fn delete_knowledge_domain(&self, id: &str) -> HyphaeResult<()> {
        self.conn
            .execute("DELETE FROM knowledge_domains WHERE id = ?1", [id])
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        Ok(())
    }
}
