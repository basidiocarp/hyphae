use anyhow::Result;
use hyphae_core::{ChunkStore, Document, Embedder};
use hyphae_store::SqliteStore;
use serde::Serialize;
use std::path::PathBuf;

const SOURCES_SCHEMA_VERSION: &str = "1.0";

#[derive(Serialize)]
struct DocumentPayload {
    id: String,
    source_path: String,
    source_type: String,
    chunk_count: usize,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    project: Option<String>,
}

#[derive(Serialize)]
struct SourcesPayload {
    project: Option<String>,
    total_sources: usize,
    total_chunks: usize,
    sources: Vec<DocumentPayload>,
}

#[derive(Serialize)]
struct VersionedPayload<'a, T: Serialize> {
    schema_version: &'a str,
    #[serde(flatten)]
    payload: &'a T,
}

pub(crate) fn cmd_ingest(
    store: &SqliteStore,
    path: PathBuf,
    recursive: bool,
    force: bool,
    project: Option<String>,
    embedder: Option<&dyn Embedder>,
) -> Result<()> {
    let pairs: Vec<(hyphae_core::Document, Vec<hyphae_core::Chunk>)> = if path.is_dir() {
        hyphae_ingest::ingest_directory(&path, embedder, recursive)?
    } else {
        vec![hyphae_ingest::ingest_file(&path, embedder)?]
    };

    let mut ingested = 0usize;
    let mut skipped = 0usize;
    for (mut doc, chunks) in pairs {
        doc.project = project.clone();
        let existing = store.get_document_by_path(&doc.source_path, project.as_deref())?;
        if let Some(existing_doc) = existing {
            if !force {
                println!(
                    "Already ingested: {}, use --force to re-ingest",
                    doc.source_path
                );
                skipped += 1;
                continue;
            }
            store.delete_document(&existing_doc.id)?;
        }
        let n = chunks.len();
        store.store_document(doc.clone())?;
        store.store_chunks(chunks)?;
        println!("✓ Ingested {}: {} chunks", doc.source_path, n);
        ingested += 1;
    }
    if skipped > 0 {
        println!("Skipped {} already-ingested source(s)", skipped);
    }
    println!("Done: {} source(s) ingested", ingested);
    Ok(())
}

pub(crate) fn cmd_search_docs(
    store: &SqliteStore,
    query: String,
    limit: u32,
    project: Option<String>,
    embedder: Option<&dyn Embedder>,
) -> Result<()> {
    #[cfg(feature = "embeddings")]
    let results = if let Some(e) = embedder {
        let embedding = e.embed(&query)?;
        store.search_chunks_hybrid(&query, &embedding, limit as usize, 0, project.as_deref())?
    } else {
        store.search_chunks_fts(&query, limit as usize, 0, project.as_deref())?
    };
    #[cfg(not(feature = "embeddings"))]
    let results = {
        let _ = embedder;
        store.search_chunks_fts(&query, limit as usize, 0, project.as_deref())?
    };

    if results.is_empty() {
        println!("No documents found");
    } else {
        for result in results {
            let meta = &result.chunk.metadata;
            let lines = match (meta.line_start, meta.line_end) {
                (Some(s), Some(e)) => format!(" (lines {s}-{e})"),
                (Some(s), None) => format!(" (line {s})"),
                _ => String::new(),
            };
            let snippet = if result.chunk.content.len() > 200 {
                format!("{}…", &result.chunk.content[..200])
            } else {
                result.chunk.content.clone()
            };
            println!(
                "[{:.2}] {}{}\n  {}",
                result.score, meta.source_path, lines, snippet
            );
        }
    }
    Ok(())
}

pub(crate) fn cmd_list_sources(
    store: &SqliteStore,
    json: bool,
    project: Option<String>,
) -> Result<()> {
    let payload = list_sources_payload(store, project.as_deref())?;
    if json {
        print_json_versioned(SOURCES_SCHEMA_VERSION, &payload)?;
        return Ok(());
    }

    if payload.sources.is_empty() {
        println!("No sources ingested yet");
    } else {
        println!("{:<60} {:<10} {:<8} Ingested", "Path", "Type", "Chunks");
        println!("{}", "-".repeat(90));
        for doc in payload.sources {
            println!(
                "{:<60} {:<10} {:<8} {}",
                doc.source_path,
                doc.source_type,
                doc.chunk_count,
                doc.created_at.format("%Y-%m-%d %H:%M"),
            );
        }
    }
    Ok(())
}

pub(crate) fn cmd_forget_source(
    store: &SqliteStore,
    path: String,
    project: Option<String>,
) -> Result<()> {
    match store.get_document_by_path(&path, project.as_deref())? {
        None => eprintln!("Source not found: {path}"),
        Some(doc) => {
            store.delete_document(&doc.id)?;
            println!("✓ Removed source: {path}");
        }
    }
    Ok(())
}

pub(crate) fn cmd_search_all(
    store: &SqliteStore,
    query: String,
    limit: usize,
    include_docs: bool,
    project: Option<String>,
    embedder: Option<&dyn Embedder>,
) -> Result<()> {
    #[cfg(feature = "embeddings")]
    let embedding = embedder.and_then(|e| e.embed(&query).ok());
    #[cfg(not(feature = "embeddings"))]
    let embedding: Option<Vec<f32>> = {
        let _ = embedder;
        None
    };

    let emb_ref = embedding.as_deref();
    let results = store.search_all(
        &query,
        emb_ref,
        limit,
        0,
        include_docs,
        project.as_deref(),
        None,
    )?;

    if results.is_empty() {
        println!("No results found");
    } else {
        for (i, r) in results.iter().enumerate() {
            match r {
                hyphae_store::UnifiedSearchResult::Memory { memory, score } => {
                    println!(
                        "{}. [memory] [{:.3}] [{}] {}",
                        i + 1,
                        score,
                        memory.topic,
                        memory.summary,
                    );
                }
                hyphae_store::UnifiedSearchResult::Chunk { chunk, score } => {
                    let meta = &chunk.metadata;
                    let lines = match (meta.line_start, meta.line_end) {
                        (Some(s), Some(e)) => format!(":{s}-{e}"),
                        (Some(s), None) => format!(":{s}"),
                        _ => String::new(),
                    };
                    let snippet = if chunk.content.len() > 200 {
                        format!("{}…", &chunk.content[..200])
                    } else {
                        chunk.content.clone()
                    };
                    println!(
                        "{}. [doc: {}{}] [{:.3}]\n  {}",
                        i + 1,
                        meta.source_path,
                        lines,
                        score,
                        snippet,
                    );
                }
            }
        }
    }
    Ok(())
}

fn list_sources_payload(store: &SqliteStore, project: Option<&str>) -> Result<SourcesPayload> {
    let docs = store.list_documents(project)?;
    let total_chunks = docs.iter().map(|doc| doc.chunk_count).sum();
    Ok(SourcesPayload {
        project: project.map(str::to_string),
        total_sources: docs.len(),
        total_chunks,
        sources: docs.iter().map(to_document_payload).collect(),
    })
}

fn to_document_payload(doc: &Document) -> DocumentPayload {
    DocumentPayload {
        id: doc.id.to_string(),
        source_path: doc.source_path.clone(),
        source_type: doc.source_type.to_string(),
        chunk_count: doc.chunk_count,
        created_at: doc.created_at,
        updated_at: doc.updated_at,
        project: doc.project.clone(),
    }
}

fn print_json<T: Serialize>(payload: &T) -> Result<()> {
    println!("{}", serde_json::to_string(payload)?);
    Ok(())
}

fn print_json_versioned<T: Serialize>(schema_version: &'static str, payload: &T) -> Result<()> {
    let versioned = VersionedPayload {
        schema_version,
        payload,
    };
    print_json(&versioned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use hyphae_core::{DocumentId, SourceType};

    fn test_store() -> SqliteStore {
        SqliteStore::in_memory().expect("in-memory store")
    }

    fn make_document(path: &str, source_type: SourceType, chunk_count: usize) -> Document {
        let now = Utc::now();
        Document {
            id: DocumentId::new(),
            source_path: path.to_string(),
            source_type,
            chunk_count,
            created_at: now,
            updated_at: now,
            project: Some("demo-project".to_string()),
            runtime_session_id: None,
        }
    }

    #[test]
    fn test_list_sources_payload_returns_structured_sources() {
        let store = test_store();
        store
            .store_document(make_document("src/lib.rs", SourceType::Code, 3))
            .unwrap();
        store
            .store_document(make_document("docs/readme.md", SourceType::Markdown, 2))
            .unwrap();

        let payload = list_sources_payload(&store, Some("demo-project")).unwrap();
        let value = serde_json::to_value(&payload).unwrap();

        assert_eq!(value["project"].as_str(), Some("demo-project"));
        assert_eq!(value["total_sources"].as_u64(), Some(2));
        assert_eq!(value["total_chunks"].as_u64(), Some(5));
        assert_eq!(
            value["sources"][0]["source_path"].as_str(),
            Some("docs/readme.md")
        );
        assert_eq!(
            value["sources"][0]["source_type"].as_str(),
            Some("markdown")
        );
        assert_eq!(
            value["sources"][1]["source_path"].as_str(),
            Some("src/lib.rs")
        );
        assert_eq!(value["sources"][1]["source_type"].as_str(), Some("code"));
    }
}
