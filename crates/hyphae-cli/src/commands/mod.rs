pub mod bench;
pub mod docs;
pub mod memoir;
pub mod memory;
pub mod prune;
pub mod self_update;

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
    println!("    Model: {}", cfg.embeddings.model);
    println!();
    println!("  MCP Settings:");
    println!("    Transport: {}", cfg.mcp.transport);
    println!("    Compact Mode: {}", cfg.mcp.compact);
    if let Some(instructions) = &cfg.mcp.instructions {
        println!("    Instructions: {}", instructions);
    }
}
