pub mod audit_secrets;
pub mod backup;
pub mod bench;
pub mod changelog;
pub mod codex_notify;
pub mod docs;
pub mod doctor;
pub mod evaluate;
pub mod export_training;
pub mod feedback;
pub mod import_claude_memory;
pub mod memoir;
pub mod memory;
pub mod project;
pub mod prune;
pub mod purge;
pub mod self_update;
pub mod session;
pub mod transcript;

use crate::config::Config;

pub(crate) fn cmd_config(cfg: &Config) {
    println!("Configuration:");
    println!("  Config Path: {}", crate::config::show_config_path());
    println!();
    println!("  Memory Settings:");
    println!("    Default Importance: {}", cfg.memory.default_importance);
    println!("    Decay Rate: {}", cfg.memory.decay_rate);
    println!("    Prune Threshold: {}", cfg.memory.prune_threshold);
    println!();
    println!("  Extraction Settings:");
    println!("    Enabled: {}", cfg.extraction.enabled);
    println!("    Min Score: {}", cfg.extraction.min_score);
    println!("    Max Facts: {}", cfg.extraction.max_facts);
    println!();
    println!("  Recall Settings:");
    println!("    Enabled: {}", cfg.recall.enabled);
    println!("    Limit: {}", cfg.recall.limit);
    println!();
    println!("  Embeddings Settings:");
    println!("    FastEmbed Model: {}", cfg.embeddings.model);
    println!(
        "    FastEmbed Available: {}",
        if cfg!(feature = "embeddings") {
            "yes"
        } else {
            "no (--no-default-features build)"
        }
    );

    let http_url = std::env::var("HYPHAE_EMBEDDING_URL").unwrap_or_default();
    let http_model = std::env::var("HYPHAE_EMBEDDING_MODEL").unwrap_or_default();
    if http_url.is_empty() {
        println!("    HTTP Embedder: not configured");
        println!("      Set HYPHAE_EMBEDDING_URL and HYPHAE_EMBEDDING_MODEL to enable");
    } else {
        println!("    HTTP Embedder: enabled");
        println!("      URL: {http_url}");
        println!(
            "      Model: {}",
            if http_model.is_empty() {
                "(not set — will error)"
            } else {
                &http_model
            }
        );
    }

    let active_backend = if !http_url.is_empty() && !http_model.is_empty() {
        "http"
    } else if cfg!(feature = "embeddings") {
        "fastembed"
    } else {
        "none (FTS-only search)"
    };
    println!("    Active Backend: {active_backend}");
    println!();
    println!("  MCP Settings:");
    println!("    Transport: {}", cfg.mcp.transport);
    println!("    Compact Mode: {}", cfg.mcp.compact);
    if let Some(instructions) = &cfg.mcp.instructions {
        println!("    Instructions: {}", instructions);
    }
}
