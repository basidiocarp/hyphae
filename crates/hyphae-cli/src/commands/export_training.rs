use anyhow::Result;
use chrono::{DateTime, Duration, Utc};
use hyphae_core::MemoryStore;
use hyphae_store::{Session, SqliteStore};
use std::path::PathBuf;

const SESSION_MIRROR_TOLERANCE_MINUTES: i64 = 5;

#[derive(Debug, Clone, Copy)]
pub enum TrainingFormat {
    Sft,
    Dpo,
    Alpaca,
}

impl std::str::FromStr for TrainingFormat {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "sft" => Ok(TrainingFormat::Sft),
            "dpo" => Ok(TrainingFormat::Dpo),
            "alpaca" => Ok(TrainingFormat::Alpaca),
            _ => Err(format!("unknown format: {s}")),
        }
    }
}

impl std::fmt::Display for TrainingFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TrainingFormat::Sft => write!(f, "sft"),
            TrainingFormat::Dpo => write!(f, "dpo"),
            TrainingFormat::Alpaca => write!(f, "alpaca"),
        }
    }
}

#[derive(serde::Serialize)]
struct SftRecord {
    instruction: String,
    response: String,
}

#[derive(serde::Serialize)]
struct DpoRecord {
    prompt: String,
    chosen: String,
    rejected: String,
}

#[derive(serde::Serialize)]
struct AlpacaRecord {
    instruction: String,
    input: String,
    output: String,
}

#[derive(Debug, Clone)]
struct TrainingExample {
    topic: String,
    summary: String,
    weight: f32,
}

fn default_memory_topics(store: &SqliteStore, project: Option<&str>) -> Result<Vec<String>> {
    let topics = store.list_topics(project)?;
    let mut selected = Vec::new();

    for (topic, _) in topics {
        let include = topic == "errors/resolved"
            || topic == "corrections"
            || topic.starts_with("decisions/")
            || topic.starts_with("context/");
        if include {
            selected.push(topic);
        }
    }

    selected.sort();
    selected.dedup();
    Ok(selected)
}

fn session_memory_topics(store: &SqliteStore, project: Option<&str>) -> Result<Vec<String>> {
    let topics = store.list_topics(project)?;
    let mut selected: Vec<String> = topics
        .into_iter()
        .filter_map(|(topic, _)| topic.starts_with("session/").then_some(topic))
        .collect();
    selected.sort();
    selected.dedup();
    Ok(selected)
}

fn structured_sessions_for_export(
    store: &SqliteStore,
    project: Option<&str>,
) -> Result<Vec<Session>> {
    Ok(if let Some(project) = project {
        store.session_context(project, 10_000)?
    } else {
        store.session_context_between(
            None,
            None,
            "0000-01-01T00:00:00Z",
            "9999-12-31T23:59:59Z",
            10_000,
        )?
    })
}

fn structured_session_example(session: Session) -> Option<TrainingExample> {
    let summary = session.summary?;
    let topic = format!("session/{}", session.project);
    let text = match session.task {
        Some(task) => format!("Session completed: {task}. {summary}"),
        None => format!("Session completed. {summary}"),
    };
    Some(TrainingExample {
        topic,
        summary: text,
        weight: 1.0,
    })
}

fn parse_timestamp(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|dt| dt.with_timezone(&Utc))
}

fn timestamps_match(a: DateTime<Utc>, b: DateTime<Utc>) -> bool {
    (a - b).num_seconds().abs() <= Duration::minutes(SESSION_MIRROR_TOLERANCE_MINUTES).num_seconds()
}

fn memory_examples_for_topics(
    store: &SqliteStore,
    topics: &[String],
    project: Option<&str>,
) -> Vec<TrainingExample> {
    let mut examples = Vec::new();

    for topic in topics {
        match store.get_by_topic(topic, project) {
            Ok(memories) => {
                examples.extend(memories.into_iter().map(|mem| TrainingExample {
                    topic: mem.topic,
                    summary: mem.summary,
                    weight: mem.weight.value(),
                }));
            }
            Err(e) => tracing::warn!("failed to read topic {}: {}", topic, e),
        }
    }

    examples
}

fn merged_session_examples(
    store: &SqliteStore,
    project: Option<&str>,
) -> Result<Vec<TrainingExample>> {
    let structured_sessions = structured_sessions_for_export(store, project)?;
    let session_topics = session_memory_topics(store, project)?;
    let mut legacy_sessions = Vec::new();
    for topic in &session_topics {
        match store.get_by_topic(topic, project) {
            Ok(memories) => legacy_sessions.extend(memories),
            Err(e) => tracing::warn!("failed to read topic {}: {}", topic, e),
        }
    }

    let mut structured_session_times = std::collections::HashMap::new();
    let mut merged = Vec::new();
    for session in structured_sessions {
        if let Some(example) = structured_session_example(session.clone()) {
            if let Some(session_time) = session
                .ended_at
                .as_deref()
                .and_then(parse_timestamp)
                .or_else(|| parse_timestamp(&session.started_at))
            {
                structured_session_times
                    .entry((example.topic.clone(), example.summary.clone()))
                    .or_insert_with(Vec::new)
                    .push(session_time);
            }
            merged.push(example);
        }
    }

    for legacy in legacy_sessions {
        match structured_session_times.get_mut(&(legacy.topic.clone(), legacy.summary.clone())) {
            Some(candidates) => {
                if let Some(index) = candidates
                    .iter()
                    .position(|candidate| timestamps_match(*candidate, legacy.created_at))
                {
                    candidates.swap_remove(index);
                } else {
                    merged.push(TrainingExample {
                        topic: legacy.topic,
                        summary: legacy.summary,
                        weight: legacy.weight.value(),
                    });
                }
            }
            _ => {
                merged.push(TrainingExample {
                    topic: legacy.topic,
                    summary: legacy.summary,
                    weight: legacy.weight.value(),
                });
            }
        }
    }

    Ok(merged)
}

fn collect_training_examples(
    store: &SqliteStore,
    topic: Option<String>,
    project: Option<&str>,
) -> Result<Vec<TrainingExample>> {
    if let Some(topic) = topic {
        if topic.starts_with("session/") {
            return Ok(merged_session_examples(store, project)?
                .into_iter()
                .filter(|example| example.topic == topic)
                .collect());
        }
        return Ok(memory_examples_for_topics(store, &[topic], project));
    }

    let topics = default_memory_topics(store, project)?;
    let mut examples = memory_examples_for_topics(store, &topics, project);
    examples.extend(merged_session_examples(store, project)?);
    Ok(examples)
}

/// ─────────────────────────────────────────────────────────────────────────
/// Export Memories as Training Data
/// ─────────────────────────────────────────────────────────────────────────
pub(crate) fn cmd_export_training(
    store: &SqliteStore,
    format: TrainingFormat,
    topic: Option<String>,
    min_weight: Option<f32>,
    output: Option<PathBuf>,
    project: Option<String>,
) -> Result<()> {
    let min_weight = min_weight.unwrap_or(0.0);
    let project_ref = project.as_deref();
    let examples = collect_training_examples(store, topic, project_ref)?;
    use std::io::Write;
    let mut handle: Box<dyn Write> = if let Some(path) = output {
        Box::new(std::fs::File::create(path)?)
    } else {
        Box::new(std::io::stdout().lock())
    };

    for example in examples {
        if example.weight < min_weight {
            continue;
        }

        match format {
            TrainingFormat::Sft => {
                let instruction = format!("What is our convention for: {}", example.topic);
                let response = example.summary.clone();
                let record = SftRecord {
                    instruction,
                    response,
                };
                writeln!(handle, "{}", serde_json::to_string(&record)?)?;
            }
            TrainingFormat::Dpo => {
                if example.topic == "corrections" || example.summary.contains("Original:") {
                    if let Some((rejected, chosen)) = parse_correction(&example.summary) {
                        let prompt = format!("Fix the code: {}", example.topic);
                        let record = DpoRecord {
                            prompt,
                            chosen,
                            rejected,
                        };
                        writeln!(handle, "{}", serde_json::to_string(&record)?)?;
                    }
                }
            }
            TrainingFormat::Alpaca => {
                let instruction = format!("Topic: {}", example.topic);
                let output = example.summary.clone();
                let record = AlpacaRecord {
                    instruction,
                    input: String::new(),
                    output,
                };
                writeln!(handle, "{}", serde_json::to_string(&record)?)?;
            }
        }
    }

    Ok(())
}

/// ─────────────────────────────────────────────────────────────────────────
/// Parse Corrections
/// ─────────────────────────────────────────────────────────────────────────
fn parse_correction(text: &str) -> Option<(String, String)> {
    let lines: Vec<&str> = text.lines().collect();

    let mut original: Option<String> = None;
    let mut correction: Option<String> = None;

    for line in lines {
        if line.starts_with("Original:") {
            original = Some(
                line.strip_prefix("Original:")
                    .unwrap_or("")
                    .trim()
                    .to_string(),
            );
        } else if line.starts_with("Correction:") {
            correction = Some(
                line.strip_prefix("Correction:")
                    .unwrap_or("")
                    .trim()
                    .to_string(),
            );
        }
    }

    match (original, correction) {
        (Some(o), Some(c)) if !o.is_empty() && !c.is_empty() => Some((o, c)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyphae_core::{Importance, Memory};
    use tempfile::TempDir;

    fn test_store() -> SqliteStore {
        SqliteStore::in_memory().expect("in-memory store")
    }

    #[test]
    fn test_collect_training_examples_includes_project_scoped_topics_and_structured_sessions() {
        let store = test_store();

        let decision = Memory::builder(
            "decisions/myapp".to_string(),
            "Use gRPC for internal services".to_string(),
            Importance::High,
        )
        .project("myapp".to_string())
        .build();
        store.store(decision).unwrap();

        let (session_id, _) = store
            .session_start("myapp", Some("refactor auth middleware"))
            .unwrap();
        store
            .session_end(
                &session_id,
                Some("Extracted JWT validation"),
                None,
                Some("0"),
            )
            .unwrap();

        let examples = collect_training_examples(&store, None, Some("myapp")).unwrap();
        let topics: Vec<String> = examples
            .iter()
            .map(|example| example.topic.clone())
            .collect();

        assert!(topics.contains(&"decisions/myapp".to_string()));
        assert!(topics.contains(&"session/myapp".to_string()));
    }

    #[test]
    fn test_collect_training_examples_without_project_keeps_structured_sessions() {
        let store = test_store();

        let (session_id, _) = store
            .session_start("myapp", Some("refactor auth middleware"))
            .unwrap();
        store
            .session_end(
                &session_id,
                Some("Extracted JWT validation"),
                None,
                Some("0"),
            )
            .unwrap();

        let examples = collect_training_examples(&store, None, None).unwrap();

        assert!(
            examples
                .iter()
                .any(|example| example.topic == "session/myapp")
        );
    }

    #[test]
    fn test_collect_training_examples_without_project_falls_back_to_session_memories() {
        let store = test_store();
        let session_memory = Memory::builder(
            "session/myapp".to_string(),
            "Session completed. legacy summary".to_string(),
            Importance::Medium,
        )
        .project("myapp".to_string())
        .build();
        store.store(session_memory).unwrap();

        let examples = collect_training_examples(&store, None, None).unwrap();

        assert!(
            examples
                .iter()
                .any(|example| example.topic == "session/myapp")
        );
    }

    #[test]
    fn test_collect_training_examples_keeps_legacy_session_examples_in_mixed_mode() {
        let store = test_store();

        let (session_id, _) = store
            .session_start("myapp", Some("refactor auth middleware"))
            .unwrap();
        store
            .session_end(
                &session_id,
                Some("Extracted JWT validation"),
                None,
                Some("0"),
            )
            .unwrap();

        let legacy_session = Memory::builder(
            "session/myapp".to_string(),
            "Session completed. legacy-only summary".to_string(),
            Importance::Medium,
        )
        .project("myapp".to_string())
        .build();
        store.store(legacy_session).unwrap();

        let examples = collect_training_examples(&store, None, Some("myapp")).unwrap();

        assert!(
            examples
                .iter()
                .any(|example| example.summary == "Session completed. legacy-only summary")
        );
        assert!(
            examples
                .iter()
                .any(|example| example.summary.contains("Extracted JWT validation"))
        );
    }

    #[test]
    fn test_collect_training_examples_with_explicit_session_topic_uses_structured_sessions() {
        let store = test_store();

        let (session_id, _) = store
            .session_start("myapp", Some("refactor auth middleware"))
            .unwrap();
        store
            .session_end(
                &session_id,
                Some("Extracted JWT validation"),
                None,
                Some("0"),
            )
            .unwrap();

        let examples =
            collect_training_examples(&store, Some("session/myapp".to_string()), Some("myapp"))
                .unwrap();

        assert_eq!(examples.len(), 1);
        assert!(examples[0].summary.contains("Extracted JWT validation"));
    }

    #[test]
    fn test_collect_training_examples_does_not_dedupe_cross_project_sessions() {
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

        let examples = collect_training_examples(&store, None, None).unwrap();

        assert!(
            examples
                .iter()
                .any(|example| example.topic == "session/project-a")
        );
        assert!(
            examples
                .iter()
                .any(|example| example.topic == "session/project-b")
        );
    }

    #[test]
    fn test_cmd_export_training_writes_to_file() {
        let store = test_store();
        let dir = TempDir::new().unwrap();
        let output = dir.path().join("training.jsonl");

        let decision = Memory::builder(
            "decisions/myapp".to_string(),
            "Use gRPC for internal services".to_string(),
            Importance::High,
        )
        .project("myapp".to_string())
        .build();
        store.store(decision).unwrap();

        cmd_export_training(
            &store,
            TrainingFormat::Sft,
            None,
            None,
            Some(output.clone()),
            Some("myapp".to_string()),
        )
        .unwrap();

        let content = std::fs::read_to_string(output).unwrap();
        assert!(content.contains("\"instruction\""));
        assert!(content.contains("Use gRPC for internal services"));
    }
}
