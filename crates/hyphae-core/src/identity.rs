use serde::{Deserialize, Serialize};

pub const SCOPED_IDENTITY_SCHEMA_VERSION: &str = "1.0";
pub const BACKUP_EXPORT_SCHEMA_VERSION: &str = "1.0";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopedIdentity {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub runtime_session_id: Option<String>,
}

impl ScopedIdentity {
    pub fn new(
        project: Option<&str>,
        project_root: Option<&str>,
        worktree_id: Option<&str>,
        scope: Option<&str>,
        runtime_session_id: Option<&str>,
    ) -> Self {
        Self {
            project: project.map(str::to_string),
            project_root: project_root.map(str::to_string),
            worktree_id: worktree_id.map(str::to_string),
            scope: scope.map(str::to_string),
            runtime_session_id: runtime_session_id.map(str::to_string),
        }
    }

    pub fn from_project(project: Option<&str>) -> Self {
        Self::new(project, None, None, None, None)
    }

    pub fn has_structured_scope(&self) -> bool {
        self.project_root.is_some() && self.worktree_id.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupExportManifest {
    pub schema_version: String,
    pub export_kind: String,
    pub generated_at: String,
    pub artifact_path: String,
    pub file_size_bytes: u64,
    pub sqlite_integrity: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scoped_identity: Option<ScopedIdentity>,
}

impl BackupExportManifest {
    pub fn new(
        generated_at: &str,
        artifact_path: &str,
        file_size_bytes: u64,
        scoped_identity: Option<ScopedIdentity>,
    ) -> Self {
        Self {
            schema_version: BACKUP_EXPORT_SCHEMA_VERSION.to_string(),
            export_kind: "sqlite_backup".to_string(),
            generated_at: generated_at.to_string(),
            artifact_path: artifact_path.to_string(),
            file_size_bytes,
            sqlite_integrity: "ok".to_string(),
            scoped_identity,
        }
    }
}
