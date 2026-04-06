//! Feedback loop persistence for recall events and session outcomes.

use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};
use rusqlite::{OptionalExtension, params};

use hyphae_core::{HyphaeError, HyphaeResult};

use super::SqliteStore;

const MAX_EFFECTIVENESS_WINDOW_MINUTES: i64 = 60;
const MIN_SIGNALS_FOR_EFFECTIVENESS: usize = 2;
const POSITION_DISCOUNT: f32 = 0.3;
const RECENCY_HALF_LIFE_DAYS: f64 = 14.0;
const SIGNAL_SESSION_SUCCESS: &str = "session_success";
const SIGNAL_SESSION_FAILURE: &str = "session_failure";
const SIGNAL_BUILD_PASSED: &str = "build_passed";
const SIGNAL_TEST_PASSED: &str = "test_passed";
const SIGNAL_CORRECTION: &str = "correction";
const SIGNAL_ERROR_RESOLVED: &str = "error_resolved";
const SIGNAL_ERROR_FREE_RUN: &str = "error_free_run";
const SIGNAL_TOOL_ERROR: &str = "tool_error";
const SIGNAL_EXPLICIT_BOOST: &str = "explicit_boost";

#[derive(Debug, Clone)]
pub(crate) struct RecallEventRecord {
    pub(crate) id: String,
    pub(crate) recalled_at: String,
    pub(crate) memory_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct OutcomeSignalRecord {
    pub(crate) signal_type: String,
    pub(crate) occurred_at: String,
    pub(crate) project: Option<String>,
}

fn parse_rfc3339_utc(value: &str, field: &str) -> HyphaeResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| HyphaeError::Database(format!("invalid {field} timestamp '{value}': {e}")))
}

fn recency_weight(computed_at: &str) -> f32 {
    let Ok(computed_at) = parse_rfc3339_utc(computed_at, "computed_at") else {
        return 1.0;
    };
    let age_days = (Utc::now() - computed_at).num_seconds() as f64 / 86_400.0;
    (-0.693_f64 * age_days / RECENCY_HALF_LIFE_DAYS).exp() as f32
}

fn normalize_signal_type(signal_type: &str) -> Option<&'static str> {
    match signal_type {
        SIGNAL_SESSION_SUCCESS => Some(SIGNAL_SESSION_SUCCESS),
        SIGNAL_SESSION_FAILURE => Some(SIGNAL_SESSION_FAILURE),
        SIGNAL_BUILD_PASSED => Some(SIGNAL_BUILD_PASSED),
        SIGNAL_TEST_PASSED | "test_pass" => Some(SIGNAL_TEST_PASSED),
        SIGNAL_CORRECTION => Some(SIGNAL_CORRECTION),
        SIGNAL_ERROR_RESOLVED => Some(SIGNAL_ERROR_RESOLVED),
        SIGNAL_ERROR_FREE_RUN => Some(SIGNAL_ERROR_FREE_RUN),
        SIGNAL_TOOL_ERROR => Some(SIGNAL_TOOL_ERROR),
        SIGNAL_EXPLICIT_BOOST => Some(SIGNAL_EXPLICIT_BOOST),
        _ => None,
    }
}

fn signal_contribution(signal_type: &str, signal_value: i64) -> Option<i64> {
    if signal_value == 0 {
        return None;
    }

    normalize_signal_type(signal_type).map(|_| signal_value)
}

fn aggregate_effectiveness(rows: &[(f32, String)]) -> f32 {
    let mut weighted_sum = 0.0_f32;

    for (effectiveness, computed_at) in rows {
        let weight = recency_weight(computed_at);
        weighted_sum += *effectiveness * weight;
    }

    weighted_sum.clamp(-1.0, 1.0)
}

impl SqliteStore {
    fn latest_recall_event_id_for_session(
        &self,
        session_id: &str,
        occurred_at: &str,
    ) -> HyphaeResult<Option<String>> {
        let occurred_at = parse_rfc3339_utc(occurred_at, "occurred_at")?;
        let session_ended_at: Option<String> = self
            .conn
            .query_row(
                "SELECT ended_at FROM sessions WHERE id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| {
                HyphaeError::Database(format!("failed to query session end for attribution: {e}"))
            })?
            .flatten();

        if let Some(session_ended_at) = session_ended_at {
            let session_ended_at = parse_rfc3339_utc(&session_ended_at, "ended_at")?;
            if occurred_at > session_ended_at {
                return Ok(None);
            }
        }

        let window_start =
            (occurred_at - Duration::minutes(MAX_EFFECTIVENESS_WINDOW_MINUTES)).to_rfc3339();

        self.conn
            .query_row(
                "SELECT id
                 FROM recall_events
                 WHERE session_id = ?1
                   AND recalled_at <= ?2
                   AND recalled_at >= ?3
                 ORDER BY recalled_at DESC
                 LIMIT 1",
                params![session_id, occurred_at.to_rfc3339(), window_start],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| HyphaeError::Database(format!("failed to query latest recall event: {e}")))
    }

    fn resolve_feedback_project(
        &self,
        session_id: Option<&str>,
        project: Option<&str>,
    ) -> HyphaeResult<Option<String>> {
        let Some(session_id) = session_id else {
            return Ok(project.map(ToOwned::to_owned));
        };

        let session_project: String = self
            .conn
            .query_row(
                "SELECT project FROM sessions WHERE id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| HyphaeError::Database(format!("failed to validate session: {e}")))?
            .ok_or_else(|| HyphaeError::NotFound(format!("session '{session_id}'")))?;

        if let Some(project_name) = project
            && project_name != session_project
        {
            return Err(HyphaeError::Validation(format!(
                "session '{session_id}' belongs to project '{session_project}', not '{project_name}'"
            )));
        }

        Ok(Some(session_project))
    }

    pub fn active_session_id(&self, project: &str) -> HyphaeResult<Option<String>> {
        self.conn
            .query_row(
                "SELECT id
                 FROM sessions
                 WHERE project = ?1 AND status = 'active'
                 ORDER BY started_at DESC
                 LIMIT 1",
                params![project],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| HyphaeError::Database(format!("failed to query active session: {e}")))
    }

    pub fn feedback_session_project(
        &self,
        session_id: &str,
        project: Option<&str>,
    ) -> HyphaeResult<String> {
        self.resolve_feedback_project(Some(session_id), project)?
            .ok_or_else(|| HyphaeError::NotFound(format!("session '{session_id}'")))
    }

    pub fn log_recall_event(
        &self,
        session_id: Option<&str>,
        query: &str,
        memory_ids: &[String],
        project: Option<&str>,
    ) -> HyphaeResult<String> {
        let event_id = format!("rec_{}", ulid::Ulid::new());
        let recalled_at = Utc::now().to_rfc3339();
        let resolved_project = self.resolve_feedback_project(session_id, project)?;
        let memory_ids_json = serde_json::to_string(memory_ids)
            .map_err(|e| HyphaeError::Database(format!("failed to encode recall event: {e}")))?;

        self.conn
            .execute(
                "INSERT INTO recall_events
                    (id, session_id, query, recalled_at, memory_ids, memory_count, project)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    event_id,
                    session_id,
                    query,
                    recalled_at,
                    memory_ids_json,
                    i64::try_from(memory_ids.len()).unwrap_or(i64::MAX),
                    resolved_project,
                ],
            )
            .map_err(|e| HyphaeError::Database(format!("failed to log recall event: {e}")))?;

        Ok(event_id)
    }

    pub fn log_outcome_signal(
        &self,
        session_id: Option<&str>,
        signal_type: &str,
        signal_value: i64,
        source: Option<&str>,
        project: Option<&str>,
    ) -> HyphaeResult<String> {
        let signal_id = format!("sig_{}", ulid::Ulid::new());
        let resolved_project = self.resolve_feedback_project(session_id, project)?;
        let stored_signal_type = normalize_signal_type(signal_type).unwrap_or(signal_type);
        let occurred_at = if let Some(session_id) = session_id {
            if matches!(
                stored_signal_type,
                SIGNAL_SESSION_SUCCESS | SIGNAL_SESSION_FAILURE
            ) {
                self.conn
                    .query_row(
                        "SELECT ended_at
                         FROM sessions
                         WHERE id = ?1 AND status = 'completed'",
                        params![session_id],
                        |row| row.get::<_, Option<String>>(0),
                    )
                    .optional()
                    .map_err(|e| {
                        HyphaeError::Database(format!("failed to query session end time: {e}"))
                    })?
                    .flatten()
                    .unwrap_or_else(|| Utc::now().to_rfc3339())
            } else {
                Utc::now().to_rfc3339()
            }
        } else {
            Utc::now().to_rfc3339()
        };
        let recall_event_id = if let Some(session_id) = session_id {
            self.latest_recall_event_id_for_session(session_id, &occurred_at)?
        } else {
            None
        };

        self.conn
            .execute(
                "INSERT INTO outcome_signals
                    (id, session_id, recall_event_id, signal_type, signal_value, occurred_at, source, project)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    signal_id,
                    session_id,
                    recall_event_id,
                    stored_signal_type,
                    signal_value,
                    occurred_at,
                    source,
                    resolved_project,
                ],
            )
            .map_err(|e| HyphaeError::Database(format!("failed to log outcome signal: {e}")))?;

        if let Some(session_id) = session_id
            && !matches!(
                stored_signal_type,
                SIGNAL_SESSION_SUCCESS | SIGNAL_SESSION_FAILURE
            )
        {
            let session_status: Option<String> = self
                .conn
                .query_row(
                    "SELECT status FROM sessions WHERE id = ?1",
                    params![session_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| {
                    HyphaeError::Database(format!("failed to check session status: {e}"))
                })?;

            if matches!(session_status.as_deref(), Some("completed")) {
                if let Err(e) = self.score_recall_effectiveness(session_id) {
                    tracing::warn!("score_recall_effectiveness failed: {e}");
                }
            }
        }

        Ok(signal_id)
    }

    pub fn count_outcome_signals(
        &self,
        session_id: Option<&str>,
        signal_type: Option<&str>,
        signal_value: Option<i64>,
    ) -> HyphaeResult<i64> {
        let signal_type = signal_type.map(|value| normalize_signal_type(value).unwrap_or(value));
        let mut stmt = self
            .conn
            .prepare(
                "SELECT COUNT(*) FROM outcome_signals
                 WHERE (?1 IS NULL OR session_id = ?1)
                   AND (?2 IS NULL OR signal_type = ?2)
                   AND (?3 IS NULL OR signal_value = ?3)",
            )
            .map_err(|e| {
                HyphaeError::Database(format!("failed to prepare outcome signal count: {e}"))
            })?;

        stmt.query_row(params![session_id, signal_type, signal_value], |row| {
            row.get(0)
        })
        .map_err(|e| HyphaeError::Database(format!("failed to count outcome signals: {e}")))
    }

    pub fn count_recall_events(
        &self,
        session_id: Option<&str>,
        project: Option<&str>,
        memory_count: Option<i64>,
    ) -> HyphaeResult<i64> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT COUNT(*) FROM recall_events
                 WHERE (?1 IS NULL OR session_id = ?1)
                   AND (?2 IS NULL OR project = ?2)
                   AND (?3 IS NULL OR memory_count = ?3)",
            )
            .map_err(|e| {
                HyphaeError::Database(format!("failed to prepare recall event count: {e}"))
            })?;

        stmt.query_row(params![session_id, project, memory_count], |row| row.get(0))
            .map_err(|e| HyphaeError::Database(format!("failed to count recall events: {e}")))
    }

    pub fn count_outcome_signals_in_window(
        &self,
        project: Option<&str>,
        signal_type: &str,
        occurred_after: &str,
        occurred_before: &str,
    ) -> HyphaeResult<i64> {
        let signal_type = normalize_signal_type(signal_type).unwrap_or(signal_type);
        let mut stmt = self
            .conn
            .prepare(
                "SELECT COUNT(*) FROM outcome_signals
                 WHERE (?1 IS NULL OR project = ?1)
                   AND signal_type = ?2
                   AND occurred_at >= ?3
                   AND occurred_at <= ?4",
            )
            .map_err(|e| {
                HyphaeError::Database(format!(
                    "failed to prepare windowed outcome signal count: {e}"
                ))
            })?;

        stmt.query_row(
            params![project, signal_type, occurred_after, occurred_before],
            |row| row.get(0),
        )
        .map_err(|e| {
            HyphaeError::Database(format!("failed to count windowed outcome signals: {e}"))
        })
    }

    pub fn count_recall_events_in_window(
        &self,
        project: Option<&str>,
        recalled_after: &str,
        recalled_before: &str,
    ) -> HyphaeResult<i64> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT COUNT(*) FROM recall_events
                 WHERE (?1 IS NULL OR project = ?1)
                   AND recalled_at >= ?2
                   AND recalled_at <= ?3",
            )
            .map_err(|e| {
                HyphaeError::Database(format!(
                    "failed to prepare windowed recall event count: {e}"
                ))
            })?;

        stmt.query_row(params![project, recalled_after, recalled_before], |row| {
            row.get(0)
        })
        .map_err(|e| HyphaeError::Database(format!("failed to count windowed recall events: {e}")))
    }

    pub(crate) fn outcome_signals_in_window(
        &self,
        project: Option<&str>,
        signal_type: &str,
        occurred_after: &str,
        occurred_before: &str,
    ) -> HyphaeResult<Vec<OutcomeSignalRecord>> {
        let signal_type = normalize_signal_type(signal_type).unwrap_or(signal_type);
        let mut stmt = self
            .conn
            .prepare(
                "SELECT signal_type, occurred_at, project
                 FROM outcome_signals
                 WHERE (?1 IS NULL OR project = ?1)
                   AND signal_type = ?2
                   AND occurred_at >= ?3
                   AND occurred_at <= ?4
                 ORDER BY occurred_at ASC",
            )
            .map_err(|e| {
                HyphaeError::Database(format!(
                    "failed to prepare windowed outcome signal query: {e}"
                ))
            })?;

        let rows = stmt
            .query_map(
                params![project, signal_type, occurred_after, occurred_before],
                |row| {
                    Ok(OutcomeSignalRecord {
                        signal_type: row.get(0)?,
                        occurred_at: row.get(1)?,
                        project: row.get(2)?,
                    })
                },
            )
            .map_err(|e| {
                HyphaeError::Database(format!("failed to query windowed outcome signals: {e}"))
            })?;

        rows.collect::<Result<Vec<_>, _>>().map_err(|e| {
            HyphaeError::Database(format!("failed to collect windowed outcome signals: {e}"))
        })
    }

    pub(crate) fn recall_events_in_window(
        &self,
        project: Option<&str>,
        recalled_after: &str,
        recalled_before: &str,
    ) -> HyphaeResult<Vec<RecallEventRecord>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, recalled_at, memory_ids
                 FROM recall_events
                 WHERE (?1 IS NULL OR project = ?1)
                   AND recalled_at >= ?2
                   AND recalled_at <= ?3
                 ORDER BY recalled_at ASC",
            )
            .map_err(|e| {
                HyphaeError::Database(format!(
                    "failed to prepare windowed recall event query: {e}"
                ))
            })?;

        let rows = stmt
            .query_map(params![project, recalled_after, recalled_before], |row| {
                let memory_ids_json: String = row.get(2)?;
                let memory_ids = serde_json::from_str(&memory_ids_json).unwrap_or_default();
                Ok(RecallEventRecord {
                    id: row.get(0)?,
                    recalled_at: row.get(1)?,
                    memory_ids,
                })
            })
            .map_err(|e| {
                HyphaeError::Database(format!("failed to query windowed recall events: {e}"))
            })?;

        rows.collect::<Result<Vec<_>, _>>().map_err(|e| {
            HyphaeError::Database(format!("failed to collect windowed recall events: {e}"))
        })
    }

    pub(crate) fn score_recall_effectiveness(&self, session_id: &str) -> HyphaeResult<usize> {
        let session_ended_at: Option<String> = self
            .conn
            .query_row(
                "SELECT ended_at
                 FROM sessions
                 WHERE id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| HyphaeError::Database(format!("failed to query session end time: {e}")))?;

        let Some(session_ended_at) = session_ended_at else {
            return Ok(0);
        };

        let session_end = parse_rfc3339_utc(&session_ended_at, "ended_at")?;
        let recalls: Vec<RecallEventRecord> = {
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT id, recalled_at, memory_ids
                     FROM recall_events
                     WHERE session_id = ?1
                     ORDER BY recalled_at ASC",
                )
                .map_err(|e| {
                    HyphaeError::Database(format!("failed to prepare recall lookup: {e}"))
                })?;

            let rows = stmt
                .query_map(params![session_id], |row| {
                    let memory_ids_json: String = row.get(2)?;
                    let memory_ids = serde_json::from_str(&memory_ids_json).map_err(|e| {
                        rusqlite::Error::FromSqlConversionFailure(
                            2,
                            rusqlite::types::Type::Text,
                            Box::new(e),
                        )
                    })?;
                    Ok(RecallEventRecord {
                        id: row.get(0)?,
                        recalled_at: row.get(1)?,
                        memory_ids,
                    })
                })
                .map_err(|e| {
                    HyphaeError::Database(format!("failed to query recall events: {e}"))
                })?;

            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| HyphaeError::Database(format!("failed to read recall events: {e}")))?
        };

        if recalls.is_empty() {
            return Ok(0);
        }

        let tx = self.conn.unchecked_transaction().map_err(|e| {
            HyphaeError::Database(format!("failed to start scoring transaction: {e}"))
        })?;
        let computed_at = Utc::now().to_rfc3339();
        let mut written = 0usize;

        for recall in recalls {
            if recall.memory_ids.is_empty() {
                continue;
            }

            let recalled_at = parse_rfc3339_utc(&recall.recalled_at, "recalled_at")?;
            let window_end = std::cmp::min(
                session_end,
                recalled_at + Duration::minutes(MAX_EFFECTIVENESS_WINDOW_MINUTES),
            );

            if window_end <= recalled_at {
                continue;
            }

            let signal_values: Vec<i64> = {
                let mut stmt = tx
                    .prepare(
                        "SELECT signal_type, signal_value
                         FROM outcome_signals
                         WHERE session_id = ?1
                           AND (
                                recall_event_id = ?4
                                OR (
                                    (
                                        recall_event_id IS NULL
                                        OR signal_type IN (?5, ?6)
                                    )
                                    AND occurred_at >= ?2
                                    AND occurred_at <= ?3
                                )
                           )
                         ORDER BY occurred_at ASC",
                    )
                    .map_err(|e| {
                        HyphaeError::Database(format!("failed to prepare signal lookup: {e}"))
                    })?;

                let rows = stmt
                    .query_map(
                        params![
                            session_id,
                            &recall.recalled_at,
                            window_end.to_rfc3339(),
                            &recall.id,
                            SIGNAL_SESSION_SUCCESS,
                            SIGNAL_SESSION_FAILURE,
                        ],
                        |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
                    )
                    .map_err(|e| {
                        HyphaeError::Database(format!("failed to query outcome signals: {e}"))
                    })?;

                rows.collect::<Result<Vec<_>, _>>()
                    .map_err(|e| {
                        HyphaeError::Database(format!("failed to read outcome signals: {e}"))
                    })?
                    .into_iter()
                    .filter_map(|(signal_type, signal_value)| {
                        signal_contribution(&signal_type, signal_value)
                    })
                    .collect()
            };

            let raw_score = if signal_values.len() < MIN_SIGNALS_FOR_EFFECTIVENESS {
                0.0
            } else {
                let positive_sum: i64 = signal_values
                    .iter()
                    .copied()
                    .filter(|value| *value > 0)
                    .sum();
                let negative_sum: i64 = signal_values
                    .iter()
                    .copied()
                    .filter(|value| *value < 0)
                    .sum();
                let total = positive_sum + negative_sum;
                let magnitude = (positive_sum.abs() + negative_sum.abs()).max(1);
                total as f32 / magnitude as f32
            };

            for (index, memory_id) in recall.memory_ids.iter().enumerate() {
                let position_factor = 1.0 / (1.0 + POSITION_DISCOUNT * index as f32);
                let effectiveness = raw_score * position_factor;

                tx.execute(
                    "INSERT INTO recall_effectiveness
                        (memory_id, recall_event_id, effectiveness, signal_count, computed_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(memory_id, recall_event_id) DO UPDATE SET
                        effectiveness = excluded.effectiveness,
                        signal_count = excluded.signal_count,
                        computed_at = COALESCE(recall_effectiveness.computed_at, excluded.computed_at)",
                    params![
                        memory_id,
                        &recall.id,
                        effectiveness,
                        signal_values.len() as i64,
                        &computed_at,
                    ],
                )
                .map_err(|e| {
                    HyphaeError::Database(format!("failed to upsert recall effectiveness: {e}"))
                })?;
                written += 1;
            }
        }

        tx.commit().map_err(|e| {
            HyphaeError::Database(format!("failed to commit scoring transaction: {e}"))
        })?;
        Ok(written)
    }

    pub(crate) fn recall_effectiveness_for_memory_ids(
        &self,
        memory_ids: &[String],
    ) -> HyphaeResult<HashMap<String, f32>> {
        if memory_ids.is_empty() {
            return Ok(HashMap::new());
        }

        let placeholders: Vec<String> = (1..=memory_ids.len()).map(|i| format!("?{i}")).collect();
        let sql = format!(
            "SELECT memory_id, effectiveness, computed_at
             FROM recall_effectiveness
             WHERE memory_id IN ({})
             ORDER BY computed_at DESC",
            placeholders.join(",")
        );

        let mut stmt = self.conn.prepare_cached(&sql).map_err(|e| {
            HyphaeError::Database(format!("failed to prepare effectiveness query: {e}"))
        })?;

        let param_values: Vec<Box<dyn rusqlite::types::ToSql>> = memory_ids
            .iter()
            .map(|id| Box::new(id.clone()) as Box<dyn rusqlite::types::ToSql>)
            .collect();
        let params_ref: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(|value| value.as_ref()).collect();

        let mut grouped: HashMap<String, Vec<(f32, String)>> = HashMap::new();
        let rows = stmt
            .query_map(params_ref.as_slice(), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, f32>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|e| {
                HyphaeError::Database(format!("failed to query effectiveness rows: {e}"))
            })?;

        for row in rows {
            let (memory_id, effectiveness, computed_at) = row.map_err(|e| {
                HyphaeError::Database(format!("failed to read effectiveness rows: {e}"))
            })?;
            grouped
                .entry(memory_id)
                .or_default()
                .push((effectiveness, computed_at));
        }

        Ok(grouped
            .into_iter()
            .map(|(memory_id, rows)| (memory_id, aggregate_effectiveness(&rows)))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyphae_core::{Importance, Memory, MemoryStore};

    #[derive(Debug, Clone)]
    struct RecallEventRow {
        id: String,
        session_id: Option<String>,
        query: String,
        recalled_at: String,
        memory_ids: Vec<String>,
        project: Option<String>,
    }

    #[derive(Debug, Clone)]
    struct OutcomeSignalRow {
        id: String,
        session_id: Option<String>,
        recall_event_id: Option<String>,
        signal_type: String,
        signal_value: i64,
        occurred_at: String,
        source: Option<String>,
        project: Option<String>,
    }

    fn test_store() -> SqliteStore {
        SqliteStore::in_memory().expect("in-memory store")
    }

    #[test]
    fn test_active_session_id_returns_latest_active_session() {
        let store = test_store();
        let (first, _) = store.session_start("demo", Some("first")).unwrap();
        let (second, _) = store.session_start("demo", Some("second")).unwrap();

        let active = store.active_session_id("demo").unwrap();
        assert_eq!(active.as_deref(), Some(first.as_str()));
        assert_eq!(first, second);
    }

    #[test]
    fn test_log_recall_event_persists_payload() {
        let store = test_store();
        let (session_id, _) = store.session_start("demo", Some("feedback")).unwrap();
        let memory_ids = vec!["mem_1".to_string(), "mem_2".to_string()];

        let event_id = store
            .log_recall_event(
                Some(&session_id),
                "ownership borrow",
                &memory_ids,
                Some("demo"),
            )
            .unwrap();

        let row: RecallEventRow = store
            .conn
            .query_row(
                "SELECT id, session_id, query, recalled_at, memory_ids, project
                 FROM recall_events
                 WHERE id = ?1",
                params![event_id],
                |row| {
                    let ids_json: String = row.get(4)?;
                    let ids = serde_json::from_str(&ids_json).unwrap_or_default();
                    Ok(RecallEventRow {
                        id: row.get(0)?,
                        session_id: row.get(1)?,
                        query: row.get(2)?,
                        recalled_at: row.get(3)?,
                        memory_ids: ids,
                        project: row.get(5)?,
                    })
                },
            )
            .unwrap();

        assert_eq!(row.session_id.as_deref(), Some(session_id.as_str()));
        assert_eq!(row.query, "ownership borrow");
        assert_eq!(row.memory_ids, memory_ids);
        assert_eq!(row.project.as_deref(), Some("demo"));
        assert!(!row.id.is_empty());
        assert!(!row.recalled_at.is_empty());
    }

    #[test]
    fn test_feedback_session_project_rejects_unknown_session() {
        let store = test_store();
        let result = store.feedback_session_project("ses_missing", None);
        assert!(matches!(result, Err(HyphaeError::NotFound(_))));
    }

    #[test]
    fn test_feedback_session_project_rejects_project_mismatch() {
        let store = test_store();
        let (session_id, _) = store.session_start("demo", Some("feedback")).unwrap();

        let result = store.feedback_session_project(&session_id, Some("other-project"));
        assert!(matches!(result, Err(HyphaeError::Validation(_))));
    }

    #[test]
    fn test_log_outcome_signal_persists_payload() {
        let store = test_store();
        let (session_id, _) = store.session_start("demo", Some("feedback")).unwrap();

        let signal_id = store
            .log_outcome_signal(
                Some(&session_id),
                "session_success",
                2,
                Some("hyphae.session_end"),
                Some("demo"),
            )
            .unwrap();

        let row: OutcomeSignalRow = store
            .conn
            .query_row(
                "SELECT id, session_id, recall_event_id, signal_type, signal_value, occurred_at, source, project
                 FROM outcome_signals
                 WHERE id = ?1",
                params![signal_id],
                |row| {
                    Ok(OutcomeSignalRow {
                        id: row.get(0)?,
                        session_id: row.get(1)?,
                        recall_event_id: row.get(2)?,
                        signal_type: row.get(3)?,
                        signal_value: row.get(4)?,
                        occurred_at: row.get(5)?,
                        source: row.get(6)?,
                        project: row.get(7)?,
                    })
                },
            )
            .unwrap();

        assert_eq!(row.session_id.as_deref(), Some(session_id.as_str()));
        assert_eq!(row.recall_event_id, None);
        assert_eq!(row.signal_type, "session_success");
        assert_eq!(row.signal_value, 2);
        assert_eq!(row.source.as_deref(), Some("hyphae.session_end"));
        assert_eq!(row.project.as_deref(), Some("demo"));
        assert!(!row.id.is_empty());
        assert!(!row.occurred_at.is_empty());
    }

    #[test]
    fn test_log_outcome_signal_rejects_unknown_session() {
        let store = test_store();
        let result = store.log_outcome_signal(
            Some("ses_missing"),
            "correction",
            -1,
            Some("cortina.post_tool_use"),
            None,
        );
        assert!(matches!(result, Err(HyphaeError::NotFound(_))));
    }

    #[test]
    fn test_log_outcome_signal_rejects_project_mismatch() {
        let store = test_store();
        let (session_id, _) = store.session_start("demo", Some("feedback")).unwrap();
        let result = store.log_outcome_signal(
            Some(&session_id),
            "correction",
            -1,
            Some("cortina.post_tool_use"),
            Some("other-project"),
        );
        assert!(matches!(result, Err(HyphaeError::Validation(_))));
    }

    #[test]
    fn test_log_outcome_signal_backfills_project_from_session() {
        let store = test_store();
        let (session_id, _) = store.session_start("demo", Some("feedback")).unwrap();

        store
            .log_outcome_signal(
                Some(&session_id),
                "session_success",
                2,
                Some("hyphae.session_end"),
                None,
            )
            .unwrap();

        let count = store
            .count_outcome_signals(Some(&session_id), Some("session_success"), Some(2))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_log_outcome_signal_normalizes_legacy_test_pass_alias() {
        let store = test_store();
        let (session_id, _) = store.session_start("demo", Some("feedback")).unwrap();

        store
            .log_outcome_signal(
                Some(&session_id),
                "test_pass",
                2,
                Some("manual"),
                Some("demo"),
            )
            .unwrap();

        let legacy_count = store
            .count_outcome_signals(Some(&session_id), Some("test_pass"), Some(2))
            .unwrap();
        let canonical_count = store
            .count_outcome_signals(Some(&session_id), Some("test_passed"), Some(2))
            .unwrap();

        assert_eq!(legacy_count, 1);
        assert_eq!(canonical_count, 1);
    }

    #[test]
    fn test_log_outcome_signal_attaches_latest_recall_event_within_session_window() {
        let store = test_store();
        let (session_id, _) = store.session_start("demo", Some("feedback")).unwrap();

        let memory = Memory::new("demo".into(), "recalled memory".into(), Importance::Medium);
        let memory_id = store.store(memory).unwrap();
        let memory_ids = vec![memory_id.to_string()];

        let first_recall = store
            .log_recall_event(Some(&session_id), "first recall", &memory_ids, Some("demo"))
            .unwrap();
        let second_recall = store
            .log_recall_event(
                Some(&session_id),
                "second recall",
                &memory_ids,
                Some("demo"),
            )
            .unwrap();

        let signal_id = store
            .log_outcome_signal(
                Some(&session_id),
                "test_passed",
                1,
                Some("cortina.post_tool_use.test"),
                Some("demo"),
            )
            .unwrap();

        let recall_event_id: Option<String> = store
            .conn
            .query_row(
                "SELECT recall_event_id FROM outcome_signals WHERE id = ?1",
                params![signal_id],
                |row| row.get(0),
            )
            .unwrap();

        assert_eq!(recall_event_id.as_deref(), Some(second_recall.as_str()));
        assert_ne!(recall_event_id.as_deref(), Some(first_recall.as_str()));
    }

    #[test]
    fn test_log_outcome_signal_computes_recall_effectiveness_after_session_end() {
        let store = test_store();
        let (session_id, _) = store.session_start("demo", Some("feedback")).unwrap();

        let first = store
            .store(Memory::new(
                "demo".into(),
                "first recalled memory".into(),
                Importance::Medium,
            ))
            .unwrap();
        let second = store
            .store(Memory::new(
                "demo".into(),
                "second recalled memory".into(),
                Importance::Medium,
            ))
            .unwrap();

        let recall_event_id = store
            .log_recall_event(
                Some(&session_id),
                "recall query",
                &[first.as_ref().to_string(), second.as_ref().to_string()],
                Some("demo"),
            )
            .unwrap();

        store
            .log_outcome_signal(
                Some(&session_id),
                "test_pass",
                2,
                Some("manual"),
                Some("demo"),
            )
            .unwrap();
        store
            .log_outcome_signal(
                Some(&session_id),
                "test_pass",
                2,
                Some("manual"),
                Some("demo"),
            )
            .unwrap();
        store
            .log_outcome_signal(
                Some(&session_id),
                "test_pass",
                2,
                Some("manual"),
                Some("demo"),
            )
            .unwrap();

        let (_project, _, _, _, _) = store
            .session_end(&session_id, Some("done"), None, Some("0"))
            .unwrap();

        let rows_before: Vec<(String, String, f32, i64, String)> = store
            .conn
            .prepare(
                "SELECT memory_id, recall_event_id, effectiveness, signal_count, computed_at
                 FROM recall_effectiveness
                 WHERE recall_event_id = ?1
                 ORDER BY memory_id",
            )
            .unwrap()
            .query_map(params![recall_event_id], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        store
            .log_outcome_signal(
                Some(&session_id),
                "correction",
                -1,
                Some("manual"),
                Some("demo"),
            )
            .unwrap();

        let rows_after: Vec<(String, String, f32, i64, String)> = store
            .conn
            .prepare(
                "SELECT memory_id, recall_event_id, effectiveness, signal_count, computed_at
                 FROM recall_effectiveness
                 WHERE recall_event_id = ?1
                 ORDER BY memory_id",
            )
            .unwrap()
            .query_map(params![recall_event_id], |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(rows_before.len(), 2);
        assert_eq!(rows_before[0].1, recall_event_id);
        assert_eq!(rows_before[0].3, 4);
        assert!(rows_before[0].2 > rows_before[1].2);
        assert_eq!(rows_before, rows_after);
    }

    #[test]
    fn test_compute_session_effectiveness_applies_session_end_signal_to_all_eligible_recalls() {
        let store = test_store();
        let (session_id, _) = store.session_start("demo", Some("feedback")).unwrap();

        let first = store
            .store(Memory::new(
                "demo".into(),
                "first recalled memory".into(),
                Importance::Medium,
            ))
            .unwrap();
        let second = store
            .store(Memory::new(
                "demo".into(),
                "second recalled memory".into(),
                Importance::Medium,
            ))
            .unwrap();

        store
            .log_recall_event(
                Some(&session_id),
                "first recall",
                &[first.as_ref().to_string()],
                Some("demo"),
            )
            .unwrap();
        store
            .log_outcome_signal(
                Some(&session_id),
                "test_passed",
                2,
                Some("manual"),
                Some("demo"),
            )
            .unwrap();
        let second_recall = store
            .log_recall_event(
                Some(&session_id),
                "second recall",
                &[second.as_ref().to_string()],
                Some("demo"),
            )
            .unwrap();

        store
            .log_outcome_signal(
                Some(&session_id),
                "test_passed",
                2,
                Some("manual"),
                Some("demo"),
            )
            .unwrap();
        store
            .log_outcome_signal(
                Some(&session_id),
                "build_passed",
                2,
                Some("manual"),
                Some("demo"),
            )
            .unwrap();
        let (_project, _, _, _, _) = store
            .session_end(&session_id, Some("done"), None, Some("0"))
            .unwrap();

        let rows: Vec<(String, String, f32)> = store
            .conn
            .prepare(
                "SELECT memory_id, recall_event_id, effectiveness
                 FROM recall_effectiveness
                 ORDER BY recall_event_id, memory_id",
            )
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, first.as_ref().to_string());
        assert!(rows[0].2 > 0.0);
        assert_eq!(rows[1].0, second.as_ref().to_string());
        assert_eq!(rows[1].1, second_recall);
        assert!(rows[1].2 > 0.0);
    }

    #[test]
    fn test_compute_session_effectiveness_accepts_two_signals() {
        let store = test_store();
        let (session_id, _) = store.session_start("demo", Some("feedback")).unwrap();

        let memory = store
            .store(Memory::new(
                "demo".into(),
                "recalled memory".into(),
                Importance::Medium,
            ))
            .unwrap();

        let recall_event_id = store
            .log_recall_event(
                Some(&session_id),
                "recall query",
                &[memory.as_ref().to_string()],
                Some("demo"),
            )
            .unwrap();

        store
            .log_outcome_signal(
                Some(&session_id),
                "test_passed",
                2,
                Some("manual"),
                Some("demo"),
            )
            .unwrap();
        let (_project, _, _, _, _) = store
            .session_end(&session_id, Some("done"), None, Some("0"))
            .unwrap();

        let rows: Vec<(String, String, i64)> = store
            .conn
            .prepare(
                "SELECT memory_id, recall_event_id, signal_count
                 FROM recall_effectiveness
                 WHERE recall_event_id = ?1",
            )
            .unwrap()
            .query_map(params![recall_event_id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].2, 2);
    }

    #[test]
    fn test_score_recall_effectiveness_records_negative_scores() {
        let store = test_store();
        let (session_id, _) = store.session_start("demo", Some("feedback")).unwrap();

        let memory = store
            .store(Memory::new(
                "demo".into(),
                "recalled memory".into(),
                Importance::Medium,
            ))
            .unwrap();

        let recall_event_id = store
            .log_recall_event(
                Some(&session_id),
                "recall query",
                &[memory.as_ref().to_string()],
                Some("demo"),
            )
            .unwrap();

        store
            .log_outcome_signal(
                Some(&session_id),
                "correction",
                -1,
                Some("manual"),
                Some("demo"),
            )
            .unwrap();
        let (_project, _, _, _, _) = store
            .session_end(&session_id, Some("done"), None, Some("2"))
            .unwrap();

        let row: (f32, i64) = store
            .conn
            .query_row(
                "SELECT effectiveness, signal_count
                 FROM recall_effectiveness
                 WHERE memory_id = ?1 AND recall_event_id = ?2",
                params![memory.as_ref().to_string(), recall_event_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();

        assert!(row.0 < 0.0);
        assert_eq!(row.1, 2);
    }

    #[test]
    fn test_score_recall_effectiveness_persists_zero_when_below_signal_threshold() {
        let store = test_store();
        let (session_id, _) = store.session_start("demo", Some("feedback")).unwrap();

        let memory = store
            .store(Memory::new(
                "demo".into(),
                "recalled memory".into(),
                Importance::Medium,
            ))
            .unwrap();

        let recall_event_id = store
            .log_recall_event(
                Some(&session_id),
                "recall query",
                &[memory.as_ref().to_string()],
                Some("demo"),
            )
            .unwrap();

        let (_project, _, _, _, _) = store
            .session_end(&session_id, Some("done"), None, Some("0"))
            .unwrap();

        let row: (f32, i64) = store
            .conn
            .query_row(
                "SELECT effectiveness, signal_count
                 FROM recall_effectiveness
                 WHERE memory_id = ?1 AND recall_event_id = ?2",
                params![memory.as_ref().to_string(), recall_event_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();

        assert_eq!(row.0, 0.0);
        assert_eq!(row.1, 1);

        let scores = store
            .recall_effectiveness_for_memory_ids(&[memory.as_ref().to_string()])
            .unwrap();
        assert_eq!(
            scores.get(memory.as_ref()).copied().unwrap_or_default(),
            0.0
        );
    }

    #[test]
    fn test_recall_effectiveness_for_memory_ids_applies_recency_weighting() {
        let store = test_store();
        let older = store
            .store(Memory::new(
                "demo".into(),
                "older recalled memory".into(),
                Importance::Medium,
            ))
            .unwrap();
        let newer = store
            .store(Memory::new(
                "demo".into(),
                "newer recalled memory".into(),
                Importance::Medium,
            ))
            .unwrap();

        let old_timestamp = (Utc::now() - Duration::days(90)).to_rfc3339();
        let new_timestamp = (Utc::now() - Duration::days(7)).to_rfc3339();

        store
            .conn
            .execute(
                "INSERT INTO recall_effectiveness
                    (memory_id, recall_event_id, effectiveness, signal_count, computed_at)
                 VALUES (?1, 'rec_old', 0.8, 3, ?2),
                        (?3, 'rec_new', 0.8, 3, ?4)",
                params![
                    older.as_ref().to_string(),
                    old_timestamp,
                    newer.as_ref().to_string(),
                    new_timestamp
                ],
            )
            .unwrap();

        let scores = store
            .recall_effectiveness_for_memory_ids(&[
                older.as_ref().to_string(),
                newer.as_ref().to_string(),
            ])
            .unwrap();

        let older_score = scores.get(older.as_ref()).copied().unwrap_or_default();
        let newer_score = scores.get(newer.as_ref()).copied().unwrap_or_default();

        assert!(older_score > 0.0);
        assert!(newer_score > 0.0);
        assert!(newer_score > older_score);
    }

    #[test]
    fn test_count_outcome_signals_and_recalls_in_window() {
        let store = test_store();
        let (session_id, _) = store.session_start("demo", Some("feedback")).unwrap();
        let memory = store
            .store(Memory::new(
                "demo".into(),
                "recalled memory".into(),
                Importance::Medium,
            ))
            .unwrap();

        let started_before = Utc::now().to_rfc3339();
        store
            .log_recall_event(
                Some(&session_id),
                "recall query",
                &[memory.as_ref().to_string()],
                Some("demo"),
            )
            .unwrap();
        store
            .log_outcome_signal(
                Some(&session_id),
                "test_passed",
                2,
                Some("manual"),
                Some("demo"),
            )
            .unwrap();
        let started_after = Utc::now().to_rfc3339();

        let recalls = store
            .count_recall_events_in_window(Some("demo"), &started_before, &started_after)
            .unwrap();
        let tests = store
            .count_outcome_signals_in_window(
                Some("demo"),
                "test_pass",
                &started_before,
                &started_after,
            )
            .unwrap();

        assert_eq!(recalls, 1);
        assert_eq!(tests, 1);
    }
}
