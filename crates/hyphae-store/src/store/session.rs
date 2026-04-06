//! Session lifecycle persistence.

use chrono::Utc;
use rusqlite::{OptionalExtension, params};
use serde::Serialize;
use serde_json::Value;
use std::path::Path;

use hyphae_core::{Embedder, HyphaeError, HyphaeResult, Memory, MemoryStore};

use super::SqliteStore;

/// A session record.
#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub project: String,
    pub project_root: Option<String>,
    pub worktree_id: Option<String>,
    pub scope: Option<String>,
    pub runtime_session_id: Option<String>,
    pub task: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub summary: Option<String>,
    pub files_modified: Option<String>,
    pub errors: Option<String>,
    pub status: String,
}

/// A session timeline event in the Cap-compatible read contract.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SessionTimelineEvent {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub detail: Option<String>,
    pub occurred_at: String,
    pub recall_event_id: Option<String>,
    pub memory_count: Option<i64>,
    pub signal_type: Option<String>,
    pub signal_value: Option<i64>,
    pub source: Option<String>,
}

/// A session timeline record in the Cap-compatible read contract.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SessionTimelineRecord {
    pub id: String,
    pub project: String,
    pub project_root: Option<String>,
    pub worktree_id: Option<String>,
    pub scope: Option<String>,
    pub runtime_session_id: Option<String>,
    pub task: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub summary: Option<String>,
    pub files_modified: Option<String>,
    pub errors: Option<String>,
    pub status: String,
    pub events: Vec<SessionTimelineEvent>,
    pub last_activity_at: String,
    pub recall_count: usize,
    pub outcome_count: usize,
}

#[derive(Debug, Clone)]
struct RecallTimelineRow {
    id: String,
    query: String,
    recalled_at: String,
    memory_count: i64,
}

#[derive(Debug, Clone)]
struct OutcomeTimelineRow {
    id: String,
    recall_event_id: Option<String>,
    signal_type: String,
    signal_value: i64,
    occurred_at: String,
    source: Option<String>,
}

impl SqliteStore {
    /// Start a new session. Returns (session_id, started_at).
    pub fn session_start(
        &self,
        project: &str,
        task: Option<&str>,
    ) -> HyphaeResult<(String, String)> {
        self.session_start_scoped(project, task, None)
    }

    /// Start a new session scoped to a specific worker or runtime. Returns (session_id, started_at).
    pub fn session_start_scoped(
        &self,
        project: &str,
        task: Option<&str>,
        scope: Option<&str>,
    ) -> HyphaeResult<(String, String)> {
        self.session_start_scoped_with_runtime(project, task, scope, None)
    }

    /// Start a new session scoped to a specific worker or runtime with external session metadata.
    pub fn session_start_scoped_with_runtime(
        &self,
        project: &str,
        task: Option<&str>,
        scope: Option<&str>,
        runtime_session_id: Option<&str>,
    ) -> HyphaeResult<(String, String)> {
        self.session_start_identity_with_runtime_and_context_signals(
            project,
            task,
            None,
            None,
            scope,
            runtime_session_id,
            None,
            None,
        )
        .map(|(session_id, started_at, _)| (session_id, started_at))
    }

    /// Start a new session with additive identity fields.
    ///
    /// When both `project_root` and `worktree_id` are present they become the
    /// preferred lookup key for active session reuse.
    ///
    /// If `scope` is also present, the active-session lookup stays scoped so
    /// parallel workers on the same worktree remain distinct.
    pub fn session_start_identity(
        &self,
        project: &str,
        task: Option<&str>,
        project_root: Option<&str>,
        worktree_id: Option<&str>,
        scope: Option<&str>,
    ) -> HyphaeResult<(String, String)> {
        self.session_start_identity_with_runtime_and_context_signals(
            project,
            task,
            project_root,
            worktree_id,
            scope,
            None,
            None,
            None,
        )
        .map(|(session_id, started_at, _)| (session_id, started_at))
    }

    /// Start a new session with additive identity fields and external session metadata.
    pub fn session_start_identity_with_runtime(
        &self,
        project: &str,
        task: Option<&str>,
        project_root: Option<&str>,
        worktree_id: Option<&str>,
        scope: Option<&str>,
        runtime_session_id: Option<&str>,
    ) -> HyphaeResult<(String, String)> {
        self.session_start_identity_with_runtime_and_context_signals(
            project,
            task,
            project_root,
            worktree_id,
            scope,
            runtime_session_id,
            None,
            None,
        )
        .map(|(session_id, started_at, _)| (session_id, started_at))
    }

    /// Start a new session with additive identity fields, external session metadata,
    /// and optional context-aware recall signals.
    pub fn session_start_identity_with_runtime_and_context_signals(
        &self,
        project: &str,
        task: Option<&str>,
        project_root: Option<&str>,
        worktree_id: Option<&str>,
        scope: Option<&str>,
        runtime_session_id: Option<&str>,
        context_signals: Option<&Value>,
        embedder: Option<&dyn Embedder>,
    ) -> HyphaeResult<(String, String, Vec<(Memory, f32)>)> {
        let (project_root, worktree_id) = normalized_identity(project_root, worktree_id);
        let recalled_context = self.recalled_context_from_signals(
            project,
            project_root,
            worktree_id,
            context_signals,
            embedder,
        );

        if let Some(existing) =
            self.find_active_session(project, project_root, worktree_id, scope)?
        {
            return Ok((existing.0, existing.1, recalled_context));
        }

        let session_id = format!("ses_{}", ulid::Ulid::new());
        let started_at = Utc::now().to_rfc3339();

        self.conn
            .execute(
                "INSERT INTO sessions (id, project, project_root, worktree_id, scope, runtime_session_id, task, started_at, status)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'active')",
                params![
                    session_id,
                    project,
                    project_root,
                    worktree_id,
                    scope,
                    runtime_session_id,
                    task,
                    started_at
                ],
            )
            .map_err(|e| HyphaeError::Database(format!("failed to insert session: {e}")))?;

        Ok((session_id, started_at, recalled_context))
    }

    /// End an active session. Returns (project, started_at, task, ended_at, duration_minutes).
    pub fn session_end(
        &self,
        session_id: &str,
        summary: Option<&str>,
        files_modified: Option<&str>,
        errors: Option<&str>,
    ) -> HyphaeResult<(String, String, Option<String>, String, i64)> {
        // Fetch the active session
        let row: (String, String, Option<String>) = self
            .conn
            .query_row(
                "SELECT project, started_at, task FROM sessions WHERE id = ?1 AND status = 'active'",
                params![session_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .map_err(|e| match e {
                rusqlite::Error::QueryReturnedNoRows => {
                    HyphaeError::NotFound(format!("no active session with id '{session_id}'"))
                }
                other => HyphaeError::Database(format!("failed to query session: {other}")),
            })?;

        let (project, started_at, task) = row;
        let ended_at = Utc::now().to_rfc3339();

        let duration_minutes = chrono::DateTime::parse_from_rfc3339(&ended_at)
            .ok()
            .zip(chrono::DateTime::parse_from_rfc3339(&started_at).ok())
            .map(|(end, start)| (end - start).num_minutes())
            .unwrap_or(0);

        self.conn
            .execute(
                "UPDATE sessions SET ended_at = ?1, summary = ?2, files_modified = ?3, errors = ?4, status = 'completed' WHERE id = ?5",
                params![ended_at, summary, files_modified, errors, session_id],
            )
            .map_err(|e| HyphaeError::Database(format!("failed to update session: {e}")))?;

        let error_count = errors
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(0);
        let signal_type = if error_count > 0 {
            "session_failure"
        } else {
            "session_success"
        };
        let signal_value = if error_count > 0 { -2 } else { 2 };
        if let Err(e) = self.log_outcome_signal(
            Some(session_id),
            signal_type,
            signal_value,
            Some("hyphae.session_end"),
            Some(&project),
        ) {
            tracing::warn!("failed to record session outcome signal: {e}");
        }
        if let Err(e) = self.score_recall_effectiveness(session_id) {
            tracing::warn!("failed to score recall effectiveness: {e}");
        }

        Ok((project, started_at, task, ended_at, duration_minutes))
    }

    /// Get recent sessions for a project.
    pub fn session_context(&self, project: &str, limit: i64) -> HyphaeResult<Vec<Session>> {
        self.session_context_scoped(project, None, limit)
    }

    /// Get recent sessions across all projects.
    pub fn session_context_all(&self, limit: i64) -> HyphaeResult<Vec<Session>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, project, project_root, worktree_id, scope, task, started_at, ended_at, summary, files_modified, errors, status, runtime_session_id
                 FROM sessions
                 ORDER BY COALESCE(ended_at, started_at) DESC
                 LIMIT ?1",
            )
            .map_err(|e| HyphaeError::Database(format!("failed to prepare query: {e}")))?;

        let rows = stmt
            .query_map(params![limit], load_session)
            .map_err(|e| HyphaeError::Database(format!("failed to query sessions: {e}")))?;

        let sessions: Vec<Session> = rows.filter_map(Result::ok).collect();
        Ok(sessions)
    }

    /// Get recent sessions for a project, optionally filtered to a worker/runtime scope.
    pub fn session_context_scoped(
        &self,
        project: &str,
        scope: Option<&str>,
        limit: i64,
    ) -> HyphaeResult<Vec<Session>> {
        self.session_context_legacy(project, scope, limit)
    }

    /// Get recent sessions using additive v1 identity fields when present.
    ///
    /// Exact `project + project_root + worktree_id` matches take precedence.
    /// When `scope` is provided, it is part of the exact lookup so parallel
    /// sessions for the same worktree stay distinct. Partial identity input is
    /// normalized to legacy behavior.
    pub fn session_context_identity(
        &self,
        project: &str,
        project_root: Option<&str>,
        worktree_id: Option<&str>,
        scope: Option<&str>,
        limit: i64,
    ) -> HyphaeResult<Vec<Session>> {
        let (project_root, worktree_id) = normalized_identity(project_root, worktree_id);

        if let (Some(project_root), Some(worktree_id)) = (project_root, worktree_id) {
            return self.session_context_by_identity(
                project,
                project_root,
                worktree_id,
                scope,
                limit,
            );
        }

        self.session_context_scoped(project, scope, limit)
    }

    /// Get sessions for a project within a time window, optionally filtered to a scope.
    pub fn session_context_between(
        &self,
        project: Option<&str>,
        scope: Option<&str>,
        occurred_after: &str,
        occurred_before: &str,
        limit: i64,
    ) -> HyphaeResult<Vec<Session>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, project, project_root, worktree_id, scope, task, started_at, ended_at, summary, files_modified, errors, status, runtime_session_id
                 FROM sessions
                 WHERE (?1 IS NULL OR project = ?1)
                   AND (?2 IS NULL OR scope = ?2)
                   AND COALESCE(ended_at, started_at) >= ?3
                   AND COALESCE(ended_at, started_at) <= ?4
                 ORDER BY COALESCE(ended_at, started_at) DESC
                 LIMIT ?5",
            )
            .map_err(|e| HyphaeError::Database(format!("failed to prepare query: {e}")))?;

        let rows = stmt
            .query_map(
                params![project, scope, occurred_after, occurred_before, limit],
                load_session,
            )
            .map_err(|e| HyphaeError::Database(format!("failed to query sessions: {e}")))?;

        let sessions: Vec<Session> = rows.filter_map(Result::ok).collect();
        Ok(sessions)
    }

    /// Look up one session by id.
    pub fn session_status(&self, session_id: &str) -> HyphaeResult<Option<Session>> {
        self.conn
            .query_row(
                "SELECT id, project, project_root, worktree_id, scope, task, started_at, ended_at, summary, files_modified, errors, status, runtime_session_id
                 FROM sessions
                 WHERE id = ?1",
                params![session_id],
                load_session,
            )
            .optional()
            .map_err(|e| HyphaeError::Database(format!("failed to query session status: {e}")))
    }

    /// Get a project-scoped session timeline with structured recall and outcome events.
    pub fn session_timeline_identity(
        &self,
        project: &str,
        project_root: Option<&str>,
        worktree_id: Option<&str>,
        scope: Option<&str>,
        limit: i64,
    ) -> HyphaeResult<Vec<SessionTimelineRecord>> {
        let sessions =
            self.session_context_identity(project, project_root, worktree_id, scope, i64::MAX)?;
        self.build_session_timeline(sessions, limit)
    }

    /// Get a cross-project session timeline with structured recall and outcome events.
    pub fn session_timeline_all(&self, limit: i64) -> HyphaeResult<Vec<SessionTimelineRecord>> {
        let sessions = self.session_context_all(i64::MAX)?;
        self.build_session_timeline(sessions, limit)
    }

    fn build_session_timeline(
        &self,
        sessions: Vec<Session>,
        limit: i64,
    ) -> HyphaeResult<Vec<SessionTimelineRecord>> {
        let mut timeline = Vec::with_capacity(sessions.len());
        for session in sessions {
            let events = self.load_session_timeline_events(&session.id)?;
            let recall_count = events.iter().filter(|event| event.kind == "recall").count();
            let outcome_count = events
                .iter()
                .filter(|event| event.kind == "outcome")
                .count();
            let last_activity_at = session_last_activity(&session, &events);

            timeline.push(SessionTimelineRecord {
                id: session.id,
                project: session.project,
                project_root: session.project_root,
                worktree_id: session.worktree_id,
                scope: session.scope,
                runtime_session_id: session.runtime_session_id,
                task: session.task,
                started_at: session.started_at,
                ended_at: session.ended_at,
                summary: session.summary,
                files_modified: session.files_modified,
                errors: session.errors,
                status: session.status,
                events,
                last_activity_at,
                recall_count,
                outcome_count,
            });
        }

        timeline.sort_by(|left, right| {
            timestamp_sort_key(&right.last_activity_at)
                .cmp(&timestamp_sort_key(&left.last_activity_at))
        });

        if let Ok(limit) = usize::try_from(limit)
            && timeline.len() > limit
        {
            timeline.truncate(limit);
        }

        Ok(timeline)
    }

    fn find_active_session(
        &self,
        project: &str,
        project_root: Option<&str>,
        worktree_id: Option<&str>,
        scope: Option<&str>,
    ) -> HyphaeResult<Option<(String, String)>> {
        if let (Some(project_root), Some(worktree_id)) = (project_root, worktree_id) {
            return self.query_active_session_by_identity(
                project,
                project_root,
                worktree_id,
                scope,
            );
        }

        self.query_active_legacy_session(project, scope)
    }

    fn recalled_context_from_signals(
        &self,
        project: &str,
        project_root: Option<&str>,
        worktree_id: Option<&str>,
        context_signals: Option<&Value>,
        embedder: Option<&dyn Embedder>,
    ) -> Vec<(Memory, f32)> {
        let Some(signals) = context_signals else {
            return Vec::new();
        };
        let Some(query) = signals_to_query(signals) else {
            return Vec::new();
        };

        let scoped_worktree = project_root.or(worktree_id);
        if let Some(embedder) = embedder
            && let Ok(embedding) = embedder.embed(&query)
        {
            let results = if let Some(worktree) = scoped_worktree {
                self.search_hybrid_scoped(&query, &embedding, 5, 0, Some(project), Some(worktree))
            } else {
                self.search_hybrid(&query, &embedding, 5, 0, Some(project))
            };

            if let Ok(results) = results {
                return results;
            }
        }

        let results = if let Some(worktree) = scoped_worktree {
            self.search_fts_scoped(&query, 5, 0, Some(project), Some(worktree))
        } else {
            self.search_fts(&query, 5, 0, Some(project))
        };

        results
            .unwrap_or_default()
            .into_iter()
            .enumerate()
            .map(|(idx, memory)| (memory, 1.0 / (idx as f32 + 1.0)))
            .collect::<Vec<(Memory, f32)>>()
    }

    fn query_active_session_by_identity(
        &self,
        project: &str,
        project_root: &str,
        worktree_id: &str,
        scope: Option<&str>,
    ) -> HyphaeResult<Option<(String, String)>> {
        self.conn
            .query_row(
                "SELECT id, started_at
                 FROM sessions
                 WHERE project = ?1
                   AND project_root = ?2
                   AND worktree_id = ?3
                   AND scope IS ?4
                   AND status = 'active'
                 ORDER BY started_at DESC
                 LIMIT 1",
                params![project, project_root, worktree_id, scope],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| HyphaeError::Database(format!("failed to query active session: {e}")))
    }

    fn query_active_legacy_session(
        &self,
        project: &str,
        scope: Option<&str>,
    ) -> HyphaeResult<Option<(String, String)>> {
        self.conn
            .query_row(
                "SELECT id, started_at
                 FROM sessions
                 WHERE project = ?1 AND scope IS ?2 AND status = 'active'
                 ORDER BY started_at DESC
                 LIMIT 1",
                params![project, scope],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| HyphaeError::Database(format!("failed to query active session: {e}")))
    }

    fn session_context_by_identity(
        &self,
        project: &str,
        project_root: &str,
        worktree_id: &str,
        scope: Option<&str>,
        limit: i64,
    ) -> HyphaeResult<Vec<Session>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, project, project_root, worktree_id, scope, task, started_at, ended_at, summary, files_modified, errors, status, runtime_session_id
                 FROM sessions
                 WHERE project = ?1
                   AND project_root = ?2
                   AND worktree_id = ?3
                   AND (?4 IS NULL OR scope = ?4)
                 ORDER BY started_at DESC
                 LIMIT ?5",
            )
            .map_err(|e| HyphaeError::Database(format!("failed to prepare query: {e}")))?;

        let rows = stmt
            .query_map(
                params![project, project_root, worktree_id, scope, limit],
                load_session,
            )
            .map_err(|e| HyphaeError::Database(format!("failed to query sessions: {e}")))?;

        Ok(rows.filter_map(Result::ok).collect())
    }

    fn session_context_legacy(
        &self,
        project: &str,
        scope: Option<&str>,
        limit: i64,
    ) -> HyphaeResult<Vec<Session>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, project, project_root, worktree_id, scope, task, started_at, ended_at, summary, files_modified, errors, status, runtime_session_id
                 FROM sessions
                 WHERE project = ?1
                   AND (?2 IS NULL OR scope = ?2)
                 ORDER BY started_at DESC
                 LIMIT ?3",
            )
            .map_err(|e| HyphaeError::Database(format!("failed to prepare query: {e}")))?;

        let rows = stmt
            .query_map(params![project, scope, limit], load_session)
            .map_err(|e| HyphaeError::Database(format!("failed to query sessions: {e}")))?;

        Ok(rows.filter_map(Result::ok).collect())
    }

    fn load_session_timeline_events(
        &self,
        session_id: &str,
    ) -> HyphaeResult<Vec<SessionTimelineEvent>> {
        let recalls = self.load_recall_timeline_rows(session_id)?;
        let recall_by_id = recalls
            .iter()
            .map(|row| (row.id.as_str(), row))
            .collect::<std::collections::HashMap<_, _>>();
        let mut events = Vec::with_capacity(recalls.len());

        for row in &recalls {
            events.push(SessionTimelineEvent {
                id: row.id.clone(),
                kind: "recall".to_string(),
                title: format_recall_title(row.memory_count),
                detail: Some(row.query.clone()),
                occurred_at: row.recalled_at.clone(),
                recall_event_id: Some(row.id.clone()),
                memory_count: Some(row.memory_count),
                signal_type: None,
                signal_value: None,
                source: None,
            });
        }

        for row in self.load_outcome_timeline_rows(session_id)? {
            let linked_recall = row
                .recall_event_id
                .as_deref()
                .and_then(|recall_id| recall_by_id.get(recall_id))
                .copied();
            let detail = [
                linked_recall.map(|recall| recall.query.as_str()),
                row.source.as_deref(),
            ]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>();

            events.push(SessionTimelineEvent {
                id: row.id,
                kind: "outcome".to_string(),
                title: format_outcome_title(&row.signal_type),
                detail: (!detail.is_empty()).then(|| detail.join(" · ")),
                occurred_at: row.occurred_at,
                recall_event_id: row.recall_event_id,
                memory_count: linked_recall.map(|recall| recall.memory_count),
                signal_type: Some(row.signal_type),
                signal_value: Some(row.signal_value),
                source: row.source,
            });
        }

        events.sort_by(|left, right| {
            timestamp_sort_key(&right.occurred_at).cmp(&timestamp_sort_key(&left.occurred_at))
        });

        Ok(events)
    }

    fn load_recall_timeline_rows(&self, session_id: &str) -> HyphaeResult<Vec<RecallTimelineRow>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, query, recalled_at, memory_count
                 FROM recall_events
                 WHERE session_id = ?1
                 ORDER BY recalled_at DESC",
            )
            .map_err(|e| {
                HyphaeError::Database(format!("failed to prepare recall timeline query: {e}"))
            })?;

        let rows = stmt
            .query_map(params![session_id], |row| {
                Ok(RecallTimelineRow {
                    id: row.get(0)?,
                    query: row.get(1)?,
                    recalled_at: row.get(2)?,
                    memory_count: row.get(3)?,
                })
            })
            .map_err(|e| {
                HyphaeError::Database(format!("failed to query recall timeline rows: {e}"))
            })?;

        rows.collect::<Result<Vec<_>, _>>().map_err(|e| {
            HyphaeError::Database(format!("failed to collect recall timeline rows: {e}"))
        })
    }

    fn load_outcome_timeline_rows(
        &self,
        session_id: &str,
    ) -> HyphaeResult<Vec<OutcomeTimelineRow>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, recall_event_id, signal_type, signal_value, occurred_at, source
                 FROM outcome_signals
                 WHERE session_id = ?1
                 ORDER BY occurred_at DESC",
            )
            .map_err(|e| {
                HyphaeError::Database(format!("failed to prepare outcome timeline query: {e}"))
            })?;

        let rows = stmt
            .query_map(params![session_id], |row| {
                Ok(OutcomeTimelineRow {
                    id: row.get(0)?,
                    recall_event_id: row.get(1)?,
                    signal_type: row.get(2)?,
                    signal_value: row.get(3)?,
                    occurred_at: row.get(4)?,
                    source: row.get(5)?,
                })
            })
            .map_err(|e| {
                HyphaeError::Database(format!("failed to query outcome timeline rows: {e}"))
            })?;

        rows.collect::<Result<Vec<_>, _>>().map_err(|e| {
            HyphaeError::Database(format!("failed to collect outcome timeline rows: {e}"))
        })
    }
}

fn signals_to_query(signals: &Value) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();

    if let Some(files) = signals.get("recent_files").and_then(Value::as_array) {
        let names: Vec<&str> = files
            .iter()
            .filter_map(Value::as_str)
            .filter_map(|path| Path::new(path).file_stem()?.to_str())
            .take(5)
            .collect();
        if !names.is_empty() {
            parts.push(names.join(" "));
        }
    }

    if let Some(errors) = signals.get("active_errors").and_then(Value::as_array) {
        let error_text: Vec<&str> = errors.iter().filter_map(Value::as_str).take(3).collect();
        if !error_text.is_empty() {
            parts.push(error_text.join(" "));
        }
    }

    if let Some(branch) = signals.get("git_branch").and_then(Value::as_str) {
        if !matches!(branch, "main" | "master" | "develop") {
            parts.push(branch.replace(['-', '/'], " "));
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

fn normalized_identity<'a>(
    project_root: Option<&'a str>,
    worktree_id: Option<&'a str>,
) -> (Option<&'a str>, Option<&'a str>) {
    match (project_root, worktree_id) {
        (Some(project_root), Some(worktree_id)) => (Some(project_root), Some(worktree_id)),
        _ => (None, None),
    }
}

fn load_session(row: &rusqlite::Row<'_>) -> rusqlite::Result<Session> {
    Ok(Session {
        id: row.get(0)?,
        project: row.get(1)?,
        project_root: row.get(2)?,
        worktree_id: row.get(3)?,
        scope: row.get(4)?,
        task: row.get(5)?,
        started_at: row.get(6)?,
        ended_at: row.get(7)?,
        summary: row.get(8)?,
        files_modified: row.get(9)?,
        errors: row.get(10)?,
        status: row.get(11)?,
        runtime_session_id: row.get(12)?,
    })
}

fn format_recall_title(memory_count: i64) -> String {
    format!(
        "Recalled {memory_count} {}",
        if memory_count == 1 {
            "memory"
        } else {
            "memories"
        }
    )
}

fn format_outcome_title(signal_type: &str) -> String {
    match signal_type {
        "build_passed" => "Build passed".to_string(),
        "correction" => "Correction detected".to_string(),
        "error_free_run" => "Error-free run".to_string(),
        "error_resolved" => "Error resolved".to_string(),
        "explicit_boost" => "Manual boost recorded".to_string(),
        "session_failure" => "Session ended with failures".to_string(),
        "session_success" => "Session completed successfully".to_string(),
        "test_pass" | "test_passed" => "Tests passed".to_string(),
        "tool_error" => "Tool error captured".to_string(),
        _ => {
            let normalized = signal_type.replace('_', " ").trim().to_string();
            if normalized.is_empty() {
                "Outcome recorded".to_string()
            } else {
                let mut chars = normalized.chars();
                let Some(first) = chars.next() else {
                    return "Outcome recorded".to_string();
                };
                first.to_uppercase().collect::<String>() + chars.as_str()
            }
        }
    }
}

fn session_last_activity(session: &Session, events: &[SessionTimelineEvent]) -> String {
    let mut timestamps = vec![session.started_at.as_str()];
    if let Some(ended_at) = session.ended_at.as_deref() {
        timestamps.push(ended_at);
    }
    timestamps.extend(events.iter().map(|event| event.occurred_at.as_str()));

    timestamps
        .into_iter()
        .max_by_key(|timestamp| timestamp_sort_key(timestamp))
        .unwrap_or(session.started_at.as_str())
        .to_string()
}

fn timestamp_sort_key(timestamp: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(timestamp)
        .map(|value| value.timestamp_micros())
        .unwrap_or(i64::MIN)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn test_store() -> SqliteStore {
        SqliteStore::in_memory().expect("in-memory store")
    }

    #[test]
    fn test_signals_to_query_uses_files_errors_and_branch() {
        let query = signals_to_query(&json!({
            "recent_files": [
                "/repo/demo/src/session_scope.rs",
                "/repo/demo/src/lib.rs"
            ],
            "active_errors": [
                "failed to compile session start",
                "missing recalled context"
            ],
            "git_branch": "feat/context-aware-recall"
        }))
        .expect("query");

        assert!(query.contains("session_scope"));
        assert!(query.contains("failed to compile session start"));
        assert!(query.contains("feat context aware recall"));
    }

    #[test]
    fn test_session_lifecycle() {
        let store = test_store();

        // Start
        let (sid, started_at) = store
            .session_start("test-project", Some("implement feature"))
            .unwrap();
        assert!(sid.starts_with("ses_"));
        assert!(!started_at.is_empty());

        // Context shows active session
        let sessions = store.session_context("test-project", 10).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].status, "active");
        assert_eq!(sessions[0].scope, None);

        // End
        let (project, _, task, _, duration) = store
            .session_end(&sid, Some("done"), Some("[\"file.rs\"]"), Some("0"))
            .unwrap();
        assert_eq!(project, "test-project");
        assert_eq!(task.as_deref(), Some("implement feature"));
        assert!(duration >= 0);

        // Context shows completed
        let sessions = store.session_context("test-project", 10).unwrap();
        assert_eq!(sessions[0].status, "completed");
    }

    #[test]
    fn test_session_end_invalid_id() {
        let store = test_store();
        let result = store.session_end("nonexistent", None, None, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_session_context_empty() {
        let store = test_store();
        let sessions = store.session_context("no-such-project", 5).unwrap();
        assert!(sessions.is_empty());
    }

    #[test]
    fn test_session_context_between_filters_by_started_at_window() {
        let store = test_store();
        let started_before = Utc::now().to_rfc3339();

        let (session_id, _) = store
            .session_start("window-project", Some("capture window"))
            .unwrap();
        let started_after = Utc::now().to_rfc3339();

        let sessions = store
            .session_context_between(
                Some("window-project"),
                None,
                &started_before,
                &started_after,
                10,
            )
            .unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, session_id);
    }

    #[test]
    fn test_session_context_between_uses_session_end_when_present() {
        let store = test_store();
        let old_started_at = (Utc::now() - chrono::Duration::days(2)).to_rfc3339();
        let recent_ended_at = Utc::now().to_rfc3339();
        let window_start = (Utc::now() - chrono::Duration::days(1)).to_rfc3339();

        store
            .conn
            .execute(
                "INSERT INTO sessions (id, project, scope, task, started_at, ended_at, summary, files_modified, errors, status)
                 VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, NULL, ?7, 'completed')",
                params![
                    "ses_windowed",
                    "window-project",
                    "carry over work",
                    old_started_at,
                    recent_ended_at,
                    "finished inside the window",
                    "0",
                ],
            )
            .unwrap();

        let sessions = store
            .session_context_between(
                Some("window-project"),
                None,
                &window_start,
                "9999-12-31T23:59:59Z",
                10,
            )
            .unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].id, "ses_windowed");
    }

    #[test]
    fn test_session_context_all_includes_recent_sessions_across_projects() {
        let store = test_store();
        let (session_a, _) = store.session_start("proj-a", Some("task a")).unwrap();
        store
            .session_end(&session_a, Some("summary a"), None, Some("0"))
            .unwrap();

        let (session_b, _) = store.session_start("proj-b", Some("task b")).unwrap();
        store
            .session_end(&session_b, Some("summary b"), None, Some("0"))
            .unwrap();

        let sessions = store.session_context_all(10).unwrap();
        assert_eq!(sessions.len(), 2);
        assert!(sessions.iter().any(|session| session.project == "proj-a"));
        assert!(sessions.iter().any(|session| session.project == "proj-b"));
    }

    #[test]
    fn test_session_start_reuses_existing_active_session() {
        let store = test_store();
        let (first_id, first_started_at) = store.session_start("demo", Some("first")).unwrap();
        let (second_id, second_started_at) = store.session_start("demo", Some("second")).unwrap();

        assert_eq!(first_id, second_id);
        assert_eq!(first_started_at, second_started_at);

        let sessions = store.session_context("demo", 10).unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].status, "active");
    }

    #[test]
    fn test_session_start_scoped_separates_parallel_workers() {
        let store = test_store();
        let (worker_a, _) = store
            .session_start_scoped("demo", Some("first"), Some("worker-a"))
            .unwrap();
        let (worker_b, _) = store
            .session_start_scoped("demo", Some("second"), Some("worker-b"))
            .unwrap();
        let (worker_a_again, _) = store
            .session_start_scoped("demo", Some("first-again"), Some("worker-a"))
            .unwrap();

        assert_ne!(worker_a, worker_b);
        assert_eq!(worker_a, worker_a_again);

        let sessions = store.session_context("demo", 10).unwrap();
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].status, "active");
        assert_eq!(sessions[1].status, "active");
    }

    #[test]
    fn test_session_start_identity_respects_scope_for_parallel_workers() {
        let store = test_store();
        let (first_id, _) = store
            .session_start_identity(
                "demo",
                Some("first"),
                Some("/repo/demo"),
                Some("wt-alpha"),
                Some("worker-a"),
            )
            .unwrap();
        let (second_id, _) = store
            .session_start_identity(
                "demo",
                Some("second"),
                Some("/repo/demo"),
                Some("wt-alpha"),
                Some("worker-b"),
            )
            .unwrap();
        let (first_again_id, _) = store
            .session_start_identity(
                "demo",
                Some("first-again"),
                Some("/repo/demo"),
                Some("wt-alpha"),
                Some("worker-a"),
            )
            .unwrap();

        assert_ne!(first_id, second_id);
        assert_eq!(first_id, first_again_id);

        let worker_a_sessions = store
            .session_context_identity(
                "demo",
                Some("/repo/demo"),
                Some("wt-alpha"),
                Some("worker-a"),
                10,
            )
            .unwrap();
        let worker_b_sessions = store
            .session_context_identity(
                "demo",
                Some("/repo/demo"),
                Some("wt-alpha"),
                Some("worker-b"),
                10,
            )
            .unwrap();

        assert_eq!(worker_a_sessions.len(), 1);
        assert_eq!(worker_b_sessions.len(), 1);
        assert_eq!(worker_a_sessions[0].id, first_id);
        assert_eq!(worker_b_sessions[0].id, second_id);
        assert_eq!(
            worker_a_sessions[0].project_root.as_deref(),
            Some("/repo/demo")
        );
        assert_eq!(
            worker_a_sessions[0].worktree_id.as_deref(),
            Some("wt-alpha")
        );
    }

    #[test]
    fn test_session_start_identity_does_not_cross_project_boundaries() {
        let store = test_store();
        let (project_a_id, _) = store
            .session_start_identity(
                "demo-a",
                Some("first"),
                Some("/repo/shared"),
                Some("wt-alpha"),
                Some("worker-a"),
            )
            .unwrap();
        let (project_b_id, _) = store
            .session_start_identity(
                "demo-b",
                Some("second"),
                Some("/repo/shared"),
                Some("wt-alpha"),
                Some("worker-a"),
            )
            .unwrap();

        assert_ne!(project_a_id, project_b_id);

        let project_a_sessions = store
            .session_context_identity(
                "demo-a",
                Some("/repo/shared"),
                Some("wt-alpha"),
                Some("worker-a"),
                10,
            )
            .unwrap();
        let project_b_sessions = store
            .session_context_identity(
                "demo-b",
                Some("/repo/shared"),
                Some("wt-alpha"),
                Some("worker-a"),
                10,
            )
            .unwrap();

        assert_eq!(project_a_sessions.len(), 1);
        assert_eq!(project_b_sessions.len(), 1);
        assert_eq!(project_a_sessions[0].id, project_a_id);
        assert_eq!(project_b_sessions[0].id, project_b_id);
    }

    #[test]
    fn test_session_start_identity_does_not_reuse_legacy_scope_session() {
        let store = test_store();
        let (legacy_id, _) = store
            .session_start_scoped("demo", Some("legacy"), Some("worker-a"))
            .unwrap();

        let (identity_id, _) = store
            .session_start_identity(
                "demo",
                Some("new caller"),
                Some("/repo/demo"),
                Some("wt-alpha"),
                Some("worker-a"),
            )
            .unwrap();

        assert_ne!(legacy_id, identity_id);
    }

    #[test]
    fn test_session_context_identity_prefers_exact_match_over_project_scope() {
        let store = test_store();

        store
            .session_start_identity(
                "demo",
                Some("alpha"),
                Some("/repo/demo"),
                Some("wt-alpha"),
                Some("worker-a"),
            )
            .unwrap();
        store
            .session_start_identity(
                "demo",
                Some("beta"),
                Some("/repo/demo"),
                Some("wt-beta"),
                Some("worker-a"),
            )
            .unwrap();

        let sessions = store
            .session_context_identity(
                "demo",
                Some("/repo/demo"),
                Some("wt-alpha"),
                Some("worker-a"),
                10,
            )
            .unwrap();

        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].worktree_id.as_deref(), Some("wt-alpha"));
    }

    #[test]
    fn test_session_context_identity_does_not_return_legacy_rows() {
        let store = test_store();
        let (_legacy_id, _) = store
            .session_start_scoped("demo", Some("legacy"), Some("worker-a"))
            .unwrap();

        let sessions = store
            .session_context_identity(
                "demo",
                Some("/repo/demo"),
                Some("wt-alpha"),
                Some("worker-a"),
                10,
            )
            .unwrap();

        assert!(sessions.is_empty());
    }

    #[test]
    fn test_partial_identity_is_normalized_to_legacy_behavior() {
        let store = test_store();
        let (first_id, _) = store
            .session_start_identity(
                "demo",
                Some("first"),
                Some("/repo/demo"),
                None,
                Some("worker-a"),
            )
            .unwrap();
        let (second_id, _) = store
            .session_start_scoped("demo", Some("second"), Some("worker-a"))
            .unwrap();

        assert_eq!(first_id, second_id);

        let session = store.session_status(&first_id).unwrap().unwrap();
        assert!(session.project_root.is_none());
        assert!(session.worktree_id.is_none());
    }

    #[test]
    fn test_session_context_scoped_filters_parallel_workers() {
        let store = test_store();
        let (worker_a, _) = store
            .session_start_scoped("demo", Some("first"), Some("worker-a"))
            .unwrap();
        let (worker_b, _) = store
            .session_start_scoped("demo", Some("second"), Some("worker-b"))
            .unwrap();

        let worker_a_sessions = store
            .session_context_scoped("demo", Some("worker-a"), 10)
            .unwrap();
        let worker_b_sessions = store
            .session_context_scoped("demo", Some("worker-b"), 10)
            .unwrap();

        assert_eq!(worker_a_sessions.len(), 1);
        assert_eq!(worker_b_sessions.len(), 1);
        assert_eq!(worker_a_sessions[0].id, worker_a);
        assert_eq!(worker_b_sessions[0].id, worker_b);
    }

    #[test]
    fn test_session_status_returns_one_session() {
        let store = test_store();
        let (session_id, _) = store
            .session_start_scoped("demo", Some("first"), Some("worker-a"))
            .unwrap();

        let session = store.session_status(&session_id).unwrap().unwrap();
        assert_eq!(session.id, session_id);
        assert_eq!(session.project, "demo");
        assert!(session.project_root.is_none());
        assert!(session.worktree_id.is_none());
        assert_eq!(session.scope.as_deref(), Some("worker-a"));
        assert_eq!(session.status, "active");
    }

    #[test]
    fn test_session_status_returns_identity_fields() {
        let store = test_store();
        let (session_id, _) = store
            .session_start_identity(
                "demo",
                Some("first"),
                Some("/repo/demo"),
                Some("wt-alpha"),
                Some("worker-a"),
            )
            .unwrap();

        let session = store.session_status(&session_id).unwrap().unwrap();
        assert_eq!(session.project_root.as_deref(), Some("/repo/demo"));
        assert_eq!(session.worktree_id.as_deref(), Some("wt-alpha"));
    }

    #[test]
    fn test_session_status_returns_completed_session() {
        let store = test_store();
        let (session_id, _) = store
            .session_start_scoped("demo", Some("first"), Some("worker-a"))
            .unwrap();
        store
            .session_end(&session_id, Some("done"), None, Some("0"))
            .unwrap();

        let session = store.session_status(&session_id).unwrap().unwrap();
        assert_eq!(session.id, session_id);
        assert_eq!(session.status, "completed");
        assert!(session.ended_at.is_some());
    }

    #[test]
    fn test_session_status_returns_none_for_unknown_session() {
        let store = test_store();
        let session = store.session_status("ses_missing").unwrap();
        assert!(session.is_none());
    }

    #[test]
    fn test_session_timeline_identity_returns_cap_compatible_timeline() {
        let store = test_store();
        let (session_id, _) = store
            .session_start_identity(
                "cap",
                Some("build session timeline"),
                Some("/repo/cap"),
                Some("wt-alpha"),
                Some("worker-a"),
            )
            .unwrap();

        let recall_id = store
            .log_recall_event(
                Some(&session_id),
                "session attribution bridge",
                &[
                    "mem_1".to_string(),
                    "mem_2".to_string(),
                    "mem_3".to_string(),
                ],
                Some("cap"),
            )
            .unwrap();
        let outcome_id = store
            .log_outcome_signal(
                Some(&session_id),
                "test_passed",
                1,
                Some("cortina.post_tool_use.test"),
                Some("cap"),
            )
            .unwrap();
        store
            .session_end(
                &session_id,
                Some("Connected session recall and outcome signals."),
                Some("[\"src/pages/Sessions.tsx\",\"server/hyphae.ts\"]"),
                Some("2"),
            )
            .unwrap();

        let timeline = store
            .session_timeline_identity(
                "cap",
                Some("/repo/cap"),
                Some("wt-alpha"),
                Some("worker-a"),
                10,
            )
            .unwrap();

        assert_eq!(timeline.len(), 1);
        let record = &timeline[0];
        assert_eq!(record.id, session_id);
        assert_eq!(record.project, "cap");
        assert_eq!(record.project_root.as_deref(), Some("/repo/cap"));
        assert_eq!(record.worktree_id.as_deref(), Some("wt-alpha"));
        assert_eq!(record.scope.as_deref(), Some("worker-a"));
        assert_eq!(record.task.as_deref(), Some("build session timeline"));
        assert_eq!(
            record.summary.as_deref(),
            Some("Connected session recall and outcome signals.")
        );
        assert_eq!(
            record.files_modified.as_deref(),
            Some("[\"src/pages/Sessions.tsx\",\"server/hyphae.ts\"]")
        );
        assert_eq!(record.errors.as_deref(), Some("2"));
        assert_eq!(record.status, "completed");
        assert_eq!(record.recall_count, 1);
        assert_eq!(record.outcome_count, 2);
        assert_eq!(record.events.len(), 3);
        assert_eq!(record.last_activity_at, record.ended_at.clone().unwrap());

        let recall = record
            .events
            .iter()
            .find(|event| event.id == recall_id)
            .unwrap();
        assert_eq!(recall.kind, "recall");
        assert_eq!(recall.title, "Recalled 3 memories");
        assert_eq!(recall.detail.as_deref(), Some("session attribution bridge"));
        assert_eq!(recall.recall_event_id.as_deref(), Some(recall_id.as_str()));
        assert_eq!(recall.memory_count, Some(3));
        assert_eq!(recall.signal_type, None);
        assert_eq!(recall.signal_value, None);
        assert_eq!(recall.source, None);

        let test_pass = record
            .events
            .iter()
            .find(|event| event.id == outcome_id)
            .unwrap();
        assert_eq!(test_pass.kind, "outcome");
        assert_eq!(test_pass.title, "Tests passed");
        assert_eq!(
            test_pass.detail.as_deref(),
            Some("session attribution bridge · cortina.post_tool_use.test")
        );
        assert_eq!(
            test_pass.recall_event_id.as_deref(),
            Some(recall_id.as_str())
        );
        assert_eq!(test_pass.memory_count, Some(3));
        assert_eq!(test_pass.signal_type.as_deref(), Some("test_passed"));
        assert_eq!(test_pass.signal_value, Some(1));
        assert_eq!(
            test_pass.source.as_deref(),
            Some("cortina.post_tool_use.test")
        );

        let session_failure = record
            .events
            .iter()
            .find(|event| event.signal_type.as_deref() == Some("session_failure"))
            .unwrap();
        assert_eq!(session_failure.title, "Session ended with failures");
        assert_eq!(
            session_failure.detail.as_deref(),
            Some("session attribution bridge · hyphae.session_end")
        );
        assert_eq!(session_failure.memory_count, Some(3));
    }

    #[test]
    fn test_session_timeline_identity_sorts_sessions_by_last_activity() {
        let store = test_store();
        let (older_id, _) = store
            .session_start_identity(
                "cap",
                Some("older"),
                Some("/repo/cap"),
                Some("wt-alpha"),
                Some("worker-a"),
            )
            .unwrap();
        store
            .session_end(&older_id, Some("older"), None, Some("0"))
            .unwrap();

        let (newer_id, _) = store
            .session_start_identity(
                "cap",
                Some("newer"),
                Some("/repo/cap"),
                Some("wt-alpha"),
                Some("worker-a"),
            )
            .unwrap();
        store
            .log_recall_event(
                Some(&newer_id),
                "newer recall",
                &["mem_1".to_string()],
                Some("cap"),
            )
            .unwrap();

        let timeline = store
            .session_timeline_identity(
                "cap",
                Some("/repo/cap"),
                Some("wt-alpha"),
                Some("worker-a"),
                10,
            )
            .unwrap();

        assert_eq!(timeline.len(), 2);
        assert_eq!(timeline[0].id, newer_id);
        assert_eq!(timeline[1].id, older_id);
        assert!(timeline[0].last_activity_at >= timeline[1].last_activity_at);
    }

    #[test]
    fn test_session_timeline_identity_applies_limit_after_last_activity_sort() {
        let store = test_store();
        let (older_id, older_started_at) = store
            .session_start_identity(
                "cap",
                Some("older"),
                Some("/repo/cap"),
                Some("wt-alpha"),
                Some("worker-a"),
            )
            .unwrap();
        store
            .session_end(&older_id, Some("older"), None, Some("0"))
            .unwrap();

        let (newer_id, newer_started_at) = store
            .session_start_identity(
                "cap",
                Some("newer"),
                Some("/repo/cap"),
                Some("wt-alpha"),
                Some("worker-a"),
            )
            .unwrap();

        store
            .conn
            .execute(
                "UPDATE sessions SET started_at = ?1 WHERE id = ?2",
                params![older_started_at, older_id],
            )
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE sessions SET started_at = ?1 WHERE id = ?2",
                params![newer_started_at, newer_id],
            )
            .unwrap();

        store
            .log_recall_event(
                Some(&older_id),
                "older recall",
                &["mem_1".to_string()],
                Some("cap"),
            )
            .unwrap();

        let timeline = store
            .session_timeline_identity(
                "cap",
                Some("/repo/cap"),
                Some("wt-alpha"),
                Some("worker-a"),
                1,
            )
            .unwrap();

        assert_eq!(timeline.len(), 1);
        assert_eq!(timeline[0].id, older_id);
    }

    #[test]
    fn test_session_timeline_all_includes_cross_project_recent_activity() {
        let store = test_store();
        let (project_a_id, _) = store
            .session_start_identity(
                "cap-a",
                Some("older"),
                Some("/repo/cap-a"),
                Some("wt-alpha"),
                Some("worker-a"),
            )
            .unwrap();
        store
            .session_end(&project_a_id, Some("older"), None, Some("0"))
            .unwrap();

        let (project_b_id, _) = store
            .session_start_identity(
                "cap-b",
                Some("newer"),
                Some("/repo/cap-b"),
                Some("wt-beta"),
                Some("worker-b"),
            )
            .unwrap();
        store
            .log_recall_event(
                Some(&project_b_id),
                "cross-project recall",
                &["mem_1".to_string()],
                Some("cap-b"),
            )
            .unwrap();

        let timeline = store.session_timeline_all(10).unwrap();

        assert_eq!(timeline.len(), 2);
        assert_eq!(timeline[0].id, project_b_id);
        assert_eq!(timeline[1].id, project_a_id);
    }
}
