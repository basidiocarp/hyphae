pub mod chunker;
pub mod readers;
pub mod session;
pub mod transcript;

use chrono::Utc;
use hyphae_core::chunk::{ChunkMetadata, Document};
use hyphae_core::error::HyphaeResult;
use hyphae_core::ids::DocumentId;
use hyphae_core::{Chunk, Embedder};
use std::path::Path;
use walkdir::WalkDir;

use crate::chunker::{ChunkStrategy, chunk_text};
use crate::readers::read_file;

/// Ingest a single file: read, chunk, and optionally embed.
pub fn ingest_file(
    path: &Path,
    embedder: Option<&dyn Embedder>,
) -> HyphaeResult<(Document, Vec<Chunk>)> {
    let (content, source_type) = read_file(path)?;

    let metadata = ChunkMetadata {
        source_path: path.to_string_lossy().to_string(),
        source_type: source_type.clone(),
        language: None,
        heading: None,
        line_start: None,
        line_end: None,
    };

    let strategy = ChunkStrategy::for_source_type(&source_type);
    let mut chunks = chunk_text(&content, metadata, strategy);

    // Embed chunk contents if an embedder is provided
    if let Some(emb) = embedder {
        let contents: Vec<&str> = chunks.iter().map(|c| c.content.as_str()).collect();
        if !contents.is_empty() {
            let embeddings = emb.embed_batch(&contents)?;
            for (chunk, embedding) in chunks.iter_mut().zip(embeddings) {
                chunk.embedding = Some(embedding);
            }
        }
    }

    let doc_id = DocumentId::new();
    let now = Utc::now();

    // Chunks are created without a document_id; bind them before storing.
    for chunk in &mut chunks {
        chunk.document_id = doc_id.clone();
    }

    let document = Document {
        id: doc_id,
        source_path: path.to_string_lossy().to_string(),
        source_type,
        chunk_count: chunks.len(),
        created_at: now,
        updated_at: now,
        project: None,
        runtime_session_id: None,
    };

    Ok((document, chunks))
}

const SKIP_DIRS: &[&str] = &["target", "node_modules", ".git"];

/// Ingest all files in a directory.
///
/// Hidden files/directories (starting with `.`) and common build directories
/// are skipped. Errors on individual files are logged and skipped.
pub fn ingest_directory(
    path: &Path,
    embedder: Option<&dyn Embedder>,
    recursive: bool,
) -> HyphaeResult<Vec<(Document, Vec<Chunk>)>> {
    let walker = if recursive {
        WalkDir::new(path)
    } else {
        WalkDir::new(path).max_depth(1)
    };

    let mut results = Vec::new();

    for entry in walker
        .into_iter()
        .filter_entry(|e| !is_skipped_dir(e))
        .flatten()
    {
        if !entry.file_type().is_file() {
            continue;
        }

        // Skip hidden files
        if let Some(name) = entry.file_name().to_str() {
            if name.starts_with('.') {
                continue;
            }
        }

        match ingest_file(entry.path(), embedder) {
            Ok(result) => results.push(result),
            Err(e) => {
                tracing::warn!(
                    path = %entry.path().display(),
                    error = %e,
                    "skipping file during directory ingestion"
                );
            }
        }
    }

    Ok(results)
}

fn is_skipped_dir(entry: &walkdir::DirEntry) -> bool {
    // Don't filter the root directory (depth 0) or non-directories
    if entry.depth() == 0 || !entry.file_type().is_dir() {
        return false;
    }
    let name = entry.file_name().to_string_lossy();
    name.starts_with('.') || SKIP_DIRS.contains(&name.as_ref())
}

/// Returns true if the given path should be skipped during ingestion.
///
/// Skips paths that contain hidden components (starting with `.`) or known
/// build/dependency directories (`target`, `node_modules`, `.git`).
pub fn should_skip(path: &Path) -> bool {
    path.components().any(|c| {
        if let std::path::Component::Normal(name) = c {
            let name = name.to_string_lossy();
            name.starts_with('.') || SKIP_DIRS.contains(&name.as_ref())
        } else {
            false
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use hyphae_core::chunk::SourceType;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_dir() -> TempDir {
        let dir = TempDir::new().unwrap();

        // Markdown file
        fs::write(
            dir.path().join("readme.md"),
            "# Hello\n\nWorld content.\n\n## Section\n\nMore text.",
        )
        .unwrap();

        // Rust file
        fs::write(
            dir.path().join("main.rs"),
            "fn main() {\n    println!(\"hi\");\n}\n\nfn helper() {\n    // help\n}",
        )
        .unwrap();

        // Text file
        fs::write(dir.path().join("notes.txt"), "Some plain text notes.").unwrap();

        // Hidden file (should be skipped)
        fs::write(dir.path().join(".hidden"), "secret").unwrap();

        // Subdirectory
        let sub = dir.path().join("sub");
        fs::create_dir(&sub).unwrap();
        fs::write(sub.join("nested.py"), "def foo():\n    pass").unwrap();

        // node_modules (should be skipped)
        let nm = dir.path().join("node_modules");
        fs::create_dir(&nm).unwrap();
        fs::write(nm.join("pkg.js"), "module.exports = {}").unwrap();

        dir
    }

    #[test]
    fn test_ingest_file_end_to_end() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.md");
        fs::write(
            &path,
            "# Title\n\nSome intro content here.\n\n## Details\n\nMore detailed information.",
        )
        .unwrap();

        let (doc, chunks) = ingest_file(&path, None).unwrap();

        assert_eq!(doc.source_type, SourceType::Markdown);
        assert_eq!(doc.chunk_count, chunks.len());
        assert!(!chunks.is_empty());
        // All chunks should reference the document
        for chunk in &chunks {
            assert_eq!(chunk.document_id, doc.id);
            assert!(chunk.embedding.is_none());
        }
    }

    #[test]
    fn test_ingest_file_code() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("example.rs");
        fs::write(&path, "fn alpha() {\n    1\n}\n\nfn beta() {\n    2\n}").unwrap();

        let (doc, chunks) = ingest_file(&path, None).unwrap();

        assert_eq!(doc.source_type, SourceType::Code);
        assert_eq!(chunks.len(), 2);
        assert!(chunks[0].content.contains("fn alpha()"));
        assert!(chunks[1].content.contains("fn beta()"));
    }

    #[test]
    fn test_ingest_directory_non_recursive() {
        let dir = create_test_dir();

        let results = ingest_directory(dir.path(), None, false).unwrap();

        // Should get readme.md, main.rs, notes.txt (not .hidden, not sub/, not node_modules/)
        assert_eq!(results.len(), 3, "expected 3 files, got {}", results.len());

        let paths: Vec<&str> = results
            .iter()
            .map(|(doc, _)| doc.source_path.as_str())
            .collect();

        // Verify no hidden or skipped files
        for p in &paths {
            assert!(!p.contains(".hidden"));
            assert!(!p.contains("node_modules"));
        }
    }

    #[test]
    fn test_ingest_directory_recursive() {
        let dir = create_test_dir();

        let results = ingest_directory(dir.path(), None, true).unwrap();

        // Should get readme.md, main.rs, notes.txt, sub/nested.py (not .hidden, not node_modules/)
        assert_eq!(results.len(), 4, "expected 4 files, got {}", results.len());

        let has_nested = results
            .iter()
            .any(|(doc, _)| doc.source_path.contains("nested.py"));
        assert!(has_nested, "should include nested file in recursive mode");
    }

    #[test]
    fn test_ingest_directory_skips_hidden_and_build_dirs() {
        let dir = create_test_dir();

        let results = ingest_directory(dir.path(), None, true).unwrap();

        for (doc, _) in &results {
            assert!(
                !doc.source_path.contains("node_modules"),
                "should skip node_modules"
            );
            assert!(
                !doc.source_path.contains(".hidden"),
                "should skip hidden files"
            );
        }
    }
}
