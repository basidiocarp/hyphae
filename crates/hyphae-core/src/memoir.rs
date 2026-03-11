use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

use crate::error::{HyphaeError, HyphaeResult};
use crate::ids::{ConceptId, LinkId, MemoirId, MemoryId};
use crate::memory::Weight;

#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Confidence(f32);

impl Confidence {
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

impl Default for Confidence {
    fn default() -> Self {
        Self(0.5)
    }
}

// ===========================================================================
// Memoir
// ===========================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Memoir {
    pub id: MemoirId,
    pub name: String,
    pub description: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Reserved for future use.
    pub consolidation_threshold: u32,
}

impl Memoir {
    pub fn new(name: String, description: String) -> Self {
        let now = Utc::now();
        Self {
            id: MemoirId::new(),
            name,
            description,
            created_at: now,
            updated_at: now,
            consolidation_threshold: 50,
        }
    }
}

// ===========================================================================
// Label
// ===========================================================================

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Label {
    pub namespace: String,
    pub value: String,
}

impl Label {
    pub fn new(namespace: impl Into<String>, value: impl Into<String>) -> HyphaeResult<Self> {
        let ns = namespace.into();
        let val = value.into();

        // Validation rules
        if ns.is_empty() {
            return Err(HyphaeError::Validation(
                "namespace cannot be empty".to_string(),
            ));
        }
        if val.is_empty() {
            return Err(HyphaeError::Validation("value cannot be empty".to_string()));
        }
        if ns.contains(':') {
            return Err(HyphaeError::Validation(
                "namespace cannot contain ':' (breaks parsing)".to_string(),
            ));
        }

        Ok(Self {
            namespace: ns,
            value: val,
        })
    }
}

impl fmt::Display for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.namespace, self.value)
    }
}

impl std::str::FromStr for Label {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err("label cannot be empty".to_string());
        }

        if let Some((ns, val)) = s.split_once(':') {
            if ns.is_empty() {
                return Err("namespace cannot be empty".to_string());
            }
            if val.is_empty() {
                return Err("value cannot be empty".to_string());
            }
            Ok(Self {
                namespace: ns.to_string(),
                value: val.to_string(),
            })
        } else {
            Ok(Self {
                namespace: "tag".to_string(),
                value: s.to_string(),
            })
        }
    }
}

// ===========================================================================
// Concept
// ===========================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Concept {
    pub id: ConceptId,
    pub memoir_id: MemoirId,
    pub name: String,
    pub definition: String,
    pub labels: Vec<Label>,
    pub confidence: Confidence,
    pub revision: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub source_memory_ids: Vec<MemoryId>,
}

impl Concept {
    pub fn new(memoir_id: MemoirId, name: String, definition: String) -> Self {
        let now = Utc::now();
        Self {
            id: ConceptId::new(),
            memoir_id,
            name,
            definition,
            labels: Vec::new(),
            confidence: Confidence::default(),
            revision: 1,
            created_at: now,
            updated_at: now,
            source_memory_ids: Vec::new(),
        }
    }
}

// ===========================================================================
// Relation
// ===========================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    PartOf,
    DependsOn,
    RelatedTo,
    Contradicts,
    Refines,
    AlternativeTo,
    CausedBy,
    InstanceOf,
    SupersededBy,
}

impl Relation {
    /// Returns `true` for symmetric relations (undirected), `false` for directional relations.
    pub fn is_symmetric(&self) -> bool {
        matches!(
            self,
            Relation::RelatedTo | Relation::Contradicts | Relation::AlternativeTo
        )
    }
}

impl fmt::Display for Relation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::PartOf => write!(f, "part_of"),
            Self::DependsOn => write!(f, "depends_on"),
            Self::RelatedTo => write!(f, "related_to"),
            Self::Contradicts => write!(f, "contradicts"),
            Self::Refines => write!(f, "refines"),
            Self::AlternativeTo => write!(f, "alternative_to"),
            Self::CausedBy => write!(f, "caused_by"),
            Self::InstanceOf => write!(f, "instance_of"),
            Self::SupersededBy => write!(f, "superseded_by"),
        }
    }
}

impl std::str::FromStr for Relation {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "part_of" | "partof" => Ok(Self::PartOf),
            "depends_on" | "dependson" => Ok(Self::DependsOn),
            "related_to" | "relatedto" => Ok(Self::RelatedTo),
            "contradicts" => Ok(Self::Contradicts),
            "refines" => Ok(Self::Refines),
            "alternative_to" | "alternativeto" => Ok(Self::AlternativeTo),
            "caused_by" | "causedby" => Ok(Self::CausedBy),
            "instance_of" | "instanceof" => Ok(Self::InstanceOf),
            "superseded_by" | "supersededby" => Ok(Self::SupersededBy),
            _ => Err(format!("invalid relation: {s}")),
        }
    }
}

// ===========================================================================
// ConceptLink
// ===========================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConceptLink {
    pub id: LinkId,
    pub source_id: ConceptId,
    pub target_id: ConceptId,
    pub relation: Relation,
    pub weight: Weight,
    pub created_at: DateTime<Utc>,
}

impl ConceptLink {
    pub fn new(source_id: ConceptId, target_id: ConceptId, relation: Relation) -> Self {
        Self {
            id: LinkId::new(),
            source_id,
            target_id,
            relation,
            weight: Weight::default(),
            created_at: Utc::now(),
        }
    }
}

// ===========================================================================
// MemoirStats
// ===========================================================================

#[derive(Debug, Clone)]
pub struct MemoirStats {
    pub total_concepts: usize,
    pub total_links: usize,
    pub avg_confidence: f32,
    pub label_counts: Vec<(String, usize)>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_label_new_valid() {
        let label = Label::new("namespace", "value");
        assert!(label.is_ok());
        let l = label.unwrap();
        assert_eq!(l.namespace, "namespace");
        assert_eq!(l.value, "value");
    }

    #[test]
    fn test_label_new_empty_namespace() {
        let label = Label::new("", "value");
        assert!(label.is_err());
        match label {
            Err(HyphaeError::Validation(msg)) => assert!(msg.contains("namespace cannot be empty")),
            _ => panic!("Expected Validation error"),
        }
    }

    #[test]
    fn test_label_new_empty_value() {
        let label = Label::new("namespace", "");
        assert!(label.is_err());
        match label {
            Err(HyphaeError::Validation(msg)) => assert!(msg.contains("value cannot be empty")),
            _ => panic!("Expected Validation error"),
        }
    }

    #[test]
    fn test_label_new_colon_in_namespace() {
        let label = Label::new("name:space", "value");
        assert!(label.is_err());
        match label {
            Err(HyphaeError::Validation(msg)) => assert!(msg.contains("cannot contain ':'")),
            _ => panic!("Expected Validation error"),
        }
    }

    #[test]
    fn test_relation_is_symmetric() {
        // Symmetric relations
        assert!(Relation::RelatedTo.is_symmetric());
        assert!(Relation::Contradicts.is_symmetric());
        assert!(Relation::AlternativeTo.is_symmetric());

        // Directional relations
        assert!(!Relation::DependsOn.is_symmetric());
        assert!(!Relation::PartOf.is_symmetric());
        assert!(!Relation::Refines.is_symmetric());
        assert!(!Relation::CausedBy.is_symmetric());
        assert!(!Relation::InstanceOf.is_symmetric());
        assert!(!Relation::SupersededBy.is_symmetric());
    }

    #[test]
    fn test_label_fromstr_roundtrip() {
        let original: Label = "ns:val".parse().expect("parse");
        let displayed = original.to_string();
        assert_eq!(displayed, "ns:val");
        let reparsed: Label = displayed.parse().expect("reparse");
        assert_eq!(reparsed, original);
    }

    #[test]
    fn test_relation_all_variants_fromstr() {
        let cases = [
            ("part_of", Relation::PartOf),
            ("depends_on", Relation::DependsOn),
            ("related_to", Relation::RelatedTo),
            ("contradicts", Relation::Contradicts),
            ("refines", Relation::Refines),
            ("alternative_to", Relation::AlternativeTo),
            ("caused_by", Relation::CausedBy),
            ("instance_of", Relation::InstanceOf),
            ("superseded_by", Relation::SupersededBy),
        ];
        for (s, expected) in cases {
            let parsed: Relation = s.parse().expect(s);
            assert_eq!(parsed, expected, "failed for {s}");
        }
    }

    #[test]
    fn test_relation_aliases() {
        assert_eq!("partof".parse::<Relation>().unwrap(), Relation::PartOf);
        assert_eq!(
            "dependson".parse::<Relation>().unwrap(),
            Relation::DependsOn
        );
        assert_eq!(
            "relatedto".parse::<Relation>().unwrap(),
            Relation::RelatedTo
        );
        assert_eq!(
            "alternativeto".parse::<Relation>().unwrap(),
            Relation::AlternativeTo
        );
        assert_eq!("causedby".parse::<Relation>().unwrap(), Relation::CausedBy);
        assert_eq!(
            "instanceof".parse::<Relation>().unwrap(),
            Relation::InstanceOf
        );
        assert_eq!(
            "supersededby".parse::<Relation>().unwrap(),
            Relation::SupersededBy
        );
    }

    #[test]
    fn test_confidence_bounds() {
        assert!(Confidence::new(0.0).is_some());
        assert!(Confidence::new(0.5).is_some());
        assert!(Confidence::new(1.0).is_some());
        assert!(Confidence::new(-0.1).is_none());
        assert!(Confidence::new(1.1).is_none());
        assert_eq!(Confidence::new_clamped(2.0).value(), 1.0);
        assert_eq!(Confidence::new_clamped(-1.0).value(), 0.0);
    }

    #[test]
    fn test_confidence_new_nan() {
        assert!(Confidence::new(f32::NAN).is_none());
    }

    #[test]
    fn test_confidence_new_clamped_nan() {
        let result = Confidence::new_clamped(f32::NAN);
        assert_eq!(result.value(), 0.5);
    }

    #[test]
    fn test_label_fromstr_empty_string() {
        let result = "".parse::<Label>();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("empty"));
    }

    #[test]
    fn test_label_fromstr_empty_namespace() {
        let result = ":value".parse::<Label>();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("namespace"));
    }

    #[test]
    fn test_label_fromstr_empty_value() {
        let result = "namespace:".parse::<Label>();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("value"));
    }

    #[test]
    fn test_label_fromstr_valid_with_colon() {
        let result = "ns:val".parse::<Label>();
        assert!(result.is_ok());
        let label = result.unwrap();
        assert_eq!(label.namespace, "ns");
        assert_eq!(label.value, "val");
    }

    #[test]
    fn test_label_fromstr_valid_without_colon() {
        let result = "simple".parse::<Label>();
        assert!(result.is_ok());
        let label = result.unwrap();
        assert_eq!(label.namespace, "tag");
        assert_eq!(label.value, "simple");
    }
}
