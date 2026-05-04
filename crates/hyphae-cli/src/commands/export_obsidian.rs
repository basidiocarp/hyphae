use anyhow::{Context, Result};
use hyphae_core::{Concept, Memoir, MemoirStore, Memory, MemoryStore, detect_secrets};
use hyphae_store::SqliteStore;
use std::fs;
use std::path::{Path, PathBuf};

pub(crate) fn cmd_export_obsidian(
    store: &SqliteStore,
    vault: PathBuf,
    project: Option<String>,
    include_memoirs: bool,
    dry_run: bool,
    overwrite: bool,
) -> Result<()> {
    if vault.as_os_str().is_empty() {
        anyhow::bail!("vault path must not be empty");
    }

    // Create vault root directory
    if !dry_run {
        fs::create_dir_all(&vault)
            .with_context(|| format!("failed to create vault directory {}", vault.display()))?;
    }

    let mut total_created = 0;
    let mut total_skipped = 0;
    let mut total_failed = 0;

    // Export memories, decisions, and audit findings
    export_memories(
        store,
        &vault,
        project.as_deref(),
        dry_run,
        overwrite,
        &mut total_created,
        &mut total_skipped,
        &mut total_failed,
    )?;

    // Export memoirs and their concepts
    if include_memoirs {
        export_memoirs(
            store,
            &vault,
            dry_run,
            overwrite,
            &mut total_created,
            &mut total_skipped,
            &mut total_failed,
        )?;
    }

    // Summary
    println!(
        "Export complete: {} created, {} skipped, {} failed",
        total_created, total_skipped, total_failed
    );

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn export_memories(
    store: &SqliteStore,
    vault: &Path,
    project: Option<&str>,
    dry_run: bool,
    overwrite: bool,
    total_created: &mut usize,
    total_skipped: &mut usize,
    total_failed: &mut usize,
) -> Result<()> {
    let topics = store
        .list_topics(project)
        .context("failed to list topics")?;

    for (topic, _count) in topics {
        let memories = store
            .get_by_topic(&topic, project)
            .with_context(|| format!("failed to get memories for topic {}", topic))?;

        for memory in memories {
            let (subfolder, note_type) = if topic.starts_with("decisions/") {
                ("decisions", "decision")
            } else if topic.starts_with("audit/") {
                ("audit", "audit")
            } else {
                ("memories", "memory")
            };

            // audit findings are cross-project; no project subdirectory
            let note_dir = if subfolder == "audit" {
                vault.join(subfolder)
            } else {
                vault.join(subfolder).join(project.unwrap_or("global"))
            };

            let filename = format!("{}.md", memory.id);
            let note_path = note_dir.join(&filename);

            let content = create_memory_note(&memory, &topic, note_type)?;

            match write_note(&note_path, &content, dry_run, overwrite) {
                Ok(written) => {
                    if written {
                        *total_created += 1;
                    } else {
                        *total_skipped += 1;
                    }
                }
                Err(e) => {
                    eprintln!(
                        "Warning: failed to write note {}: {}",
                        note_path.display(),
                        e
                    );
                    *total_failed += 1;
                }
            }
        }
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn export_memoirs(
    store: &SqliteStore,
    vault: &Path,
    dry_run: bool,
    overwrite: bool,
    total_created: &mut usize,
    total_skipped: &mut usize,
    total_failed: &mut usize,
) -> Result<()> {
    let memoirs = store.list_memoirs().context("failed to list memoirs")?;

    for memoir in memoirs {
        let memoir_dir = vault.join("memoirs").join(sanitize_filename(&memoir.name));

        // Write memoir _index.md
        let index_content = create_memoir_index(&memoir)?;
        let index_path = memoir_dir.join("_index.md");

        match write_note(&index_path, &index_content, dry_run, overwrite) {
            Ok(written) => {
                if written {
                    *total_created += 1;
                } else {
                    *total_skipped += 1;
                }
            }
            Err(e) => {
                eprintln!(
                    "Warning: failed to write memoir index {}: {}",
                    index_path.display(),
                    e
                );
                *total_failed += 1;
            }
        }

        // Write concept notes
        let concepts = store
            .list_concepts(&memoir.id)
            .with_context(|| format!("failed to list concepts for memoir {}", memoir.name))?;

        for concept in concepts {
            let concept_content = create_concept_note(&concept, &memoir.name)?;
            let concept_path = memoir_dir.join(format!("{}.md", sanitize_filename(&concept.name)));

            match write_note(&concept_path, &concept_content, dry_run, overwrite) {
                Ok(written) => {
                    if written {
                        *total_created += 1;
                    } else {
                        *total_skipped += 1;
                    }
                }
                Err(e) => {
                    eprintln!(
                        "Warning: failed to write concept {}: {}",
                        concept_path.display(),
                        e
                    );
                    *total_failed += 1;
                }
            }
        }
    }

    Ok(())
}

fn sanitize_filename(s: &str) -> String {
    let sanitized: String = s
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0' => '-',
            c => c,
        })
        .collect::<String>()
        .to_lowercase()
        .replace(' ', "-");
    let stripped = sanitized.trim_start_matches('.');
    if stripped.is_empty() {
        "unnamed".to_string()
    } else {
        stripped.to_string()
    }
}

fn write_note(path: &Path, content: &str, dry_run: bool, overwrite: bool) -> Result<bool> {
    let already_exists = path.exists();

    if already_exists && !overwrite {
        println!("SKIP {}", path.display());
        return Ok(false);
    }

    let secrets = detect_secrets(content);
    if !secrets.is_empty() {
        eprintln!(
            "Warning: skipping note {} — contains potential secrets: {}",
            path.display(),
            secrets.join(", ")
        );
        return Ok(false);
    }

    if dry_run {
        println!(
            "{} {}",
            if already_exists { "UPDATE" } else { "CREATE" },
            path.display()
        );
        return Ok(true);
    }

    // Create parent directory
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create directory {}", parent.display()))?;
    }

    // Write file
    fs::write(path, content).with_context(|| format!("failed to write note {}", path.display()))?;

    println!(
        "{} {}",
        if already_exists { "UPDATE" } else { "CREATE" },
        path.display()
    );
    Ok(true)
}

fn memory_frontmatter(memory: &Memory, note_type: &str, topic: &str) -> String {
    let tags_field = if memory.keywords.is_empty() {
        "tags: []".to_string()
    } else {
        let items = memory
            .keywords
            .iter()
            .map(|k| format!("  - {}", k))
            .collect::<Vec<_>>()
            .join("\n");
        format!("tags:\n{}", items)
    };

    format!(
        "---\nhyphae_id: {}\ntype: {}\nproject: {}\ntopic: {}\nimportance: {}\n{}\ncreated_at: {}\nupdated_at: {}\nsource: hyphae-export-v1\n---\n",
        memory.id,
        note_type,
        memory.project.as_deref().unwrap_or("global"),
        topic,
        memory.importance,
        tags_field,
        memory.created_at.to_rfc3339(),
        memory.updated_at.to_rfc3339(),
    )
}

fn concept_frontmatter(concept: &Concept, memoir_name: &str) -> String {
    format!(
        "---\nhyphae_id: {}\ntype: concept\nproject: global\nmemoir: {}\ntopic: memoirs/{}\ntags: []\ncreated_at: {}\nupdated_at: {}\nsource: hyphae-export-v1\n---\n",
        concept.id,
        memoir_name,
        memoir_name,
        concept.created_at.to_rfc3339(),
        concept.updated_at.to_rfc3339(),
    )
}

fn create_memory_note(memory: &Memory, topic: &str, note_type: &str) -> Result<String> {
    let frontmatter = memory_frontmatter(memory, note_type, topic);
    let body = format!(
        "## Summary\n{}\n\n## Keywords\n{}",
        memory.summary,
        if memory.keywords.is_empty() {
            "(none)".to_string()
        } else {
            memory.keywords.join(", ")
        }
    );

    Ok(format!("{}{}", frontmatter, body))
}

fn create_memoir_index(memoir: &Memoir) -> Result<String> {
    let frontmatter = format!(
        "---\nhyphae_id: {}\ntype: memoir\nproject: global\ntopic: memoirs/{}\ntags: []\ncreated_at: {}\nupdated_at: {}\nsource: hyphae-export-v1\n---\n",
        memoir.id,
        memoir.name,
        memoir.created_at.to_rfc3339(),
        memoir.updated_at.to_rfc3339(),
    );

    let body = format!("## Description\n{}", memoir.description);

    Ok(format!("{}{}", frontmatter, body))
}

fn create_concept_note(concept: &Concept, memoir_name: &str) -> Result<String> {
    let frontmatter = concept_frontmatter(concept, memoir_name);
    let body = format!("## Definition\n{}", concept.definition);

    Ok(format!("{}{}", frontmatter, body))
}
