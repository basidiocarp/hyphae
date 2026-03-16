use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

use crate::ids::MemoryId;

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Weight(f32);

impl Weight {
    pub fn new(v: f32) -> Option<Self> {
        if v.is_nan() || !(0.0..=1.0).contains(&v) {
            None
        } else {
            Some(Self(v))
        }
    }

    pub fn new_clamped(v: f32) -> Self {
        if v.is_nan() {
            Self(0.5)
        } else {
            Self(v.clamp(0.0, 1.0))
        }
    }

    pub fn value(self) -> f32 {
        self.0
    }
}

impl Default for Weight {
    fn default() -> Self {
        Self(1.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: MemoryId,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_accessed: DateTime<Utc>,
    pub access_count: u32,
    pub weight: Weight,

    pub topic: String,
    pub summary: String,
    pub raw_excerpt: Option<String>,
    pub keywords: Vec<String>,

    pub importance: Importance,
    pub source: MemorySource,

    pub related_ids: Vec<MemoryId>,

    pub project: Option<String>,

    pub expires_at: Option<DateTime<Utc>>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding: Option<Vec<f32>>,
}

impl Memory {
    pub fn new(topic: String, summary: String, importance: Importance) -> Self {
        Self::builder(topic, summary, importance).build()
    }

    pub fn builder(topic: String, summary: String, importance: Importance) -> MemoryBuilder {
        MemoryBuilder::new(topic, summary, importance)
    }
}

#[must_use]
pub struct MemoryBuilder {
    topic: String,
    summary: String,
    importance: Importance,
    keywords: Vec<String>,
    raw_excerpt: Option<String>,
    embedding: Option<Vec<f32>>,
    source: MemorySource,
    related_ids: Vec<MemoryId>,
    weight: Weight,
    project: Option<String>,
    expires_at: Option<DateTime<Utc>>,
}

impl MemoryBuilder {
    fn new(topic: String, summary: String, importance: Importance) -> Self {
        Self {
            topic,
            summary,
            importance,
            keywords: Vec::new(),
            raw_excerpt: None,
            embedding: None,
            source: MemorySource::Manual,
            related_ids: Vec::new(),
            weight: Weight::default(),
            project: None,
            expires_at: None,
        }
    }

    pub fn keywords(mut self, keywords: Vec<String>) -> Self {
        self.keywords = keywords;
        self
    }

    pub fn raw_excerpt(mut self, raw_excerpt: String) -> Self {
        self.raw_excerpt = Some(raw_excerpt);
        self
    }

    pub fn embedding(mut self, embedding: Vec<f32>) -> Self {
        self.embedding = Some(embedding);
        self
    }

    pub fn source(mut self, source: MemorySource) -> Self {
        self.source = source;
        self
    }

    pub fn related_ids(mut self, related_ids: Vec<MemoryId>) -> Self {
        self.related_ids = related_ids;
        self
    }

    pub fn project(mut self, project: String) -> Self {
        self.project = Some(project);
        self
    }

    pub fn weight(mut self, weight: f32) -> Self {
        self.weight = Weight::new_clamped(weight);
        self
    }

    pub fn expires_at(mut self, dt: DateTime<Utc>) -> Self {
        self.expires_at = Some(dt);
        self
    }

    pub fn build(self) -> Memory {
        let now = Utc::now();
        let expires_at = if self.importance == Importance::Ephemeral && self.expires_at.is_none() {
            Some(now + Duration::hours(4))
        } else {
            self.expires_at
        };
        Memory {
            id: MemoryId::new(),
            created_at: now,
            updated_at: now,
            last_accessed: now,
            access_count: 0,
            weight: self.weight,
            topic: self.topic,
            summary: self.summary,
            raw_excerpt: self.raw_excerpt,
            keywords: self.keywords,
            importance: self.importance,
            source: self.source,
            related_ids: self.related_ids,
            project: self.project,
            expires_at,
            embedding: self.embedding,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Importance {
    Critical,
    High,
    Medium,
    Low,
    Ephemeral,
}

impl fmt::Display for Importance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Critical => write!(f, "critical"),
            Self::High => write!(f, "high"),
            Self::Medium => write!(f, "medium"),
            Self::Low => write!(f, "low"),
            Self::Ephemeral => write!(f, "ephemeral"),
        }
    }
}

impl std::str::FromStr for Importance {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "critical" => Ok(Self::Critical),
            "high" => Ok(Self::High),
            "medium" => Ok(Self::Medium),
            "low" => Ok(Self::Low),
            "ephemeral" => Ok(Self::Ephemeral),
            _ => Err(format!("invalid importance: {s}")),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MemorySource {
    ClaudeCode {
        session_id: String,
        file_path: Option<String>,
    },
    Conversation {
        thread_id: String,
    },
    Manual,
}

impl fmt::Display for MemorySource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ClaudeCode { session_id, .. } => write!(f, "claude-code:{session_id}"),
            Self::Conversation { thread_id } => write!(f, "conversation:{thread_id}"),
            Self::Manual => write!(f, "manual"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_new_generates_unique_ids() {
        let m1 = Memory::new("topic".into(), "summary".into(), Importance::Medium);
        let m2 = Memory::new("topic".into(), "summary".into(), Importance::Medium);
        assert_ne!(m1.id, m2.id);
    }

    #[test]
    fn test_memory_builder_basic() {
        let keywords = vec!["rust".to_string(), "memory".to_string()];
        let m = Memory::builder("topic".into(), "summary".into(), Importance::High)
            .keywords(keywords.clone())
            .raw_excerpt("raw text".into())
            .source(MemorySource::Manual)
            .weight(0.8)
            .build();

        assert_eq!(m.topic, "topic");
        assert_eq!(m.summary, "summary");
        assert_eq!(m.importance, Importance::High);
        assert_eq!(m.keywords, keywords);
        assert_eq!(m.raw_excerpt.as_deref(), Some("raw text"));
        assert!((m.weight.value() - 0.8).abs() < f32::EPSILON);
        assert_eq!(m.access_count, 0);
    }

    #[test]
    fn test_memory_builder_defaults() {
        let m = Memory::builder("t".into(), "s".into(), Importance::Low).build();
        assert_eq!(m.weight.value(), 1.0);
        assert!(m.keywords.is_empty());
        assert!(m.raw_excerpt.is_none());
        assert!(m.embedding.is_none());
        assert!(m.related_ids.is_empty());
        assert_eq!(m.access_count, 0);
    }

    #[test]
    fn test_importance_display_and_fromstr_roundtrip() {
        for variant in [
            Importance::Critical,
            Importance::High,
            Importance::Medium,
            Importance::Low,
            Importance::Ephemeral,
        ] {
            let s = variant.to_string();
            let parsed: Importance = s.parse().expect("should parse");
            assert_eq!(parsed, variant);
        }
    }

    #[test]
    fn test_memory_source_serde_roundtrip() {
        let sources = vec![
            MemorySource::Manual,
            MemorySource::ClaudeCode {
                session_id: "sess-1".into(),
                file_path: Some("src/main.rs".into()),
            },
            MemorySource::Conversation {
                thread_id: "thread-42".into(),
            },
        ];
        for src in sources {
            let json = serde_json::to_string(&src).expect("serialize");
            let back: MemorySource = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(src, back);
        }
    }

    #[test]
    fn test_weight_new_valid() {
        assert!(Weight::new(0.0).is_some());
        assert!(Weight::new(0.5).is_some());
        assert!(Weight::new(1.0).is_some());
    }

    #[test]
    fn test_weight_new_out_of_range() {
        assert!(Weight::new(-0.1).is_none());
        assert!(Weight::new(1.1).is_none());
    }

    #[test]
    fn test_weight_clamped() {
        assert_eq!(Weight::new_clamped(2.0).value(), 1.0);
        assert_eq!(Weight::new_clamped(-1.0).value(), 0.0);
        assert!((Weight::new_clamped(0.5).value() - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_weight_new_nan() {
        assert!(Weight::new(f32::NAN).is_none());
    }

    #[test]
    fn test_weight_new_clamped_nan() {
        let result = Weight::new_clamped(f32::NAN);
        assert_eq!(result.value(), 0.5);
    }
}

#[derive(Debug, Clone)]
pub struct StoreStats {
    pub total_memories: usize,
    pub total_topics: usize,
    pub avg_weight: f32,
    pub oldest_memory: Option<DateTime<Utc>>,
    pub newest_memory: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct TopicHealth {
    pub topic: String,
    pub entry_count: usize,
    pub avg_weight: f32,
    pub avg_access_count: f32,
    pub oldest: Option<DateTime<Utc>>,
    pub newest: Option<DateTime<Utc>>,
    pub last_accessed: Option<DateTime<Utc>>,
    pub needs_consolidation: bool,
    pub stale_count: usize,
}
