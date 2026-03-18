use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use anyhow::{Context, Result};
use hyphae_core::{Importance, Memory, MemorySource, MemoryStore};
use hyphae_store::SqliteStore;
use sha2::{Digest, Sha256};

struct ParsedMemory {
    name: String,
    description: String,
    memory_type: String,
    body: String,
    source_path: String,
    hash_prefix: String,
}

fn claude_memory_dirs(custom_path: Option<&Path>) -> Vec<PathBuf> {
    if let Some(p) = custom_path {
        return vec![p.to_path_buf()];
    }

    let home = match directories::BaseDirs::new() {
        Some(dirs) => dirs.home_dir().to_path_buf(),
        None => return Vec::new(),
    };

    let projects_dir = home.join(".claude").join("projects");
    let Ok(entries) = std::fs::read_dir(&projects_dir) else {
        return Vec::new();
    };

    let mut dirs = Vec::new();
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|ft| ft.is_dir()) {
            let memory_dir = entry.path().join("memory");
            if memory_dir.is_dir() {
                dirs.push(memory_dir);
            }
        }
    }
    dirs
}

fn discover_memory_files(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|ext| ext == "md")
                && path.file_name().is_some_and(|n| n != "MEMORY.md")
            {
                files.push(path);
            }
        }
    }
    files
}

fn parse_frontmatter(content: &str) -> Option<HashMap<String, String>> {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return None;
    }
    // Skip first "---" line
    let after_first = &trimmed[3..];
    let after_first = after_first.strip_prefix('\n').unwrap_or(after_first);

    let end_idx = after_first.find("\n---")?;
    let frontmatter_block = &after_first[..end_idx];

    let mut map = HashMap::new();
    for line in frontmatter_block.lines() {
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim().to_string();
            let value = value.trim().to_string();
            if !key.is_empty() {
                map.insert(key, value);
            }
        }
    }
    Some(map)
}

fn extract_body(content: &str) -> String {
    let trimmed = content.trim_start();
    if !trimmed.starts_with("---") {
        return content.to_string();
    }
    let after_first = &trimmed[3..];
    let after_first = after_first.strip_prefix('\n').unwrap_or(after_first);

    if let Some(end_idx) = after_first.find("\n---") {
        let after_second = &after_first[end_idx + 4..];
        let after_second = after_second.strip_prefix('\n').unwrap_or(after_second);
        after_second.trim().to_string()
    } else {
        content.to_string()
    }
}

fn compute_hash_prefix(content: &str) -> String {
    let hash = Sha256::digest(content.as_bytes());
    let hex = format!("{hash:x}");
    hex[..12].to_string()
}

fn map_topic(memory_type: &str) -> String {
    match memory_type {
        "user" => "claude-memory/user".to_string(),
        "feedback" => "claude-memory/feedback".to_string(),
        "project" => "claude-memory/project".to_string(),
        "reference" => "claude-memory/reference".to_string(),
        other => format!("claude-memory/{other}"),
    }
}

fn map_importance(memory_type: &str) -> Importance {
    match memory_type {
        "feedback" => Importance::High,
        "user" | "project" => Importance::Medium,
        "reference" => Importance::Low,
        _ => Importance::Medium,
    }
}

fn parse_memory_file(path: &Path) -> Result<ParsedMemory> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;

    let frontmatter = parse_frontmatter(&content).unwrap_or_default();
    let name = frontmatter.get("name").cloned().unwrap_or_else(|| {
        path.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string())
    });
    let description = frontmatter.get("description").cloned().unwrap_or_default();
    let memory_type = frontmatter
        .get("type")
        .cloned()
        .unwrap_or_else(|| "project".to_string());
    let body = extract_body(&content);
    let hash_prefix = compute_hash_prefix(&content);

    Ok(ParsedMemory {
        name,
        description,
        memory_type,
        body,
        source_path: path.display().to_string(),
        hash_prefix,
    })
}

fn import_single(store: &SqliteStore, parsed: &ParsedMemory, force: bool) -> Result<ImportAction> {
    if !force && store.memory_exists_with_keyword(&parsed.hash_prefix)? {
        return Ok(ImportAction::Skipped);
    }

    let topic = map_topic(&parsed.memory_type);
    let importance = map_importance(&parsed.memory_type);

    let summary = if parsed.body.is_empty() {
        if parsed.description.is_empty() {
            parsed.name.clone()
        } else {
            parsed.description.clone()
        }
    } else {
        parsed.body.clone()
    };

    let keywords = vec![
        format!("hash:{}", parsed.hash_prefix),
        format!("source:{}", parsed.source_path),
        format!("claude-memory-name:{}", parsed.name),
        parsed.memory_type.clone(),
    ];

    let memory = Memory::builder(topic, summary, importance)
        .keywords(keywords)
        .raw_excerpt(parsed.source_path.clone())
        .source(MemorySource::ClaudeCode {
            session_id: format!("import-{}", parsed.hash_prefix),
            file_path: Some(parsed.source_path.clone()),
        })
        .build();

    store.store(memory)?;
    Ok(ImportAction::Imported)
}

#[derive(Debug, Clone, Copy)]
enum ImportAction {
    Imported,
    Skipped,
}

pub(crate) fn run(
    store: &SqliteStore,
    path: Option<PathBuf>,
    dry_run: bool,
    force: bool,
) -> Result<()> {
    let dirs = claude_memory_dirs(path.as_deref());
    if dirs.is_empty() {
        println!("No Claude Code memory directories found.");
        return Ok(());
    }

    let files = discover_memory_files(&dirs);
    if files.is_empty() {
        println!("No memory files found.");
        return Ok(());
    }

    let mut imported = 0usize;
    let mut skipped = 0usize;
    let mut errors = 0usize;

    for file in &files {
        match parse_memory_file(file) {
            Ok(parsed) => {
                if dry_run {
                    let topic = map_topic(&parsed.memory_type);
                    let action = if !force
                        && store
                            .memory_exists_with_keyword(&parsed.hash_prefix)
                            .unwrap_or(false)
                    {
                        "skip (already imported)"
                    } else {
                        "import"
                    };
                    println!(
                        "[dry-run] Would {action}: {topic} — {} ({})",
                        parsed.name, parsed.hash_prefix
                    );
                    if action.starts_with("import") {
                        imported += 1;
                    } else {
                        skipped += 1;
                    }
                } else {
                    match import_single(store, &parsed, force) {
                        Ok(ImportAction::Imported) => {
                            let topic = map_topic(&parsed.memory_type);
                            println!("Imported: {topic} — {}", parsed.name);
                            imported += 1;
                        }
                        Ok(ImportAction::Skipped) => {
                            skipped += 1;
                        }
                        Err(e) => {
                            eprintln!("Error importing {}: {e}", file.display());
                            errors += 1;
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("Error parsing {}: {e}", file.display());
                errors += 1;
            }
        }
    }

    println!("\nImported: {imported}, Skipped: {skipped} (already imported), Errors: {errors}");
    Ok(())
}

pub(crate) fn watch(store: &SqliteStore, path: Option<PathBuf>, force: bool) -> Result<()> {
    // Initial import
    run(store, path.clone(), false, force)?;

    let dirs = claude_memory_dirs(path.as_deref());
    if dirs.is_empty() {
        eprintln!("No Claude Code memory directories to watch.");
        return Ok(());
    }

    println!("\nWatching for changes... (Ctrl+C to stop)");

    let (tx, rx) = mpsc::channel();

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(event) = res {
            let _ = tx.send(event);
        }
    })?;

    for dir in &dirs {
        use notify::Watcher;
        watcher.watch(dir, notify::RecursiveMode::NonRecursive)?;
        eprintln!("[watch] Watching: {}", dir.display());
    }

    let (ctrlc_tx, ctrlc_rx) = mpsc::channel();
    ctrlc::set_handler(move || {
        let _ = ctrlc_tx.send(());
    })?;

    loop {
        // Check for ctrl-c
        if ctrlc_rx.try_recv().is_ok() {
            println!("\nStopping watch...");
            break;
        }

        // Check for fs events with a timeout
        match rx.recv_timeout(std::time::Duration::from_millis(500)) {
            Ok(event) => {
                use notify::EventKind;
                match event.kind {
                    EventKind::Create(_) | EventKind::Modify(_) => {
                        for path in &event.paths {
                            if path.extension().is_some_and(|ext| ext == "md")
                                && path.file_name().is_some_and(|n| n != "MEMORY.md")
                            {
                                match parse_memory_file(path) {
                                    Ok(parsed) => match import_single(store, &parsed, force) {
                                        Ok(ImportAction::Imported) => {
                                            let topic = map_topic(&parsed.memory_type);
                                            println!("[sync] Imported: {topic} — {}", parsed.name);
                                        }
                                        Ok(ImportAction::Skipped) => {
                                            eprintln!(
                                                "[sync] Skipped (already imported): {}",
                                                parsed.name
                                            );
                                        }
                                        Err(e) => {
                                            eprintln!(
                                                "[sync] Error importing {}: {e}",
                                                path.display()
                                            );
                                        }
                                    },
                                    Err(e) => {
                                        eprintln!("[sync] Error parsing {}: {e}", path.display());
                                    }
                                }
                            }
                        }
                    }
                    EventKind::Remove(_) => {
                        for path in &event.paths {
                            eprintln!(
                                "[sync] File removed: {} (not deleting from hyphae)",
                                path.display()
                            );
                        }
                    }
                    _ => {}
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_frontmatter_basic() {
        let content =
            "---\nname: test-memory\ndescription: A test\ntype: project\n---\n\nBody content here.";
        let fm = parse_frontmatter(content).unwrap();
        assert_eq!(fm.get("name").unwrap(), "test-memory");
        assert_eq!(fm.get("description").unwrap(), "A test");
        assert_eq!(fm.get("type").unwrap(), "project");
    }

    #[test]
    fn test_parse_frontmatter_missing() {
        let content = "No frontmatter here";
        assert!(parse_frontmatter(content).is_none());
    }

    #[test]
    fn test_extract_body() {
        let content = "---\nname: test\ntype: project\n---\n\nThis is the body.";
        let body = extract_body(content);
        assert_eq!(body, "This is the body.");
    }

    #[test]
    fn test_extract_body_no_frontmatter() {
        let content = "Just plain content";
        let body = extract_body(content);
        assert_eq!(body, "Just plain content");
    }

    #[test]
    fn test_compute_hash_prefix() {
        let hash1 = compute_hash_prefix("hello");
        let hash2 = compute_hash_prefix("hello");
        assert_eq!(hash1, hash2);
        assert_eq!(hash1.len(), 12);

        let hash3 = compute_hash_prefix("different");
        assert_ne!(hash1, hash3);
    }

    #[test]
    fn test_map_topic() {
        assert_eq!(map_topic("user"), "claude-memory/user");
        assert_eq!(map_topic("feedback"), "claude-memory/feedback");
        assert_eq!(map_topic("project"), "claude-memory/project");
        assert_eq!(map_topic("reference"), "claude-memory/reference");
        assert_eq!(map_topic("unknown"), "claude-memory/unknown");
    }

    #[test]
    fn test_map_importance() {
        assert_eq!(map_importance("feedback"), Importance::High);
        assert_eq!(map_importance("user"), Importance::Medium);
        assert_eq!(map_importance("project"), Importance::Medium);
        assert_eq!(map_importance("reference"), Importance::Low);
        assert_eq!(map_importance("other"), Importance::Medium);
    }
}
