use std::path::Path;

use chrono::{Duration, Utc};
use serde_json::{Value, json};

use hyphae_core::chunk::ChunkMetadata;
use hyphae_core::{
    ChunkStore, Document, DocumentId, Embedder, Importance, Memory, MemoryStore, SourceType,
};
use hyphae_ingest::chunker::{ChunkStrategy, detect_output_type};
use hyphae_store::SqliteStore;

use crate::protocol::ToolResult;

use super::{get_bounded_i64, get_str, validate_required_string};

use hyphae_store::UnifiedSearchResult;

pub(crate) fn tool_ingest_file(
    store: &SqliteStore,
    embedder: Option<&dyn Embedder>,
    args: &Value,
    _compact: bool,
    project: Option<&str>,
) -> ToolResult {
    let path_str = match validate_required_string(args, "path") {
        Ok(s) => s,
        Err(e) => return e,
    };
    let recursive = args
        .get("recursive")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let path = Path::new(path_str);

    let results = if path.is_dir() {
        match hyphae_ingest::ingest_directory(path, embedder, recursive) {
            Ok(v) => v,
            Err(e) => return ToolResult::error(format!("ingestion error: {e}")),
        }
    } else {
        match hyphae_ingest::ingest_file(path, embedder) {
            Ok(pair) => vec![pair],
            Err(e) => return ToolResult::error(format!("ingestion error: {e}")),
        }
    };

    let mut total_chunks = 0usize;
    let mut doc_count = 0usize;

    for (mut doc, chunks) in results {
        doc.project = project.map(String::from);
        // Replace existing document at the same path
        if let Ok(Some(existing)) = store.get_document_by_path(&doc.source_path, project) {
            if let Err(e) = store.delete_document(&existing.id) {
                return ToolResult::error(format!(
                    "failed to delete existing document {}: {e}",
                    doc.source_path
                ));
            }
        }
        if let Err(e) = store.store_document(doc) {
            return ToolResult::error(format!("store error: {e}"));
        }
        let n = chunks.len();
        if let Err(e) = store.store_chunks(chunks) {
            return ToolResult::error(format!("store error: {e}"));
        }
        total_chunks += n;
        doc_count += 1;
    }

    ToolResult::text(format!(
        "Ingested {doc_count} document(s), {total_chunks} chunk(s) total"
    ))
}

pub(crate) fn tool_search_docs(
    store: &SqliteStore,
    embedder: Option<&dyn Embedder>,
    args: &Value,
    compact: bool,
    project: Option<&str>,
) -> ToolResult {
    let query = match validate_required_string(args, "query") {
        Ok(q) => q,
        Err(e) => return e,
    };
    let limit = get_bounded_i64(args, "limit", 10, 1, 100) as usize;
    let offset = get_bounded_i64(args, "offset", 0, 0, 10000) as usize;

    let results = if let Some(emb) = embedder {
        match emb.embed(query) {
            Ok(embedding) => {
                match store.search_chunks_hybrid(query, &embedding, limit, offset, project) {
                    Ok(r) => r,
                    Err(e) => return ToolResult::error(format!("search error: {e}")),
                }
            }
            Err(_) => match store.search_chunks_fts(query, limit, offset, project) {
                Ok(r) => r,
                Err(e) => return ToolResult::error(format!("search error: {e}")),
            },
        }
    } else {
        match store.search_chunks_fts(query, limit, offset, project) {
            Ok(r) => r,
            Err(e) => return ToolResult::error(format!("search error: {e}")),
        }
    };

    if results.is_empty() {
        return ToolResult::text("No results found.".to_string());
    }

    let max_content = if compact { 200 } else { 400 };
    let mut out = String::new();
    for (i, r) in results.iter().enumerate() {
        let chunk = &r.chunk;
        let meta = &chunk.metadata;
        let lines = match (meta.line_start, meta.line_end) {
            (Some(s), Some(e)) => format!(" (lines {s}-{e})"),
            (Some(s), None) => format!(" (line {s})"),
            _ => String::new(),
        };
        let snippet = if chunk.content.len() > max_content {
            format!("{}…", &chunk.content[..max_content])
        } else {
            chunk.content.clone()
        };
        out.push_str(&format!(
            "{}. [score={:.3}] {}{}\n{}\n\n",
            i + 1,
            r.score,
            meta.source_path,
            lines,
            snippet,
        ));
    }

    ToolResult::text(out.trim_end().to_string())
}

pub(crate) fn tool_list_sources(store: &SqliteStore, project: Option<&str>) -> ToolResult {
    let docs = match store.list_documents(project) {
        Ok(d) => d,
        Err(e) => return ToolResult::error(format!("db error: {e}")),
    };

    if docs.is_empty() {
        return ToolResult::text("No sources ingested.".to_string());
    }

    let mut out = format!(
        "{:<50} {:<10} {:<8} {}\n",
        "Path", "Type", "Chunks", "Ingested"
    );
    out.push_str(&"-".repeat(90));
    out.push('\n');
    for doc in &docs {
        out.push_str(&format!(
            "{:<50} {:<10} {:<8} {}\n",
            truncate_path(&doc.source_path, 50),
            format!("{:?}", doc.source_type).to_lowercase(),
            doc.chunk_count,
            doc.created_at.format("%Y-%m-%d"),
        ));
    }

    ToolResult::text(out)
}

pub(crate) fn tool_forget_source(
    store: &SqliteStore,
    args: &Value,
    project: Option<&str>,
) -> ToolResult {
    let path = match validate_required_string(args, "path") {
        Ok(p) => p,
        Err(e) => return e,
    };

    let doc = match store.get_document_by_path(path, project) {
        Ok(Some(d)) => d,
        Ok(None) => return ToolResult::error(format!("Source not found: {path}")),
        Err(e) => return ToolResult::error(format!("db error: {e}")),
    };

    match store.delete_document(&doc.id) {
        Ok(()) => ToolResult::text(format!("Deleted source: {path}")),
        Err(e) => ToolResult::error(format!("delete error: {e}")),
    }
}

fn truncate_path(path: &str, max: usize) -> String {
    if path.len() <= max {
        path.to_string()
    } else {
        format!("…{}", &path[path.len() - (max - 1)..])
    }
}

pub(crate) fn tool_store_command_output(
    store: &SqliteStore,
    args: &Value,
    _compact: bool,
    project: Option<&str>,
) -> ToolResult {
    let command = match validate_required_string(args, "command") {
        Ok(c) => c,
        Err(e) => return e,
    };
    let output = match validate_required_string(args, "output") {
        Ok(o) => o,
        Err(e) => return e,
    };
    let ttl_hours = get_bounded_i64(args, "ttl_hours", 4, 1, 168);
    let project_override = get_str(args, "project");
    let effective_project = project_override.or(project);

    // 1. Auto-detect output type and chunk
    let output_type = detect_output_type(output);
    let source_path = format!("cmd://{command}");
    let metadata = ChunkMetadata {
        source_path: source_path.clone(),
        source_type: SourceType::Text,
        language: None,
        heading: None,
        line_start: None,
        line_end: None,
    };
    let strategy = ChunkStrategy::ByStructuredOutput {
        output_type,
        max_tokens: 500,
    };
    let mut chunks = hyphae_ingest::chunker::chunk_text(output, metadata, strategy);

    // 2. Create document
    let now = Utc::now();
    let doc_id = DocumentId::new();
    let chunk_count = chunks.len();

    let doc = Document {
        id: doc_id.clone(),
        source_path,
        source_type: SourceType::Text,
        chunk_count,
        created_at: now,
        updated_at: now,
        project: effective_project.map(String::from),
    };

    // Replace existing document at the same source path
    if let Ok(Some(existing)) = store.get_document_by_path(&doc.source_path, effective_project) {
        if let Err(e) = store.delete_document(&existing.id) {
            return ToolResult::error(format!("failed to delete existing document: {e}"));
        }
    }

    // 3. Fix chunk document_ids to point to our new document
    for chunk in &mut chunks {
        chunk.document_id = doc_id.clone();
    }

    // 4. Store document + chunks
    if let Err(e) = store.store_document(doc) {
        return ToolResult::error(format!("store error: {e}"));
    }
    if let Err(e) = store.store_chunks(chunks) {
        return ToolResult::error(format!("store error: {e}"));
    }

    // 5. Create summary memory with Ephemeral importance
    let first_lines: String = output.lines().take(3).collect::<Vec<_>>().join("\n");
    let summary = format!("Command `{command}` output ({chunk_count} chunks):\n{first_lines}");

    let expires_at = Utc::now() + Duration::hours(ttl_hours);
    let mut builder = Memory::builder(
        "command_output".to_string(),
        summary.clone(),
        Importance::Ephemeral,
    )
    .keywords(vec![command.to_string()])
    .expires_at(expires_at);

    if let Some(p) = effective_project {
        builder = builder.project(p.to_string());
    }

    let memory = builder.build();
    if let Err(e) = store.store(memory) {
        tracing::warn!("failed to store summary memory: {e}");
    }

    // 6. Return result
    let result = json!({
        "summary": summary,
        "document_id": doc_id.to_string(),
        "chunk_count": chunk_count,
    });
    ToolResult::text(result.to_string())
}

pub(crate) fn tool_get_command_chunks(store: &SqliteStore, args: &Value) -> ToolResult {
    let doc_id_str = match validate_required_string(args, "document_id") {
        Ok(id) => id,
        Err(e) => return e,
    };
    let offset = get_bounded_i64(args, "offset", 0, 0, 10000) as usize;
    let limit = get_bounded_i64(args, "limit", 5, 1, 20) as usize;

    let doc_id = DocumentId::from(doc_id_str);

    // Get all chunks for the document (already sorted by chunk_index from store)
    let chunks = match store.get_chunks(&doc_id) {
        Ok(c) => c,
        Err(e) => return ToolResult::error(format!("db error: {e}")),
    };

    let total = chunks.len();
    let paginated: Vec<Value> = chunks
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|c| {
            json!({
                "index": c.chunk_index,
                "content": c.content,
                "heading": c.metadata.heading,
            })
        })
        .collect();

    let has_more = offset + limit < total;

    let result = json!({
        "chunks": paginated,
        "total": total,
        "offset": offset,
        "has_more": has_more,
    });
    ToolResult::text(result.to_string())
}

pub(crate) fn tool_search_all(
    store: &SqliteStore,
    embedder: Option<&dyn Embedder>,
    args: &Value,
    compact: bool,
    project: Option<&str>,
) -> ToolResult {
    let query = match validate_required_string(args, "query") {
        Ok(q) => q,
        Err(e) => return e,
    };
    let limit = get_bounded_i64(args, "limit", 10, 1, 50) as usize;
    let offset = get_bounded_i64(args, "offset", 0, 0, 10000) as usize;
    let include_docs = args
        .get("include_docs")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    let embedding = embedder.and_then(|emb| emb.embed(query).ok());
    let emb_ref = embedding.as_deref();

    let results = match store.search_all(query, emb_ref, limit, offset, include_docs, project) {
        Ok(r) => r,
        Err(e) => return ToolResult::error(format!("search error: {e}")),
    };

    if results.is_empty() {
        return ToolResult::text("No results found.".to_string());
    }

    let max_content = if compact { 150 } else { 300 };
    let mut out = String::new();
    for (i, r) in results.iter().enumerate() {
        match r {
            UnifiedSearchResult::Memory { memory, score } => {
                if compact {
                    out.push_str(&format!(
                        "{}. [memory] [{}] {}\n",
                        i + 1,
                        memory.topic,
                        memory.summary
                    ));
                } else {
                    out.push_str(&format!(
                        "{}. [memory] [score={:.3}] topic={}\n  {}\n",
                        i + 1,
                        score,
                        memory.topic,
                        memory.summary,
                    ));
                    if !memory.keywords.is_empty() {
                        out.push_str(&format!("  keywords: {}\n", memory.keywords.join(", ")));
                    }
                    out.push('\n');
                }
            }
            UnifiedSearchResult::Chunk { chunk, score } => {
                let meta = &chunk.metadata;
                let lines = match (meta.line_start, meta.line_end) {
                    (Some(s), Some(e)) => format!(":{s}-{e}"),
                    (Some(s), None) => format!(":{s}"),
                    _ => String::new(),
                };
                let snippet = if chunk.content.len() > max_content {
                    format!("{}…", &chunk.content[..max_content])
                } else {
                    chunk.content.clone()
                };
                if compact {
                    out.push_str(&format!(
                        "{}. [doc: {}{}] {}\n",
                        i + 1,
                        meta.source_path,
                        lines,
                        snippet.replace('\n', " "),
                    ));
                } else {
                    out.push_str(&format!(
                        "{}. [doc: {}{}] [score={:.3}]\n  {}\n\n",
                        i + 1,
                        meta.source_path,
                        lines,
                        score,
                        snippet,
                    ));
                }
            }
        }
    }

    ToolResult::text(out.trim_end().to_string())
}
