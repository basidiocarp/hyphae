use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Typed artifact categories stored in the `artifacts` table.
///
/// Use `#[non_exhaustive]` because new artifact types are expected over time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ArtifactType {
    CompactSummary,
    CouncilLifecycle,
    ProjectUnderstanding,
}

impl ArtifactType {
    /// Canonical string value written to the `artifact_type` column.
    pub fn as_str(&self) -> &'static str {
        match self {
            ArtifactType::CompactSummary => "compact_summary",
            ArtifactType::CouncilLifecycle => "council_lifecycle",
            ArtifactType::ProjectUnderstanding => "project_understanding",
        }
    }
}

/// Error returned when parsing an unknown artifact type string.
#[derive(Debug, PartialEq)]
pub struct UnknownArtifactType(pub String);

impl std::fmt::Display for UnknownArtifactType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "unknown artifact type: '{}'", self.0)
    }
}

impl FromStr for ArtifactType {
    type Err = UnknownArtifactType;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "compact_summary" => Ok(ArtifactType::CompactSummary),
            "council_lifecycle" => Ok(ArtifactType::CouncilLifecycle),
            "project_understanding" => Ok(ArtifactType::ProjectUnderstanding),
            _ => Err(UnknownArtifactType(s.to_owned())),
        }
    }
}

/// A stored artifact record returned from `query_artifacts` and `latest_artifact`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub artifact_id: String,
    pub artifact_type: String,
    pub project: Option<String>,
    pub source_id: Option<String>,
    pub payload: serde_json::Value,
    pub created_at: String,
    pub schema_version: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_artifact_type_roundtrip() {
        for (t, expected) in [
            (ArtifactType::CompactSummary, "compact_summary"),
            (ArtifactType::CouncilLifecycle, "council_lifecycle"),
            (ArtifactType::ProjectUnderstanding, "project_understanding"),
        ] {
            assert_eq!(t.as_str(), expected);
            assert_eq!(expected.parse::<ArtifactType>().unwrap(), t);
        }
    }

    #[test]
    fn test_artifact_type_unknown_returns_err() {
        assert!("unknown_type".parse::<ArtifactType>().is_err());
        assert!("".parse::<ArtifactType>().is_err());
    }

    #[test]
    fn test_artifact_type_serde_snake_case() {
        let serialized = serde_json::to_string(&ArtifactType::CompactSummary).unwrap();
        assert_eq!(serialized, "\"compact_summary\"");

        let deserialized: ArtifactType = serde_json::from_str("\"council_lifecycle\"").unwrap();
        assert_eq!(deserialized, ArtifactType::CouncilLifecycle);
    }
}
