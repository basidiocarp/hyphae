use anyhow::Result;
use hyphae_core::{ChunkStore, Embedder};
use hyphae_store::SqliteStore;
use std::path::PathBuf;

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
        store.search_chunks_hybrid(&query, &embedding, limit as usize, project.as_deref())?
    } else {
        store.search_chunks_fts(&query, limit as usize, project.as_deref())?
    };
    #[cfg(not(feature = "embeddings"))]
    let results = {
        let _ = embedder;
        store.search_chunks_fts(&query, limit as usize, project.as_deref())?
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

pub(crate) fn cmd_list_sources(store: &SqliteStore, project: Option<String>) -> Result<()> {
    let docs = store.list_documents(project.as_deref())?;
    if docs.is_empty() {
        println!("No sources ingested yet");
    } else {
        println!("{:<60} {:<10} {:<8} Ingested", "Path", "Type", "Chunks");
        println!("{}", "-".repeat(90));
        for doc in docs {
            println!(
                "{:<60} {:<10} {:<8} {}",
                doc.source_path,
                format!("{:?}", doc.source_type),
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
    let results = store.search_all(&query, emb_ref, limit, include_docs, project.as_deref())?;

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
