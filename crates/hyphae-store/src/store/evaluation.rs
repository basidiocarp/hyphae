use std::cmp::Ordering;
use std::collections::HashMap;

use chrono::{DateTime, Duration, Utc};

use hyphae_core::{HyphaeResult, Memory, MemoryStore};

use super::{SqliteStore, session::Session};

const MIRROR_TOLERANCE_MINUTES: i64 = 5;
const CORRECTION_MIRROR_TOLERANCE_SECONDS: i64 = 30;

#[derive(Debug, Clone)]
pub struct EvaluationWindow {
    pub error_count: usize,
    pub correction_count: usize,
    pub resolved_count: usize,
    pub failed_test_count: usize,
    pub resolved_test_count: usize,
    pub total_session_length: usize,
    pub session_count: usize,
    pub recalled_memory_count: usize,
}

#[derive(Debug, Clone)]
pub struct RecallEffectivenessRow {
    pub memory_id: String,
    pub effectiveness: f32,
}

#[derive(Debug, Clone)]
struct RecallEffectivenessOccurrence {
    memory_id: String,
    recalled_at: Option<DateTime<Utc>>,
    effectiveness: Option<f32>,
}

#[derive(Debug, Clone)]
pub struct RecallEffectivenessWindow {
    pub recalled_memory_count: usize,
    pub scored_memory_count: usize,
    pub non_zero_score_count: usize,
    pub total_effectiveness: f64,
    pub top_recalled_memories: Vec<RecallEffectivenessRow>,
}

impl RecallEffectivenessWindow {
    #[must_use]
    pub fn non_zero_score_fraction(&self) -> f64 {
        if self.recalled_memory_count == 0 {
            return 0.0;
        }
        self.non_zero_score_count as f64 / self.recalled_memory_count as f64
    }

    #[must_use]
    pub fn average_effectiveness(&self) -> f64 {
        if self.scored_memory_count == 0 {
            return 0.0;
        }
        self.total_effectiveness / self.scored_memory_count as f64
    }

    #[must_use]
    pub fn score_coverage(&self) -> f64 {
        if self.recalled_memory_count == 0 {
            return 0.0;
        }
        self.scored_memory_count as f64 / self.recalled_memory_count as f64
    }
}

impl EvaluationWindow {
    #[must_use]
    pub fn error_rate(&self) -> f64 {
        if self.session_count == 0 {
            return 0.0;
        }
        self.error_count as f64 / self.session_count as f64
    }

    #[must_use]
    pub fn correction_rate(&self) -> f64 {
        if self.session_count == 0 {
            return 0.0;
        }
        self.correction_count as f64 / self.session_count as f64
    }

    #[must_use]
    pub fn resolution_rate(&self) -> f64 {
        let total = self.error_count + self.resolved_count;
        if total == 0 {
            return 0.0;
        }
        self.resolved_count as f64 / total as f64
    }

    #[must_use]
    pub fn test_fix_rate(&self) -> f64 {
        let total = self.failed_test_count + self.resolved_test_count;
        if total == 0 {
            return 0.0;
        }
        self.resolved_test_count as f64 / total as f64
    }

    #[must_use]
    pub fn memory_utilization(&self) -> f64 {
        if self.session_count == 0 {
            return 0.0;
        }
        (self.recalled_memory_count as f64 / (self.recalled_memory_count + 5) as f64) * 100.0
    }
}

fn get_memories_in_window(
    store: &SqliteStore,
    topic_pattern: &str,
    days_ago_start: i64,
    days_ago_end: i64,
    project: Option<&str>,
) -> HyphaeResult<Vec<Memory>> {
    let all_memories = store.get_by_topic(topic_pattern, project)?;

    let cutoff_start = Utc::now()
        .checked_sub_signed(chrono::Duration::days(days_ago_start))
        .unwrap_or(Utc::now());
    let cutoff_end = Utc::now()
        .checked_sub_signed(chrono::Duration::days(days_ago_end))
        .unwrap_or(Utc::now());

    Ok(all_memories
        .into_iter()
        .filter(|m| m.created_at >= cutoff_end && m.created_at <= cutoff_start)
        .collect())
}

fn window_bounds(
    days_ago_start: i64,
    days_ago_end: i64,
) -> (chrono::DateTime<Utc>, chrono::DateTime<Utc>) {
    let recent_bound = Utc::now()
        .checked_sub_signed(chrono::Duration::days(days_ago_start))
        .unwrap_or(Utc::now());
    let older_bound = Utc::now()
        .checked_sub_signed(chrono::Duration::days(days_ago_end))
        .unwrap_or(Utc::now());
    (older_bound, recent_bound)
}

fn session_text(session: &Session) -> String {
    match (&session.task, &session.summary) {
        (Some(task), Some(summary)) => format!("{task} {summary}"),
        (Some(task), None) => task.clone(),
        (None, Some(summary)) => summary.clone(),
        (None, None) => String::new(),
    }
}

fn compatibility_session_summary(session: &Session) -> Option<String> {
    let summary = session.summary.as_deref()?;
    Some(match session.task.as_deref() {
        Some(task) => format!("Session completed: {task}. {summary}"),
        None => format!("Session completed. {summary}"),
    })
}

fn structured_sessions_in_window(
    store: &SqliteStore,
    project: Option<&str>,
    days_ago_start: i64,
    days_ago_end: i64,
) -> HyphaeResult<Vec<Session>> {
    let (older_bound, recent_bound) = window_bounds(days_ago_start, days_ago_end);
    store.session_context_between(
        project,
        None,
        &older_bound.to_rfc3339(),
        &recent_bound.to_rfc3339(),
        10_000,
    )
}

fn structured_signal_count(
    store: &SqliteStore,
    project: Option<&str>,
    signal_type: &str,
    days_ago_start: i64,
    days_ago_end: i64,
) -> HyphaeResult<usize> {
    let (older_bound, recent_bound) = window_bounds(days_ago_start, days_ago_end);
    Ok(store.count_outcome_signals_in_window(
        project,
        signal_type,
        &older_bound.to_rfc3339(),
        &recent_bound.to_rfc3339(),
    )? as usize)
}

fn structured_correction_signals_in_window(
    store: &SqliteStore,
    project: Option<&str>,
    days_ago_start: i64,
    days_ago_end: i64,
) -> HyphaeResult<Vec<super::feedback::OutcomeSignalRecord>> {
    let (older_bound, recent_bound) = window_bounds(days_ago_start, days_ago_end);
    store.outcome_signals_in_window(
        project,
        "correction",
        &older_bound.to_rfc3339(),
        &recent_bound.to_rfc3339(),
    )
}

fn structured_recall_events_in_window(
    store: &SqliteStore,
    project: Option<&str>,
    days_ago_start: i64,
    days_ago_end: i64,
) -> HyphaeResult<Vec<super::feedback::RecallEventRecord>> {
    let (older_bound, recent_bound) = window_bounds(days_ago_start, days_ago_end);
    store.recall_events_in_window(
        project,
        &older_bound.to_rfc3339(),
        &recent_bound.to_rfc3339(),
    )
}

fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn timestamps_match(a: DateTime<Utc>, b: DateTime<Utc>) -> bool {
    (a - b).num_seconds().abs() <= Duration::minutes(MIRROR_TOLERANCE_MINUTES).num_seconds()
}

fn correction_timestamps_match(a: DateTime<Utc>, b: DateTime<Utc>) -> bool {
    (a - b).num_seconds().abs() <= CORRECTION_MIRROR_TOLERANCE_SECONDS
}

fn legacy_session_memories_in_window(
    store: &SqliteStore,
    project: Option<&str>,
    days_ago_start: i64,
    days_ago_end: i64,
) -> HyphaeResult<Vec<Memory>> {
    if let Some(project) = project {
        let session_topic = format!("session/{project}");
        return get_memories_in_window(
            store,
            &session_topic,
            days_ago_start,
            days_ago_end,
            Some(project),
        );
    }

    let (older_bound, recent_bound) = window_bounds(days_ago_start, days_ago_end);
    let topics = store.list_topics(None)?;
    let mut sessions = Vec::new();
    for (topic, _) in topics {
        if !topic.starts_with("session/") {
            continue;
        }
        let mut mems = store.get_by_topic(&topic, None)?;
        mems.retain(|m| m.created_at >= older_bound && m.created_at <= recent_bound);
        sessions.extend(mems);
    }
    Ok(sessions)
}

fn merge_session_counts(
    structured_sessions: &[Session],
    legacy_sessions: &[Memory],
) -> (usize, usize) {
    let mut structured_session_times: HashMap<(String, String), Vec<DateTime<Utc>>> =
        HashMap::new();
    let mut session_count = structured_sessions.len();
    let mut total_session_length = structured_sessions
        .iter()
        .map(|session| session_text(session).len())
        .sum();

    for session in structured_sessions {
        if let (Some(summary), Some(session_time)) = (
            compatibility_session_summary(session),
            session
                .ended_at
                .as_deref()
                .and_then(parse_timestamp)
                .or_else(|| parse_timestamp(&session.started_at)),
        ) {
            structured_session_times
                .entry((session.project.clone(), summary))
                .or_default()
                .push(session_time);
        }
    }

    for legacy in legacy_sessions {
        let dedupe_key = (
            legacy
                .project
                .clone()
                .unwrap_or_else(|| legacy.topic.clone()),
            legacy.summary.clone(),
        );
        match structured_session_times.get_mut(&dedupe_key) {
            Some(candidates) => {
                if let Some(index) = candidates
                    .iter()
                    .position(|candidate| timestamps_match(*candidate, legacy.created_at))
                {
                    candidates.swap_remove(index);
                } else {
                    session_count += 1;
                    total_session_length += legacy.summary.len();
                }
            }
            _ => {
                session_count += 1;
                total_session_length += legacy.summary.len();
            }
        }
    }

    (session_count, total_session_length)
}

fn legacy_recalled_memories_in_window(
    store: &SqliteStore,
    project: Option<&str>,
    days_ago_start: i64,
    days_ago_end: i64,
) -> HyphaeResult<Vec<Memory>> {
    let (older_bound, recent_bound) = window_bounds(days_ago_start, days_ago_end);
    let topics = store.list_topics(project)?;
    let mut recalled = Vec::new();

    for (topic, _) in topics {
        let memories = store.get_by_topic(&topic, project)?;
        recalled.extend(memories.into_iter().filter(|m| {
            m.access_count > 0 && m.last_accessed >= older_bound && m.last_accessed <= recent_bound
        }));
    }

    Ok(recalled)
}

fn merge_correction_counts(
    legacy_corrections: &[Memory],
    structured_corrections: &[super::feedback::OutcomeSignalRecord],
) -> usize {
    let mut used_structured = vec![false; structured_corrections.len()];
    let mut count = structured_corrections.len();

    for legacy in legacy_corrections {
        let matched = structured_corrections
            .iter()
            .enumerate()
            .any(|(index, signal)| {
                if used_structured[index] || signal.signal_type != "correction" {
                    return false;
                }
                if signal.project.as_deref() != legacy.project.as_deref() {
                    return false;
                }
                let Some(occurred_at) = parse_timestamp(&signal.occurred_at) else {
                    return false;
                };
                if correction_timestamps_match(occurred_at, legacy.created_at) {
                    used_structured[index] = true;
                    true
                } else {
                    false
                }
            });

        if !matched {
            count += 1;
        }
    }

    count
}

fn merge_recalled_memory_counts(
    legacy_recalled_memories: &[Memory],
    structured_recall_events: &[super::feedback::RecallEventRecord],
) -> usize {
    let mut structured_recalled = Vec::new();
    for event in structured_recall_events {
        let Some(recalled_at) = parse_timestamp(&event.recalled_at) else {
            continue;
        };
        for memory_id in &event.memory_ids {
            structured_recalled.push((memory_id.clone(), recalled_at));
        }
    }

    let mut used_structured = vec![false; structured_recalled.len()];
    let mut count = structured_recalled.len();

    for legacy in legacy_recalled_memories {
        let legacy_id = legacy.id.to_string();
        let matched =
            structured_recalled
                .iter()
                .enumerate()
                .any(|(index, (memory_id, recalled_at))| {
                    if used_structured[index] {
                        return false;
                    }
                    if memory_id == &legacy_id
                        && timestamps_match(*recalled_at, legacy.last_accessed)
                    {
                        used_structured[index] = true;
                        true
                    } else {
                        false
                    }
                });

        if !matched {
            count += 1;
        }
    }

    count
}

fn recall_effectiveness_by_event_id(
    store: &SqliteStore,
    recall_event_ids: &[String],
) -> HyphaeResult<HashMap<(String, String), f32>> {
    if recall_event_ids.is_empty() {
        return Ok(HashMap::new());
    }

    let placeholders: Vec<String> = (1..=recall_event_ids.len())
        .map(|i| format!("?{i}"))
        .collect();
    let sql = format!(
        "SELECT recall_event_id, memory_id, effectiveness
         FROM recall_effectiveness
         WHERE recall_event_id IN ({})",
        placeholders.join(",")
    );

    let mut stmt = store.conn.prepare_cached(&sql).map_err(|e| {
        hyphae_core::HyphaeError::Database(format!(
            "failed to prepare recall-effectiveness window query: {e}"
        ))
    })?;

    let rows = stmt
        .query_map(rusqlite::params_from_iter(recall_event_ids.iter()), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, f32>(2)?,
            ))
        })
        .map_err(|e| {
            hyphae_core::HyphaeError::Database(format!(
                "failed to query recall-effectiveness window rows: {e}"
            ))
        })?;

    let mut scores = HashMap::new();
    for row in rows {
        let (recall_event_id, memory_id, effectiveness) = row.map_err(|e| {
            hyphae_core::HyphaeError::Database(format!(
                "failed to read recall-effectiveness window row: {e}"
            ))
        })?;
        scores.insert((recall_event_id, memory_id), effectiveness);
    }

    Ok(scores)
}

fn recall_effectiveness_occurrences_in_window(
    legacy_recalled_memories: &[Memory],
    structured_recall_events: &[super::feedback::RecallEventRecord],
    effectiveness_by_event: &HashMap<(String, String), f32>,
) -> Vec<RecallEffectivenessOccurrence> {
    let mut structured_occurrences = Vec::new();
    for event in structured_recall_events {
        let recalled_at = parse_timestamp(&event.recalled_at);
        for memory_id in &event.memory_ids {
            structured_occurrences.push(RecallEffectivenessOccurrence {
                memory_id: memory_id.clone(),
                recalled_at,
                effectiveness: effectiveness_by_event
                    .get(&(event.id.clone(), memory_id.clone()))
                    .copied(),
            });
        }
    }

    let mut merged_occurrences = structured_occurrences.clone();
    let mut used_structured = vec![false; structured_occurrences.len()];

    for legacy in legacy_recalled_memories {
        let legacy_id = legacy.id.to_string();
        let matched = structured_occurrences
            .iter()
            .enumerate()
            .any(|(index, occurrence)| {
                if used_structured[index] {
                    return false;
                }

                let Some(recalled_at) = occurrence.recalled_at else {
                    return false;
                };

                if occurrence.memory_id == legacy_id
                    && timestamps_match(recalled_at, legacy.last_accessed)
                {
                    used_structured[index] = true;
                    true
                } else {
                    false
                }
            });

        if !matched {
            merged_occurrences.push(RecallEffectivenessOccurrence {
                memory_id: legacy_id,
                recalled_at: Some(legacy.last_accessed),
                effectiveness: None,
            });
        }
    }

    merged_occurrences
}

pub fn collect_recall_effectiveness_window(
    store: &SqliteStore,
    days_ago_start: i64,
    days_ago_end: i64,
    project: Option<&str>,
) -> HyphaeResult<RecallEffectivenessWindow> {
    let legacy_recalled_memories =
        legacy_recalled_memories_in_window(store, project, days_ago_start, days_ago_end)?;
    let structured_recall_events =
        structured_recall_events_in_window(store, project, days_ago_start, days_ago_end)?;
    let recall_event_ids: Vec<String> = structured_recall_events
        .iter()
        .map(|event| event.id.clone())
        .collect();
    let effectiveness_by_event = recall_effectiveness_by_event_id(store, &recall_event_ids)?;
    let scored_occurrences = recall_effectiveness_occurrences_in_window(
        &legacy_recalled_memories,
        &structured_recall_events,
        &effectiveness_by_event,
    );

    let recalled_memory_count = scored_occurrences.len();
    let scored_memory_count = scored_occurrences
        .iter()
        .filter(|occurrence| occurrence.effectiveness.is_some())
        .count();
    let non_zero_score_count = scored_occurrences
        .iter()
        .filter(|occurrence| {
            occurrence
                .effectiveness
                .is_some_and(|effectiveness| effectiveness.abs() > f32::EPSILON)
        })
        .count();
    let total_effectiveness = scored_occurrences
        .iter()
        .filter_map(|occurrence| occurrence.effectiveness.map(|effectiveness| effectiveness as f64))
        .sum();

    let mut top_by_memory: HashMap<String, (f64, usize)> = HashMap::new();
    for occurrence in &scored_occurrences {
        let Some(effectiveness) = occurrence.effectiveness else {
            continue;
        };
        let entry = top_by_memory
            .entry(occurrence.memory_id.clone())
            .or_insert((0.0, 0));
        entry.0 += effectiveness as f64;
        entry.1 += 1;
    }

    let mut scored_rows: Vec<RecallEffectivenessRow> = top_by_memory
        .into_iter()
        .map(|(memory_id, (total, count))| RecallEffectivenessRow {
            memory_id,
            effectiveness: (total / count as f64) as f32,
        })
        .collect();

    scored_rows.sort_by(|a, b| {
        b.effectiveness
            .partial_cmp(&a.effectiveness)
            .unwrap_or(Ordering::Equal)
            .then_with(|| a.memory_id.cmp(&b.memory_id))
    });

    Ok(RecallEffectivenessWindow {
        recalled_memory_count,
        scored_memory_count,
        non_zero_score_count,
        total_effectiveness,
        top_recalled_memories: scored_rows.into_iter().take(5).collect(),
    })
}

pub fn collect_evaluation_window(
    store: &SqliteStore,
    days_ago_start: i64,
    days_ago_end: i64,
    project: Option<&str>,
) -> HyphaeResult<EvaluationWindow> {
    let legacy_errors = get_memories_in_window(
        store,
        "errors/active",
        days_ago_start,
        days_ago_end,
        project,
    )?
    .len();
    let legacy_corrections =
        get_memories_in_window(store, "corrections", days_ago_start, days_ago_end, project)?;
    let legacy_resolved = get_memories_in_window(
        store,
        "errors/resolved",
        days_ago_start,
        days_ago_end,
        project,
    )?
    .len();
    let legacy_failed_tests =
        get_memories_in_window(store, "tests/failed", days_ago_start, days_ago_end, project)?.len();
    let legacy_resolved_tests = get_memories_in_window(
        store,
        "tests/resolved",
        days_ago_start,
        days_ago_end,
        project,
    )?
    .len();

    let structured_sessions =
        structured_sessions_in_window(store, project, days_ago_start, days_ago_end)?;
    let legacy_sessions =
        legacy_session_memories_in_window(store, project, days_ago_start, days_ago_end)?;
    let (session_count, total_session_length) =
        merge_session_counts(&structured_sessions, &legacy_sessions);

    let error_count =
        structured_signal_count(store, project, "tool_error", days_ago_start, days_ago_end)?
            .saturating_add(legacy_errors);
    let correction_count = merge_correction_counts(
        &legacy_corrections,
        &structured_correction_signals_in_window(store, project, days_ago_start, days_ago_end)?,
    );
    let resolved_count = structured_signal_count(
        store,
        project,
        "error_resolved",
        days_ago_start,
        days_ago_end,
    )?
    .saturating_add(legacy_resolved);
    let failed_test_count = legacy_failed_tests;
    let resolved_test_count =
        structured_signal_count(store, project, "test_passed", days_ago_start, days_ago_end)?
            .saturating_add(legacy_resolved_tests);

    let recalled_memory_count = merge_recalled_memory_counts(
        &legacy_recalled_memories_in_window(store, project, days_ago_start, days_ago_end)?,
        &structured_recall_events_in_window(store, project, days_ago_start, days_ago_end)?,
    );

    Ok(EvaluationWindow {
        error_count,
        correction_count,
        resolved_count,
        failed_test_count,
        resolved_test_count,
        total_session_length,
        session_count,
        recalled_memory_count,
    })
}

#[cfg(test)]
mod tests {
    use hyphae_core::Importance;
    use rusqlite::params;

    use super::*;

    fn test_store() -> SqliteStore {
        SqliteStore::in_memory().expect("in-memory store")
    }

    #[test]
    fn test_collect_evaluation_window_counts_duplicate_summaries_as_distinct_sessions() {
        let store = test_store();

        for _ in 0..2 {
            let (session_id, _) = store
                .session_start("demo-project", Some("same task"))
                .unwrap();
            store
                .session_end(&session_id, Some("same summary"), None, Some("0"))
                .unwrap();
        }

        let window = collect_evaluation_window(&store, 0, 1, Some("demo-project")).unwrap();

        assert_eq!(window.session_count, 2);
    }

    #[test]
    fn test_collect_evaluation_window_keeps_legacy_session_with_same_summary_when_not_mirrored() {
        let store = test_store();

        let (session_id, _) = store
            .session_start("demo-project", Some("same task"))
            .unwrap();
        store
            .session_end(&session_id, Some("same summary"), None, Some("0"))
            .unwrap();

        let mut legacy_session = Memory::builder(
            "session/demo-project".to_string(),
            "Session completed: same task. same summary".to_string(),
            Importance::Medium,
        )
        .project("demo-project".to_string())
        .build();
        let older = Utc::now() - Duration::days(1);
        legacy_session.created_at = older;
        legacy_session.updated_at = older;
        legacy_session.last_accessed = older;
        store.store(legacy_session).unwrap();

        let window = collect_evaluation_window(&store, 0, 2, Some("demo-project")).unwrap();

        assert_eq!(window.session_count, 2);
    }

    #[test]
    fn test_collect_evaluation_window_does_not_dedupe_cross_project_sessions() {
        let store = test_store();

        let (session_id, _) = store.session_start("project-a", Some("same task")).unwrap();
        store
            .session_end(&session_id, Some("same summary"), None, Some("0"))
            .unwrap();

        let legacy_session = Memory::builder(
            "session/project-b".to_string(),
            "Session completed: same task. same summary".to_string(),
            Importance::Medium,
        )
        .project("project-b".to_string())
        .build();
        store.store(legacy_session).unwrap();

        let window = collect_evaluation_window(&store, 0, 1, None).unwrap();

        assert_eq!(window.session_count, 2);
    }

    #[test]
    fn test_collect_evaluation_window_prefers_structured_corrections_over_legacy_mirrors() {
        let store = test_store();

        let (session_id, _) = store
            .session_start("demo-project", Some("session"))
            .unwrap();
        store
            .log_outcome_signal(
                Some(&session_id),
                "correction",
                -1,
                Some("cortina.post_tool_use"),
                Some("demo-project"),
            )
            .unwrap();
        store
            .session_end(&session_id, Some("done"), None, Some("0"))
            .unwrap();

        store
            .store(
                Memory::builder(
                    "corrections".to_string(),
                    "legacy correction".to_string(),
                    Importance::Medium,
                )
                .project("demo-project".to_string())
                .build(),
            )
            .unwrap();

        let window = collect_evaluation_window(&store, 0, 1, Some("demo-project")).unwrap();

        assert_eq!(window.correction_count, 1);
    }

    #[test]
    fn test_collect_evaluation_window_uses_legacy_corrections_without_structured_signals() {
        let store = test_store();

        store
            .store(
                Memory::builder(
                    "corrections".to_string(),
                    "legacy correction".to_string(),
                    Importance::Medium,
                )
                .project("demo-project".to_string())
                .build(),
            )
            .unwrap();

        let window = collect_evaluation_window(&store, 0, 1, Some("demo-project")).unwrap();

        assert_eq!(window.correction_count, 1);
    }

    #[test]
    fn test_collect_evaluation_window_keeps_distinct_legacy_corrections_in_mixed_mode() {
        let store = test_store();

        let (session_id, _) = store
            .session_start("demo-project", Some("session"))
            .unwrap();
        store
            .log_outcome_signal(
                Some(&session_id),
                "correction",
                -1,
                Some("cortina.post_tool_use"),
                Some("demo-project"),
            )
            .unwrap();
        store
            .session_end(&session_id, Some("done"), None, Some("0"))
            .unwrap();

        let mut legacy_correction = Memory::builder(
            "corrections".to_string(),
            "legacy correction".to_string(),
            Importance::Medium,
        )
        .project("demo-project".to_string())
        .build();
        let older = Utc::now() - Duration::days(1);
        legacy_correction.created_at = older;
        legacy_correction.updated_at = older;
        legacy_correction.last_accessed = older;
        store.store(legacy_correction).unwrap();

        let window = collect_evaluation_window(&store, 0, 2, Some("demo-project")).unwrap();

        assert_eq!(window.correction_count, 2);
    }

    #[test]
    fn test_collect_evaluation_window_does_not_dedupe_cross_project_corrections() {
        let store = test_store();

        let (session_id, _) = store.session_start("project-a", Some("session")).unwrap();
        store
            .log_outcome_signal(
                Some(&session_id),
                "correction",
                -1,
                Some("cortina.post_tool_use"),
                Some("project-a"),
            )
            .unwrap();
        store
            .session_end(&session_id, Some("done"), None, Some("0"))
            .unwrap();

        let legacy_correction = Memory::builder(
            "corrections".to_string(),
            "legacy correction".to_string(),
            Importance::Medium,
        )
        .project("project-b".to_string())
        .build();
        store.store(legacy_correction).unwrap();

        let window = collect_evaluation_window(&store, 0, 1, None).unwrap();

        assert_eq!(window.correction_count, 2);
    }

    #[test]
    fn test_collect_evaluation_window_dedupes_structured_recall_mirrors() {
        let store = test_store();

        let memory = Memory::builder(
            "context/demo-project".to_string(),
            "recalled memory".to_string(),
            Importance::Medium,
        )
        .project("demo-project".to_string())
        .build();
        let memory_id = store.store(memory).unwrap();
        let now = Utc::now().to_rfc3339();
        store
            .conn
            .execute(
                "UPDATE memories SET access_count = 1, last_accessed = ?1 WHERE id = ?2",
                params![now, memory_id.to_string()],
            )
            .unwrap();
        store
            .log_recall_event(
                None,
                "structured recall",
                &[memory_id.to_string()],
                Some("demo-project"),
            )
            .unwrap();

        let window = collect_evaluation_window(&store, 0, 1, Some("demo-project")).unwrap();

        assert_eq!(window.recalled_memory_count, 1);
    }

    #[test]
    fn test_collect_evaluation_window_keeps_distinct_legacy_recalls_in_mixed_mode() {
        let store = test_store();

        let current_memory = Memory::builder(
            "context/demo-project".to_string(),
            "current recall".to_string(),
            Importance::Medium,
        )
        .project("demo-project".to_string())
        .build();
        let current_memory_id = store.store(current_memory).unwrap();
        store
            .log_recall_event(
                None,
                "structured recall",
                &[current_memory_id.to_string()],
                Some("demo-project"),
            )
            .unwrap();

        let mut legacy_memory = Memory::builder(
            "context/demo-project".to_string(),
            "legacy recall".to_string(),
            Importance::Medium,
        )
        .project("demo-project".to_string())
        .build();
        let older = Utc::now() - Duration::days(1);
        legacy_memory.created_at = older;
        legacy_memory.updated_at = older;
        legacy_memory.last_accessed = older;
        legacy_memory.access_count = 1;
        store.store(legacy_memory).unwrap();

        let window = collect_evaluation_window(&store, 0, 2, Some("demo-project")).unwrap();

        assert_eq!(window.recalled_memory_count, 2);
    }

    #[test]
    fn test_collect_recall_effectiveness_window_reports_non_zero_scores_and_top_entries() {
        let store = test_store();
        let (session_id, _) = store
            .session_start("demo-project", Some("evaluate structured path"))
            .unwrap();

        let scored = Memory::builder(
            "context/demo-project".to_string(),
            "high value recall".to_string(),
            Importance::Medium,
        )
        .project("demo-project".to_string())
        .build();
        let scored_id = scored.id.to_string();
        store.store(scored).unwrap();

        store
            .log_recall_event(
                Some(&session_id),
                "high value recall",
                &[scored_id.clone()],
                Some("demo-project"),
            )
            .unwrap();
        store
            .log_outcome_signal(
                Some(&session_id),
                "test_passed",
                2,
                Some("manual"),
                Some("demo-project"),
            )
            .unwrap();
        store
            .log_outcome_signal(
                Some(&session_id),
                "build_passed",
                2,
                Some("manual"),
                Some("demo-project"),
            )
            .unwrap();

        let zero = Memory::builder(
            "context/demo-project".to_string(),
            "quiet recall".to_string(),
            Importance::Medium,
        )
        .project("demo-project".to_string())
        .build();
        let zero_id = zero.id.to_string();
        store.store(zero).unwrap();
        store
            .log_recall_event(
                Some(&session_id),
                "quiet recall",
                &[zero_id.clone()],
                Some("demo-project"),
            )
            .unwrap();

        store
            .session_end(&session_id, Some("done"), None, Some("0"))
            .unwrap();

        let report =
            collect_recall_effectiveness_window(&store, 0, 1, Some("demo-project")).unwrap();

        assert_eq!(report.recalled_memory_count, 2);
        assert_eq!(report.scored_memory_count, 2);
        assert_eq!(report.non_zero_score_count, 1);
        assert!(report.non_zero_score_fraction() > 0.0);
        assert!(report.average_effectiveness() > 0.0);
        assert_eq!(report.top_recalled_memories.len(), 2);
        assert_eq!(report.top_recalled_memories[0].memory_id, scored_id);
        assert!(
            report.top_recalled_memories[0].effectiveness
                >= report.top_recalled_memories[1].effectiveness
        );
    }

    #[test]
    fn test_collect_recall_effectiveness_window_ignores_scores_outside_window() {
        let store = test_store();

        let memory = Memory::builder(
            "context/demo-project".to_string(),
            "recalled memory".to_string(),
            Importance::Medium,
        )
        .project("demo-project".to_string())
        .build();
        let memory_id = memory.id.to_string();
        store.store(memory).unwrap();

        let (stale_session_id, _) = store
            .session_start("demo-project", Some("stale scored recall"))
            .unwrap();
        store
            .log_recall_event(
                Some(&stale_session_id),
                "stale scored recall",
                std::slice::from_ref(&memory_id),
                Some("demo-project"),
            )
            .unwrap();
        store
            .log_outcome_signal(
                Some(&stale_session_id),
                "test_passed",
                2,
                Some("manual"),
                Some("demo-project"),
            )
            .unwrap();
        store
            .log_outcome_signal(
                Some(&stale_session_id),
                "build_passed",
                2,
                Some("manual"),
                Some("demo-project"),
            )
            .unwrap();
        store
            .session_end(&stale_session_id, Some("done"), None, Some("0"))
            .unwrap();

        let stale_timestamp = (Utc::now() - Duration::days(10)).to_rfc3339();
        store
            .conn
            .execute(
                "UPDATE recall_events
                 SET recalled_at = ?1
                 WHERE session_id = ?2",
                params![stale_timestamp, stale_session_id],
            )
            .unwrap();
        store
            .conn
            .execute(
                "UPDATE recall_effectiveness
                 SET computed_at = ?1
                 WHERE recall_event_id IN (
                     SELECT id FROM recall_events WHERE session_id = ?2
                 )",
                params![stale_timestamp, stale_session_id],
            )
            .unwrap();

        let (current_session_id, _) = store
            .session_start("demo-project", Some("fresh unscored recall"))
            .unwrap();
        store
            .log_recall_event(
                Some(&current_session_id),
                "fresh unscored recall",
                std::slice::from_ref(&memory_id),
                Some("demo-project"),
            )
            .unwrap();

        let report =
            collect_recall_effectiveness_window(&store, 0, 1, Some("demo-project")).unwrap();

        assert_eq!(report.recalled_memory_count, 1);
        assert_eq!(report.scored_memory_count, 0);
        assert_eq!(report.non_zero_score_count, 0);
        assert_eq!(report.average_effectiveness(), 0.0);
        assert!(report.top_recalled_memories.is_empty());
    }

    #[test]
    fn test_collect_recall_effectiveness_window_weights_repeated_recalls_by_occurrence() {
        let store = test_store();

        let repeated = Memory::builder(
            "context/demo-project".to_string(),
            "repeated recall".to_string(),
            Importance::Medium,
        )
        .project("demo-project".to_string())
        .build();
        let repeated_id = repeated.id.to_string();
        store.store(repeated).unwrap();

        let quiet = Memory::builder(
            "context/demo-project".to_string(),
            "quiet recall".to_string(),
            Importance::Medium,
        )
        .project("demo-project".to_string())
        .build();
        let quiet_id = quiet.id.to_string();
        store.store(quiet).unwrap();

        let first_event = store
            .log_recall_event(
                None,
                "repeat-1",
                std::slice::from_ref(&repeated_id),
                Some("demo-project"),
            )
            .unwrap();
        let second_event = store
            .log_recall_event(
                None,
                "repeat-2",
                std::slice::from_ref(&repeated_id),
                Some("demo-project"),
            )
            .unwrap();
        let _quiet_event = store
            .log_recall_event(
                None,
                "quiet",
                std::slice::from_ref(&quiet_id),
                Some("demo-project"),
            )
            .unwrap();

        let computed_at = Utc::now().to_rfc3339();
        for recall_event_id in [&first_event, &second_event] {
            store
                .conn
                .execute(
                    "INSERT INTO recall_effectiveness
                        (memory_id, recall_event_id, effectiveness, signal_count, computed_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![repeated_id, recall_event_id, 0.5_f32, 2_i64, computed_at],
                )
                .unwrap();
        }

        let report =
            collect_recall_effectiveness_window(&store, 0, 1, Some("demo-project")).unwrap();

        assert_eq!(report.recalled_memory_count, 3);
        assert_eq!(report.scored_memory_count, 2);
        assert_eq!(report.non_zero_score_count, 2);
        assert!((report.non_zero_score_fraction() - (2.0 / 3.0)).abs() < 1e-6);
        assert!((report.average_effectiveness() - 0.5).abs() < 1e-6);
        assert_eq!(report.top_recalled_memories[0].memory_id, repeated_id);
        assert_eq!(report.top_recalled_memories[0].effectiveness, 0.5);
    }
}
