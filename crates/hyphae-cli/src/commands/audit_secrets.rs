use regex::Regex;

use hyphae_core::MemoryStore;
use hyphae_store::SqliteStore;

// ─────────────────────────────────────────────────────────────────────────────
// Secrets Detection Patterns
// ─────────────────────────────────────────────────────────────────────────────

const SECRET_PATTERNS: &[(&str, &str)] = &[
    (r"(?i)(api[_-]?key|apikey)\s*[:=]\s*\S{10,}", "API key"),
    (
        r"(?i)(secret|password|passwd|pwd)\s*[:=]\s*\S{8,}",
        "password/secret",
    ),
    (r"sk-[a-zA-Z0-9]{20,}", "OpenAI API key"),
    (r"ghp_[a-zA-Z0-9]{36,}", "GitHub personal access token"),
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

pub fn cmd_audit_secrets(
    store: &SqliteStore,
    topic: Option<String>,
    detailed: bool,
    project: Option<String>,
) -> anyhow::Result<()> {
    let mut total = 0;
    let mut with_secrets = 0;
    let mut all_findings = Vec::new();

    // If topic specified, only audit that topic
    let memories = if let Some(t) = topic {
        store.get_by_topic(&t, project.as_deref())?
    } else {
        // Audit all memories by getting all topics and combining
        let topics = store.list_topics(project.as_deref())?;
        let mut all_mems = Vec::new();
        for (topic_name, _) in topics {
            let mems = store.get_by_topic(&topic_name, project.as_deref())?;
            all_mems.extend(mems);
        }
        all_mems
    };

    for memory in memories {
        total += 1;
        let secrets = detect_secrets(&memory.summary);

        if !secrets.is_empty() {
            with_secrets += 1;
            all_findings.push((memory.id.clone(), memory.topic.clone(), secrets));
        }
    }

    println!("Secrets audit results:");
    println!("  Total memories scanned: {total}");
    println!("  Memories with secrets: {with_secrets}");
    println!(
        "  Risk level: {}",
        if with_secrets > 0 { "HIGH" } else { "LOW" }
    );

    if with_secrets > 0 {
        println!("\nSecrets detected:");
        for (id, topic_name, secrets) in all_findings {
            println!("  - {id} ({topic_name}):");
            if detailed {
                for secret in secrets {
                    println!("    * {secret}");
                }
            } else {
                println!("    * {} found", secrets.len());
            }
        }
        println!("\nRecommendation: Use 'hyphae_memory_forget' to remove these memories.");
    }

    Ok(())
}
