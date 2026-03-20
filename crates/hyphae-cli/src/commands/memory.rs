use anyhow::Result;
use hyphae_core::{Embedder, MemoryStore};
use hyphae_store::SqliteStore;
use regex::Regex;

fn parse_importance(s: &str) -> hyphae_core::Importance {
    match s.parse() {
        Ok(importance) => importance,
        Err(_) => {
            tracing::warn!("unrecognized importance level: {s}, defaulting to medium");
            hyphae_core::Importance::Medium
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Secrets Detection
// ─────────────────────────────────────────────────────────────────────────────

const SECRET_PATTERNS: &[(&str, &str)] = &[
    (r"(?i)(api[_-]?key|apikey)\s*[:=]\s*\S{10,}", "API key"),
    (
        r"(?i)(secret|password|passwd|pwd)\s*[:=]\s*\S{8,}",
        "password/secret",
    ),
    (r"sk-[a-zA-Z0-9]{20,}", "OpenAI API key"),
    (r"ghp_[a-zA-Z0-9]{36}", "GitHub personal access token"),
    (r"(?i)bearer\s+[a-zA-Z0-9._-]{20,}", "Bearer token"),
    (r"AKIA[0-9A-Z]{16}", "AWS access key"),
    (
        r"(?i)(token|auth)\s*[:=]\s*[a-zA-Z0-9._-]{20,}",
        "auth token",
    ),
    (r"-----BEGIN\s+(RSA\s+)?PRIVATE\s+KEY-----", "private key"),
];

/// Detect common secret patterns in content.
fn detect_secrets(content: &str) -> Vec<String> {
    let mut detected = Vec::new();

    for (pattern, secret_type) in SECRET_PATTERNS {
        if let Ok(regex) = Regex::new(pattern) {
            if regex.is_match(content) {
                detected.push(secret_type.to_string());
            }
        }
    }

    detected
}

pub(crate) fn cmd_store(
    store: &SqliteStore,
    topic: String,
    content: String,
    importance: &str,
    project: Option<String>,
) -> Result<()> {
    // Check for secrets before storing
    let warnings = detect_secrets(&content);
    if !warnings.is_empty() {
        eprintln!(
            "Warning: possible secrets detected: {}",
            warnings.join(", ")
        );
    }

    let mut mem = hyphae_core::Memory::new(topic, content, parse_importance(importance));
    mem.project = project;
    store.store(mem)?;
    println!("Memory stored");
    Ok(())
}

pub(crate) fn cmd_search(
    store: &SqliteStore,
    query: String,
    limit: usize,
    project: Option<String>,
) -> Result<()> {
    let results = store.search_fts(&query, limit, 0, project.as_deref())?;
    for mem in &results {
        crate::display::print_memory(mem, None);
    }
    Ok(())
}

pub(crate) fn cmd_stats(store: &SqliteStore, project: Option<String>) -> Result<()> {
    let stats = store.stats(project.as_deref())?;
    println!("Database Statistics:");
    println!("  Total memories: {}", stats.total_memories);
    Ok(())
}

pub(crate) fn cmd_embed_all(
    store: &SqliteStore,
    embedder: Option<&dyn Embedder>,
    topic_filter: Option<String>,
    batch_size: usize,
    project: Option<String>,
) -> Result<()> {
    let embedder = embedder.ok_or_else(|| {
        anyhow::anyhow!(
            "no embedder available\n\
             Set HYPHAE_EMBEDDING_URL and HYPHAE_EMBEDDING_MODEL for HTTP embeddings,\n\
             or build with embeddings feature: cargo install hyphae"
        )
    })?;

    let project_ref = project.as_deref();

    // Collect all memories, optionally filtered by topic
    let memories = if let Some(ref t) = topic_filter {
        store.get_by_topic(t, project_ref)?
    } else {
        let topics = store.list_topics(project_ref)?;
        let mut all = Vec::new();
        for (t, _) in &topics {
            let mems = store.get_by_topic(t, project_ref)?;
            all.extend(mems);
        }
        all
    };

    // Filter to those without embeddings
    let to_embed: Vec<_> = memories.iter().filter(|m| m.embedding.is_none()).collect();

    if to_embed.is_empty() {
        println!("All memories already have embeddings.");
        return Ok(());
    }

    let total = to_embed.len();
    println!("Embedding {total} memories (batch size: {batch_size})...");

    let mut embedded = 0usize;
    let mut errors = 0usize;

    for chunk in to_embed.chunks(batch_size) {
        let texts: Vec<String> = chunk
            .iter()
            .map(|m| format!("{} {}", m.topic, m.summary))
            .collect();
        let text_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();

        match embedder.embed_batch(&text_refs) {
            Ok(vecs) => {
                for (mem, vec) in chunk.iter().zip(vecs) {
                    let mut updated = (*mem).clone();
                    updated.embedding = Some(vec);
                    if store.update(&updated).is_ok() {
                        embedded += 1;
                    } else {
                        errors += 1;
                    }
                }
            }
            Err(e) => {
                eprintln!("Batch embedding failed: {e}");
                errors += chunk.len();
            }
        }

        // Progress
        let done = embedded + errors;
        let pct = done * 100 / total;
        let bar_width = 30;
        let filled = bar_width * done / total;
        let bar: String = "=".repeat(filled) + &" ".repeat(bar_width - filled);
        eprint!("\rEmbedding: {done}/{total} [{bar}] {pct}%");
    }

    eprintln!(); // newline after progress bar
    println!("Done: {embedded} embedded, {errors} errors (of {total} total)");

    Ok(())
}
