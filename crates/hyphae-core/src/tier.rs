/// Memory tier classification for context-window eviction and recall prioritization.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryTier {
    /// Always-needed facts. Never evicted. High-importance persistent facts.
    Core,
    /// Recent session context. Evicted oldest-first as sessions age.
    #[default]
    Recall,
    /// Long-term archive. Evicted first. Only retrieved when explicitly searched.
    Archival,
}

impl std::fmt::Display for MemoryTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MemoryTier::Core => write!(f, "core"),
            MemoryTier::Recall => write!(f, "recall"),
            MemoryTier::Archival => write!(f, "archival"),
        }
    }
}

impl std::str::FromStr for MemoryTier {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "core" => Ok(MemoryTier::Core),
            "recall" => Ok(MemoryTier::Recall),
            "archival" => Ok(MemoryTier::Archival),
            other => Err(format!("unknown memory tier: {other}")),
        }
    }
}
