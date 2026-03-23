use anyhow::Result;
use hyphae_core::ChunkStore;
use hyphae_store::SqliteStore;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

pub struct WatchOptions {
    pub path: PathBuf,
    pub recursive: bool,
    pub debounce_ms: u64,
    pub project: Option<String>,
}

pub fn run_watch(opts: WatchOptions, store: &SqliteStore) -> Result<()> {
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })?;

    let (tx, rx) = mpsc::channel();
    let mut watcher: RecommendedWatcher = notify::recommended_watcher(move |res| {
        if let Ok(event) = res {
            let _ = tx.send(event);
        }
    })?;

    let mode = if opts.recursive {
        RecursiveMode::Recursive
    } else {
        RecursiveMode::NonRecursive
    };
    watcher.watch(&opts.path, mode)?;

    let mut debounce: HashMap<PathBuf, Instant> = HashMap::new();
    eprintln!(
        "[watch] Watching {} (press Ctrl+C to stop)",
        opts.path.display()
    );

    while running.load(Ordering::SeqCst) {
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(event) => process_event(&event, &opts, store, &mut debounce),
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    eprintln!("[watch] Stopped.");
    Ok(())
}

fn process_event(
    event: &notify::Event,
    opts: &WatchOptions,
    store: &SqliteStore,
    debounce: &mut HashMap<PathBuf, Instant>,
) {
    for path in &event.paths {
        if !path.is_file() {
            continue;
        }

        if hyphae_ingest::should_skip(path) {
            continue;
        }

        if debounce
            .get(path)
            .map(|t| t.elapsed().as_millis() < opts.debounce_ms as u128)
            .unwrap_or(false)
        {
            continue;
        }
        debounce.insert(path.clone(), Instant::now());

        match &event.kind {
            EventKind::Create(_) | EventKind::Modify(_) => {
                reingest_file(path, opts, store);
            }
            EventKind::Remove(_) => {
                remove_file(path, opts, store);
            }
            _ => {}
        }
    }
}

fn reingest_file(path: &Path, opts: &WatchOptions, store: &SqliteStore) {
    let path_str = path.to_string_lossy();
    match hyphae_ingest::ingest_file(path, None) {
        Ok((doc, chunks)) => {
            let chunk_count = chunks.len();
            if let Ok(Some(existing)) =
                store.get_document_by_path(&path_str, opts.project.as_deref())
            {
                let _ = store.delete_document(&existing.id);
            }
            let result = store
                .store_document(doc)
                .and_then(|_| store.store_chunks(chunks))
                .map(|_| ());
            match result {
                Ok(()) => eprintln!(
                    "[watch] Re-ingested: {} ({} chunks)",
                    path.display(),
                    chunk_count
                ),
                Err(e) => eprintln!("[watch] Failed to store {}: {e}", path.display()),
            }
        }
        Err(e) => eprintln!("[watch] Failed to ingest {}: {e}", path.display()),
    }
}

fn remove_file(path: &Path, opts: &WatchOptions, store: &SqliteStore) {
    let path_str = path.to_string_lossy();
    match store.get_document_by_path(&path_str, opts.project.as_deref()) {
        Ok(Some(doc)) => match store.delete_document(&doc.id) {
            Ok(()) => eprintln!("[watch] Removed: {}", path.display()),
            Err(e) => eprintln!("[watch] Failed to remove {}: {e}", path.display()),
        },
        Ok(None) => {}
        Err(e) => eprintln!("[watch] Error looking up {}: {e}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_debounce_skips_rapid_events() {
        let mut debounce: HashMap<PathBuf, Instant> = HashMap::new();
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.txt");
        debounce.insert(path.clone(), Instant::now());
        let skip = debounce
            .get(&path)
            .map(|t| t.elapsed().as_millis() < 500)
            .unwrap_or(false);
        assert!(skip, "should skip rapid event");
    }

    #[test]
    fn test_skip_hidden_files() {
        let root = std::env::temp_dir().join(format!(
            "hyphae-watch-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let project = root.join("project");
        std::fs::create_dir_all(&project).unwrap();
        let hidden = project.join(".hidden_file.txt");
        let visible = project.join("visible.txt");
        assert!(hyphae_ingest::should_skip(&hidden));
        assert!(!hyphae_ingest::should_skip(&visible));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn test_skip_build_dirs() {
        let in_target = Path::new("/project/target/debug/binary");
        let in_node_modules = Path::new("/project/node_modules/lodash/index.js");
        let in_git = Path::new("/project/.git/config");
        assert!(hyphae_ingest::should_skip(in_target));
        assert!(hyphae_ingest::should_skip(in_node_modules));
        assert!(hyphae_ingest::should_skip(in_git));
    }
}
