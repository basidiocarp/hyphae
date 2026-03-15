use crate::error::{HyphaeError, HyphaeResult};
use crate::ids::{ChunkId, DocumentId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: DocumentId,
    pub source_path: String,
    pub source_type: SourceType,
    pub chunk_count: usize,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub project: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum SourceType {
    Code,
    Markdown,
    Pdf,
    #[default]
    Text,
}

impl fmt::Display for SourceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Code => write!(f, "code"),
            Self::Markdown => write!(f, "markdown"),
            Self::Pdf => write!(f, "pdf"),
            Self::Text => write!(f, "text"),
        }
    }
}

impl FromStr for SourceType {
    type Err = HyphaeError;

    fn from_str(s: &str) -> HyphaeResult<Self> {
        match s {
            "code" => Ok(Self::Code),
            "markdown" => Ok(Self::Markdown),
            "pdf" => Ok(Self::Pdf),
            "text" => Ok(Self::Text),
            other => Err(HyphaeError::Validation(format!(
                "unknown source type: {other}"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chunk {
    pub id: ChunkId,
    pub document_id: DocumentId,
    pub chunk_index: u32,
    pub content: String,
    pub metadata: ChunkMetadata,
    pub embedding: Option<Vec<f32>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkMetadata {
    pub source_path: String,
    pub source_type: SourceType,
    pub language: Option<String>,
    pub heading: Option<String>,
    pub line_start: Option<u32>,
    pub line_end: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkSearchResult {
    pub chunk: Chunk,
    pub score: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_source_type_roundtrip() {
        let cases = [
            (SourceType::Code, "code"),
            (SourceType::Markdown, "markdown"),
            (SourceType::Pdf, "pdf"),
            (SourceType::Text, "text"),
        ];
        for (variant, s) in &cases {
            assert_eq!(variant.to_string(), *s);
            assert_eq!(SourceType::from_str(s).unwrap(), *variant);
        }
    }

    #[test]
    fn test_source_type_invalid() {
        let result = SourceType::from_str("unknown");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), HyphaeError::Validation(_)));
    }

    #[test]
    fn test_chunk_metadata_defaults() {
        let meta = ChunkMetadata {
            source_path: "test.rs".to_string(),
            source_type: SourceType::default(),
            language: None,
            heading: None,
            line_start: None,
            line_end: None,
        };
        assert!(meta.language.is_none());
        assert!(meta.heading.is_none());
        assert!(meta.line_start.is_none());
        assert!(meta.line_end.is_none());
        assert_eq!(meta.source_type, SourceType::Text);
    }
}
