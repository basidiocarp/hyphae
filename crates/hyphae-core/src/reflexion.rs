use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

use crate::error::HyphaeResult;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReflexionErrorType {
    BuildError,
    TypeError,
    TestFailure,
    RuntimePanic,
    Other,
}

impl fmt::Display for ReflexionErrorType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BuildError => write!(f, "build_error"),
            Self::TypeError => write!(f, "type_error"),
            Self::TestFailure => write!(f, "test_failure"),
            Self::RuntimePanic => write!(f, "runtime_panic"),
            Self::Other => write!(f, "other"),
        }
    }
}

impl FromStr for ReflexionErrorType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "build_error" => Ok(Self::BuildError),
            "type_error" => Ok(Self::TypeError),
            "test_failure" => Ok(Self::TestFailure),
            "runtime_panic" => Ok(Self::RuntimePanic),
            "other" => Ok(Self::Other),
            _ => Err(format!("invalid error type: {s}")),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReflexionConfidence {
    High,
    Medium,
    Low,
}

impl fmt::Display for ReflexionConfidence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::High => write!(f, "high"),
            Self::Medium => write!(f, "medium"),
            Self::Low => write!(f, "low"),
        }
    }
}

impl FromStr for ReflexionConfidence {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "high" => Ok(Self::High),
            "medium" => Ok(Self::Medium),
            "low" => Ok(Self::Low),
            _ => Err(format!("invalid confidence level: {s}")),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReflexionRecord {
    pub id: String,
    pub error_type: ReflexionErrorType,
    pub root_cause: String,
    pub fix_applied: String,
    pub abstract_pattern: String,
    pub project: Option<String>,
    pub confidence: ReflexionConfidence,
    pub created_at: DateTime<Utc>,
}

impl ReflexionRecord {
    #[must_use]
    pub fn new(
        id: String,
        error_type: ReflexionErrorType,
        root_cause: String,
        fix_applied: String,
        abstract_pattern: String,
        project: Option<String>,
        confidence: ReflexionConfidence,
    ) -> Self {
        Self {
            id,
            error_type,
            root_cause,
            fix_applied,
            abstract_pattern,
            project,
            confidence,
            created_at: Utc::now(),
        }
    }
}

/// Reflexion storage trait for persisting and retrieving structured error-learning records.
pub trait ReflexionStore {
    /// Store a reflexion record and return its ID.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database write fails.
    fn store_reflexion(&self, record: &ReflexionRecord) -> HyphaeResult<String>;

    /// Search reflexion records by query, with optional error type filter.
    /// Results are sorted by confidence (high > medium > low) then by `created_at` DESC.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database query fails.
    fn search_reflexions(
        &self,
        query: &str,
        error_type: Option<&ReflexionErrorType>,
        limit: usize,
    ) -> HyphaeResult<Vec<ReflexionRecord>>;

    /// List all reflexion records sorted by confidence and creation time.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database query fails.
    fn list_reflexions_by_pattern(&self, limit: usize) -> HyphaeResult<Vec<ReflexionRecord>>;
}
