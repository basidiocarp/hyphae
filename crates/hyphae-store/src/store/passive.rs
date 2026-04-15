use chrono::Utc;
use serde::Serialize;
use serde_json::Value;

use hyphae_core::{MemoirStore, MemoryStore, SCOPED_IDENTITY_SCHEMA_VERSION, ScopedIdentity};

use super::SqliteStore;

const COUNCIL_TOPIC: &str = "session/council-lifecycle";
const DEFAULT_SESSION_LIMIT: i64 = 5;
const DEFAULT_MEMORY_LIMIT: usize = 5;
const DEFAULT_CONCEPT_LIMIT: usize = 8;

#[derive(Debug, Clone, Serialize)]
pub struct CompactSummaryArtifact {
    pub artifact_type: &'static str,
    pub session_id: String,
    pub project: String,
    pub task: Option<String>,
    pub summary: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct PassiveMemoryItem {
    pub topic: String,
    pub summary: String,
    pub importance: String,
    pub updated_at: String,
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectUnderstandingConcept {
    pub name: String,
    pub definition: String,
    pub labels: Vec<String>,
    pub confidence: f32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProjectUnderstandingBundle {
    pub artifact_type: &'static str,
    pub project: String,
    pub memoir_name: String,
    pub generated_at: String,
    pub total_concepts: usize,
    pub exported_concepts: usize,
    pub concepts: Vec<ProjectUnderstandingConcept>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CouncilArtifact {
    pub artifact_type: &'static str,
    pub topic: String,
    pub session_id: Option<String>,
    pub event_name: String,
    pub summary: String,
    pub host: Option<String>,
    pub status: Option<String>,
    pub prompt_excerpt: Option<String>,
    pub transcript_path: Option<String>,
    pub updated_at: String,
    pub importance: String,
    pub keywords: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PassiveContextBundle {
    pub schema_version: &'static str,
    pub scoped_identity: ScopedIdentity,
    pub project: Option<String>,
    pub generated_at: String,
    pub compact_summaries: Vec<CompactSummaryArtifact>,
    pub council_artifacts: Vec<CouncilArtifact>,
    pub project_context: Vec<PassiveMemoryItem>,
    pub decisions: Vec<PassiveMemoryItem>,
    pub understanding: Option<ProjectUnderstandingBundle>,
}

impl SqliteStore {
    pub fn list_compact_summary_artifacts(
        &self,
        project: Option<&str>,
        limit: usize,
    ) -> hyphae_core::HyphaeResult<Vec<CompactSummaryArtifact>> {
        let sessions = match project {
            Some(project) => self.session_context(project, limit as i64)?,
            None => self.session_context_all(limit as i64)?,
        };

        Ok(sessions
            .into_iter()
            .filter_map(|session| {
                session.summary.map(|summary| CompactSummaryArtifact {
                    artifact_type: "compact_summary",
                    session_id: session.id,
                    project: session.project,
                    task: session.task,
                    summary,
                    started_at: session.started_at,
                    ended_at: session.ended_at,
                    status: session.status,
                })
            })
            .collect())
    }

    pub fn project_understanding_bundle(
        &self,
        project: &str,
        concept_limit: usize,
    ) -> hyphae_core::HyphaeResult<Option<ProjectUnderstandingBundle>> {
        let memoir_name = format!("code:{project}");
        let Some(memoir) = self.get_memoir_by_name(&memoir_name)? else {
            return Ok(None);
        };

        let concepts = self.list_concepts(&memoir.id)?;
        let total_concepts = concepts.len();
        let exported_concepts: Vec<ProjectUnderstandingConcept> = concepts
            .into_iter()
            .take(concept_limit)
            .map(|concept| ProjectUnderstandingConcept {
                name: concept.name,
                definition: concept.definition,
                labels: concept
                    .labels
                    .into_iter()
                    .map(|label| label.to_string())
                    .collect(),
                confidence: concept.confidence.value(),
            })
            .collect();

        Ok(Some(ProjectUnderstandingBundle {
            artifact_type: "project_understanding",
            project: project.to_string(),
            memoir_name,
            generated_at: Utc::now().to_rfc3339(),
            total_concepts,
            exported_concepts: exported_concepts.len(),
            concepts: exported_concepts,
        }))
    }

    pub fn list_council_artifacts(
        &self,
        project: Option<&str>,
        limit: usize,
    ) -> hyphae_core::HyphaeResult<Vec<CouncilArtifact>> {
        let memories = self.get_by_topic(COUNCIL_TOPIC, project)?;

        Ok(memories
            .into_iter()
            .take(limit)
            .map(|memory| {
                let parsed = serde_json::from_str::<Value>(&memory.summary).ok();
                let metadata = parsed
                    .as_ref()
                    .and_then(|value| value.get("metadata"))
                    .and_then(Value::as_object);

                CouncilArtifact {
                    artifact_type: "council_lifecycle",
                    topic: memory.topic,
                    session_id: parsed
                        .as_ref()
                        .and_then(|value| value.get("session_id"))
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    event_name: parsed
                        .as_ref()
                        .and_then(|value| value.get("event_name"))
                        .and_then(Value::as_str)
                        .unwrap_or("council_lifecycle")
                        .to_string(),
                    summary: parsed
                        .as_ref()
                        .and_then(|value| value.get("summary"))
                        .and_then(Value::as_str)
                        .unwrap_or(&memory.summary)
                        .to_string(),
                    host: parsed
                        .as_ref()
                        .and_then(|value| value.get("host"))
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    status: parsed
                        .as_ref()
                        .and_then(|value| value.get("status"))
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    prompt_excerpt: metadata
                        .and_then(|meta| meta.get("prompt_excerpt"))
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    transcript_path: metadata
                        .and_then(|meta| meta.get("transcript_path"))
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned),
                    updated_at: memory.updated_at.to_rfc3339(),
                    importance: memory.importance.to_string(),
                    keywords: memory.keywords,
                }
            })
            .collect())
    }

    pub fn passive_context_bundle(
        &self,
        project: Option<&str>,
    ) -> hyphae_core::HyphaeResult<PassiveContextBundle> {
        let compact_summaries =
            self.list_compact_summary_artifacts(project, DEFAULT_SESSION_LIMIT as usize)?;

        let (council_artifacts, project_context, decisions, understanding) = match project {
            Some(project) => {
                let council_artifacts =
                    self.list_council_artifacts(Some(project), DEFAULT_MEMORY_LIMIT)?;
                let project_context = self
                    .get_by_topic(&format!("context/{project}"), Some(project))?
                    .into_iter()
                    .take(DEFAULT_MEMORY_LIMIT)
                    .map(|memory| PassiveMemoryItem {
                        topic: memory.topic,
                        summary: memory.summary,
                        importance: memory.importance.to_string(),
                        updated_at: memory.updated_at.to_rfc3339(),
                        keywords: memory.keywords,
                    })
                    .collect();

                let decisions = self
                    .get_by_topic(&format!("decisions/{project}"), Some(project))?
                    .into_iter()
                    .take(DEFAULT_MEMORY_LIMIT)
                    .map(|memory| PassiveMemoryItem {
                        topic: memory.topic,
                        summary: memory.summary,
                        importance: memory.importance.to_string(),
                        updated_at: memory.updated_at.to_rfc3339(),
                        keywords: memory.keywords,
                    })
                    .collect();

                let understanding =
                    self.project_understanding_bundle(project, DEFAULT_CONCEPT_LIMIT)?;
                (council_artifacts, project_context, decisions, understanding)
            }
            None => (Vec::new(), Vec::new(), Vec::new(), None),
        };

        Ok(PassiveContextBundle {
            schema_version: SCOPED_IDENTITY_SCHEMA_VERSION,
            scoped_identity: ScopedIdentity::from_project(project),
            project: project.map(ToOwned::to_owned),
            generated_at: Utc::now().to_rfc3339(),
            compact_summaries,
            council_artifacts,
            project_context,
            decisions,
            understanding,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyphae_core::{Concept, Importance, Memoir, Memory, MemoryStore};

    fn test_store() -> SqliteStore {
        SqliteStore::in_memory().unwrap()
    }

    fn make_memory(topic: &str, summary: &str, project: &str) -> Memory {
        Memory::builder(topic.into(), summary.into(), Importance::Medium)
            .project(project.to_string())
            .build()
    }

    #[test]
    fn test_list_compact_summary_artifacts_uses_structured_sessions() {
        let store = test_store();
        let (session_id, _) = store.session_start("demo", Some("task")).unwrap();
        store
            .session_end(&session_id, Some("compact summary"), None, Some("0"))
            .unwrap();

        let artifacts = store
            .list_compact_summary_artifacts(Some("demo"), 5)
            .unwrap();
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].artifact_type, "compact_summary");
        assert_eq!(artifacts[0].summary, "compact summary");
    }

    #[test]
    fn test_project_understanding_bundle_reads_code_memoir() {
        let store = test_store();
        let memoir = Memoir::new("code:demo".into(), "demo memoir".into());
        let memoir_id = store.create_memoir(memoir).unwrap();
        let concept = Concept::new(memoir_id, "WidgetService".into(), "Handles widgets".into());
        store.add_concept(concept).unwrap();

        let bundle = store
            .project_understanding_bundle("demo", 8)
            .unwrap()
            .expect("bundle");
        assert_eq!(bundle.artifact_type, "project_understanding");
        assert_eq!(bundle.project, "demo");
        assert_eq!(bundle.total_concepts, 1);
        assert_eq!(bundle.concepts[0].name, "WidgetService");
    }

    #[test]
    fn test_list_council_artifacts_parses_normalized_payloads() {
        let store = test_store();
        let council_memory = Memory::builder(
            COUNCIL_TOPIC.to_string(),
            r#"{"session_id":"ses_123","event_name":"user_prompt_submit","summary":"council lifecycle captured from prompt","host":"claude_code","status":"captured","metadata":{"prompt_excerpt":"/council review this task","transcript_path":"/tmp/transcript.jsonl"}}"#.to_string(),
            Importance::High,
        )
        .project("demo".to_string())
        .build();
        store.store(council_memory).unwrap();

        let artifacts = store.list_council_artifacts(Some("demo"), 5).unwrap();
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].artifact_type, "council_lifecycle");
        assert_eq!(artifacts[0].session_id.as_deref(), Some("ses_123"));
        assert_eq!(
            artifacts[0].prompt_excerpt.as_deref(),
            Some("/council review this task")
        );
    }

    #[test]
    fn test_passive_context_bundle_combines_sessions_memories_and_understanding() {
        let store = test_store();
        let (session_id, _) = store.session_start("demo", Some("task")).unwrap();
        store
            .session_end(&session_id, Some("compact summary"), None, Some("0"))
            .unwrap();

        store
            .store(make_memory("context/demo", "context note", "demo"))
            .unwrap();
        store
            .store(make_memory("decisions/demo", "decision note", "demo"))
            .unwrap();
        store
            .store(
                Memory::builder(
                    COUNCIL_TOPIC.to_string(),
                    r#"{"session_id":"ses_123","event_name":"user_prompt_submit","summary":"council lifecycle captured from prompt"}"#.to_string(),
                    Importance::High,
                )
                .project("demo".to_string())
                .build(),
            )
            .unwrap();

        let memoir = Memoir::new("code:demo".into(), "demo memoir".into());
        let memoir_id = store.create_memoir(memoir).unwrap();
        let concept = Concept::new(memoir_id, "WidgetService".into(), "Handles widgets".into());
        store.add_concept(concept).unwrap();

        let bundle = store.passive_context_bundle(Some("demo")).unwrap();
        assert_eq!(bundle.compact_summaries.len(), 1);
        assert_eq!(bundle.council_artifacts.len(), 1);
        assert_eq!(bundle.project_context.len(), 1);
        assert_eq!(bundle.decisions.len(), 1);
        assert!(bundle.understanding.is_some());
    }
}
