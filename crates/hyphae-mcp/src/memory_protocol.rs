use hyphae_core::{SCOPED_IDENTITY_SCHEMA_VERSION, ScopedIdentity};
use serde::Serialize;

pub const PROTOCOL_RESOURCE_URI: &str = "hyphae://protocol/current";

#[derive(Debug, Clone, Serialize)]
pub struct MemoryProtocolSurface {
    pub schema_version: &'static str,
    pub artifact_type: &'static str,
    pub scoped_identity: ScopedIdentity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    pub summary: &'static str,
    pub recall: RecallPhase,
    pub store: StorePhase,
    pub resources: Vec<ProtocolResource>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RecallPhase {
    pub when: Vec<&'static str>,
    pub tools: Vec<&'static str>,
    pub passive_resource_uri: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct StorePhase {
    pub when: Vec<&'static str>,
    pub tool: &'static str,
    pub project_topics: Vec<String>,
    pub shared_topics: Vec<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProtocolResource {
    pub uri: &'static str,
    pub purpose: &'static str,
}

pub fn protocol_surface(project: Option<&str>) -> MemoryProtocolSurface {
    let project_topics = match project {
        Some(project) => vec![format!("context/{project}"), format!("decisions/{project}")],
        None => vec![
            "context/{project}".to_string(),
            "decisions/{project}".to_string(),
        ],
    };

    MemoryProtocolSurface {
        schema_version: SCOPED_IDENTITY_SCHEMA_VERSION,
        artifact_type: "memory_protocol",
        scoped_identity: ScopedIdentity::from_project(project),
        project: project.map(str::to_string),
        summary: "Recall selectively at task start, store durable outcomes, and use project-aware Hyphae resources instead of broad memory dumps.",
        recall: RecallPhase {
            when: vec![
                "At task start before broad implementation.",
                "After a context switch or when local repo context is insufficient.",
            ],
            tools: vec!["hyphae_gather_context", "hyphae_memory_recall"],
            passive_resource_uri: "hyphae://context/current",
        },
        store: StorePhase {
            when: vec![
                "After a durable architecture or workflow decision.",
                "After resolving an error worth reusing.",
                "After project context changes that future sessions should inherit.",
            ],
            tool: "hyphae_memory_store",
            project_topics,
            shared_topics: vec!["errors/resolved", "preferences"],
        },
        resources: vec![
            ProtocolResource {
                uri: PROTOCOL_RESOURCE_URI,
                purpose: "Canonical memory-use protocol for hosts that need a concise Hyphae contract.",
            },
            ProtocolResource {
                uri: "hyphae://context/current",
                purpose: "Project-scoped passive context bundle for startup recall.",
            },
            ProtocolResource {
                uri: "hyphae://artifacts/project-understanding/current",
                purpose: "Project understanding bundle exported from the code memoir.",
            },
        ],
    }
}

pub fn protocol_surface_json(project: Option<&str>) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(&protocol_surface(project))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_protocol_surface_is_project_aware() {
        let surface = protocol_surface(Some("demo"));
        assert_eq!(surface.schema_version, "1.0");
        assert_eq!(surface.project.as_deref(), Some("demo"));
        assert_eq!(surface.scoped_identity.project.as_deref(), Some("demo"));
        assert!(surface.store.project_topics.contains(&"context/demo".to_string()));
        assert!(
            surface
                .store
                .project_topics
                .contains(&"decisions/demo".to_string())
        );
    }

    #[test]
    fn test_protocol_surface_without_project_uses_templates() {
        let surface = protocol_surface(None);
        assert!(surface.project.is_none());
        assert_eq!(
            surface.store.project_topics,
            vec!["context/{project}".to_string(), "decisions/{project}".to_string()]
        );
    }

    #[test]
    fn test_protocol_surface_json_is_structured_json() {
        let json = protocol_surface_json(Some("demo")).expect("serialize protocol");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse protocol json");
        assert_eq!(parsed["schema_version"].as_str(), Some("1.0"));
        assert_eq!(parsed["artifact_type"].as_str(), Some("memory_protocol"));
        assert_eq!(parsed["project"].as_str(), Some("demo"));
        assert_eq!(parsed["resources"][0]["uri"].as_str(), Some(PROTOCOL_RESOURCE_URI));
    }
}
