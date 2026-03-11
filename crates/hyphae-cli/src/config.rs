//! Configuration loading from TOML files.
//!
//! Lookup order:
//! 1. `$HYPHAE_CONFIG` environment variable
//! 2. `~/.config/hyphae/config.toml`
//! 3. Built-in defaults (everything is optional)

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::Deserialize;

/// Top-level configuration.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    pub store: StoreConfig,
    pub memory: MemoryConfig,
    pub embeddings: EmbeddingsConfig,
    pub extraction: ExtractionConfig,
    pub recall: RecallConfig,
    pub mcp: McpConfig,
}

/// Database storage settings.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct StoreConfig {
    /// SQLite database path. Default: platform-specific data dir.
    pub path: Option<String>,
}

/// Memory decay and pruning settings.
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct MemoryConfig {
    pub default_importance: String,
    pub decay_rate: f32,
    pub prune_threshold: f32,
}

/// Embedding model settings.
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct EmbeddingsConfig {
    /// Model identifier (fastembed model_code, e.g. "BAAI/bge-small-en-v1.5").
    pub model: String,
}

impl Default for EmbeddingsConfig {
    fn default() -> Self {
        Self {
            model: "BAAI/bge-small-en-v1.5".into(),
        }
    }
}

/// Auto-extraction settings (Layer 0).
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct ExtractionConfig {
    pub enabled: bool,
    /// Minimum keyword score to keep a fact.
    pub min_score: f32,
    /// Maximum facts per extraction pass.
    pub max_facts: usize,
}

/// Context recall/injection settings (Layer 2).
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct RecallConfig {
    pub enabled: bool,
    /// Maximum memories to inject.
    pub limit: usize,
}

/// MCP server settings.
#[derive(Debug, Deserialize)]
#[serde(default)]
pub struct McpConfig {
    pub transport: String,
    /// Compact mode: shorter MCP responses to save tokens (default: true).
    pub compact: bool,
    /// Custom system instructions appended to MCP server info.
    pub instructions: Option<String>,
}

// --- Defaults ---

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            default_importance: "medium".into(),
            decay_rate: 0.95,
            prune_threshold: 0.1,
        }
    }
}

impl Default for ExtractionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_score: 3.0,
            max_facts: 10,
        }
    }
}

impl Default for RecallConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            limit: 15,
        }
    }
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            transport: "stdio".into(),
            compact: true,
            instructions: None,
        }
    }
}

impl Config {
    /// Validate configuration values, returning an error if any field is invalid.
    pub fn validate(&self) -> Result<()> {
        let valid_importance = ["critical", "high", "medium", "low"];
        if !valid_importance.contains(&self.memory.default_importance.as_str()) {
            anyhow::bail!(
                "invalid default_importance {:?}: must be one of critical, high, medium, low",
                self.memory.default_importance
            );
        }

        if self.memory.decay_rate <= 0.0 || self.memory.decay_rate > 1.0 {
            anyhow::bail!(
                "invalid decay_rate {}: must be in (0.0, 1.0]",
                self.memory.decay_rate
            );
        }

        if !(0.0..=1.0).contains(&self.memory.prune_threshold) {
            anyhow::bail!(
                "invalid prune_threshold {}: must be in [0.0, 1.0]",
                self.memory.prune_threshold
            );
        }

        if self.extraction.min_score <= 0.0 {
            anyhow::bail!(
                "invalid extraction.min_score {}: must be > 0.0",
                self.extraction.min_score
            );
        }

        if self.extraction.max_facts == 0 {
            anyhow::bail!("invalid extraction.max_facts: must be > 0");
        }

        if self.recall.limit == 0 {
            anyhow::bail!("invalid recall.limit: must be > 0");
        }

        Ok(())
    }
}

/// Load config from disk. Returns defaults if no config file exists.
pub fn load_config() -> Result<Config> {
    let path = config_path();

    if let Some(p) = &path
        && p.exists()
    {
        let content =
            std::fs::read_to_string(p).with_context(|| format!("reading {}", p.display()))?;
        let config: Config =
            toml::from_str(&content).with_context(|| format!("parsing {}", p.display()))?;
        config.validate()?;
        return Ok(config);
    }

    Ok(Config::default())
}

/// Resolve the config file path.
fn config_path() -> Option<PathBuf> {
    // 1. Environment variable
    if let Ok(p) = std::env::var("HYPHAE_CONFIG") {
        return Some(PathBuf::from(p));
    }

    // 2. ~/.config/hyphae/config.toml
    if let Some(home) = dirs_home() {
        let p = home.join(".config").join("hyphae").join("config.toml");
        return Some(p);
    }

    None
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}

/// Show the active config path (for `hyphae config show`).
#[allow(dead_code)]
pub fn show_config_path() -> String {
    match config_path() {
        Some(p) if p.exists() => format!("{} (loaded)", p.display()),
        Some(p) => format!("{} (not found, using defaults)", p.display()),
        None => "no config path resolved (using defaults)".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert!(config.extraction.enabled);
        assert_eq!(config.memory.decay_rate, 0.95);
        assert_eq!(config.recall.limit, 15);
        assert!(config.mcp.compact);
    }

    #[test]
    fn test_parse_minimal_toml() {
        let toml_str = r#"
[memory]
decay_rate = 0.90
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.memory.decay_rate, 0.90);
        // Other fields should be defaults
        assert!(config.extraction.enabled);
    }

    #[test]
    fn test_validate_valid_config() {
        let config = Config::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_invalid_importance() {
        let mut config = Config::default();
        config.memory.default_importance = "medimum".into();
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("default_importance"));
    }

    #[test]
    fn test_validate_decay_rate_zero() {
        let mut config = Config::default();
        config.memory.decay_rate = 0.0;
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("decay_rate"));
    }

    #[test]
    fn test_validate_decay_rate_above_one() {
        let mut config = Config::default();
        config.memory.decay_rate = 2.0;
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("decay_rate"));
    }

    #[test]
    fn test_validate_prune_threshold_negative() {
        let mut config = Config::default();
        config.memory.prune_threshold = -0.1;
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("prune_threshold"));
    }

    #[test]
    fn test_validate_prune_threshold_above_one() {
        let mut config = Config::default();
        config.memory.prune_threshold = 1.1;
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("prune_threshold"));
    }

    #[test]
    fn test_parse_full_toml() {
        let toml_str = r#"
[store]
path = "/tmp/test.db"

[memory]
default_importance = "high"
decay_rate = 0.90
prune_threshold = 0.2

[extraction]
enabled = false
min_score = 5.0
max_facts = 5

[recall]
enabled = true
limit = 20

[mcp]
transport = "stdio"
instructions = "Custom instructions here"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.store.path.as_deref(), Some("/tmp/test.db"));
        assert!(!config.extraction.enabled);
        assert_eq!(config.recall.limit, 20);
        assert!(config.mcp.instructions.is_some());
    }

    #[test]
    fn test_validate_min_score_zero() {
        let mut config = Config::default();
        config.extraction.min_score = 0.0;
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("min_score"));
    }

    #[test]
    fn test_validate_min_score_negative() {
        let mut config = Config::default();
        config.extraction.min_score = -1.5;
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("min_score"));
    }

    #[test]
    fn test_validate_max_facts_zero() {
        let mut config = Config::default();
        config.extraction.max_facts = 0;
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("max_facts"));
    }

    #[test]
    fn test_validate_recall_limit_zero() {
        let mut config = Config::default();
        config.recall.limit = 0;
        let err = config.validate().unwrap_err();
        assert!(err.to_string().contains("recall.limit"));
    }
}
