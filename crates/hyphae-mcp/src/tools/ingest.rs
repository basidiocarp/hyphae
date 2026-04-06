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

use super::{
    get_bounded_i64, get_str, normalize_identity, scoped_worktree_root, validate_required_string,
};

use hyphae_store::UnifiedSearchResult;

const COMMAND_OUTPUT_SCHEMA_VERSION: &str = "1.0";

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
    match args.get("schema_version").and_then(|value| value.as_str()) {
        Some(COMMAND_OUTPUT_SCHEMA_VERSION) => {}
        Some(version) => {
            return ToolResult::error(format!(
                "unsupported command output schema_version: {version} (expected {COMMAND_OUTPUT_SCHEMA_VERSION})"
            ));
        }
        None => {
            return ToolResult::error("missing required field: schema_version".to_string());
        }
    }

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
    let (project_root, worktree_id) =
        normalize_identity(get_str(args, "project_root"), get_str(args, "worktree_id"));
    let runtime_session_id = get_str(args, "runtime_session_id");

    // 1. Auto-detect output type and chunk
    let output_type = detect_output_type(output);
    let source_path = command_output_source_path(command, project_root, worktree_id);
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
        runtime_session_id: runtime_session_id.map(String::from),
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
    if project_root.is_none() {
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
    }

    // 6. Return result
    let result = json!({
        "summary": summary,
        "document_id": doc_id.to_string(),
        "chunk_count": chunk_count,
    });
    ToolResult::text(result.to_string())
}

fn command_output_source_path(
    command: &str,
    project_root: Option<&str>,
    worktree_id: Option<&str>,
) -> String {
    match (project_root, worktree_id) {
        (Some(project_root), Some(worktree_id)) => {
            format!("cmd://{project_root}::{worktree_id}::{command}")
        }
        _ => format!("cmd://{command}"),
    }
}

pub(crate) fn tool_get_command_chunks(store: &SqliteStore, args: &Value) -> ToolResult {
    let doc_id_str = match validate_required_string(args, "document_id") {
        Ok(id) => id,
        Err(e) => return e,
    };
    let offset = get_bounded_i64(args, "offset", 0, 0, 10000) as usize;
    let limit = get_bounded_i64(args, "limit", 5, 1, 20) as usize;

    let doc_id = DocumentId::from(doc_id_str);

    let runtime_session_id = match store.get_document(&doc_id) {
        Ok(Some(document)) => document.runtime_session_id,
        Ok(None) => None,
        Err(e) => return ToolResult::error(format!("db error: {e}")),
    };

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
        "runtime_session_id": runtime_session_id,
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
    let raw_project_root = get_str(args, "project_root");
    let raw_worktree_id = get_str(args, "worktree_id");
    if raw_project_root.is_some() ^ raw_worktree_id.is_some() {
        return ToolResult::error(
            "project_root and worktree_id must be provided together".to_string(),
        );
    }
    let (project_root, worktree_id) = normalize_identity(raw_project_root, raw_worktree_id);
    let scoped_worktree = scoped_worktree_root(project_root, worktree_id);

    let embedding = embedder.and_then(|emb| emb.embed(query).ok());
    let emb_ref = embedding.as_deref();

    let results = if let Some(worktree) = scoped_worktree {
        match store.search_all_scoped(
            query,
            emb_ref,
            limit,
            offset,
            include_docs,
            project,
            Some(worktree),
            None,
        ) {
            Ok(r) => r,
            Err(e) => return ToolResult::error(format!("search error: {e}")),
        }
    } else {
        match store.search_all(query, emb_ref, limit, offset, include_docs, project, None) {
            Ok(r) => r,
            Err(e) => return ToolResult::error(format!("search error: {e}")),
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use hyphae_core::{Chunk, ChunkId, ChunkStore, Importance, Memory, MemoryStore};
    use serde_json::json;

    fn test_store() -> SqliteStore {
        SqliteStore::in_memory().expect("in-memory store")
    }

    #[test]
    fn test_store_command_output_namespaces_source_path_for_identity_v1() {
        let store = test_store();

        let first = tool_store_command_output(
            &store,
            &json!({
                "schema_version": "1.0",
                "command": "cargo test",
                "output": "alpha output",
                "project": "demo",
                "project_root": "/repo/demo",
                "worktree_id": "wt-alpha"
            }),
            false,
            None,
        );
        assert!(!first.is_error);

        let second = tool_store_command_output(
            &store,
            &json!({
                "schema_version": "1.0",
                "command": "cargo test",
                "output": "beta output",
                "project": "demo",
                "project_root": "/repo/demo",
                "worktree_id": "wt-beta"
            }),
            false,
            None,
        );
        assert!(!second.is_error);

        let docs = store.list_documents(Some("demo")).unwrap();
        assert_eq!(docs.len(), 2);
        assert!(
            docs.iter()
                .any(|doc| { doc.source_path == "cmd:///repo/demo::wt-alpha::cargo test" })
        );
        assert!(
            docs.iter()
                .any(|doc| { doc.source_path == "cmd:///repo/demo::wt-beta::cargo test" })
        );
    }

    #[test]
    fn test_store_command_output_without_identity_preserves_legacy_replacement() {
        let store = test_store();

        let first = tool_store_command_output(
            &store,
            &json!({
                "schema_version": "1.0",
                "command": "cargo test",
                "output": "first output",
                "project": "demo"
            }),
            false,
            None,
        );
        assert!(!first.is_error);
        let first_parsed: Value = serde_json::from_str(&first.content[0].text).unwrap();
        let first_document_id = DocumentId::from(first_parsed["document_id"].as_str().unwrap());

        let second = tool_store_command_output(
            &store,
            &json!({
                "schema_version": "1.0",
                "command": "cargo test",
                "output": "second output",
                "project": "demo"
            }),
            false,
            None,
        );
        assert!(!second.is_error);
        let second_parsed: Value = serde_json::from_str(&second.content[0].text).unwrap();
        let second_document_id = DocumentId::from(second_parsed["document_id"].as_str().unwrap());

        let docs = store.list_documents(Some("demo")).unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].source_path, "cmd://cargo test");
        assert_ne!(first_document_id, second_document_id);
        assert!(store.get_document(&first_document_id).unwrap().is_none());

        let latest_chunks = store.get_chunks(&second_document_id).unwrap();
        assert!(!latest_chunks.is_empty());
        assert!(
            latest_chunks
                .iter()
                .any(|chunk| chunk.content.contains("second output"))
        );
    }

    #[test]
    fn test_store_command_output_identity_v1_skips_project_scoped_summary_memory() {
        let store = test_store();

        let result = tool_store_command_output(
            &store,
            &json!({
                "schema_version": "1.0",
                "command": "cargo test",
                "output": "alpha output",
                "project": "demo",
                "project_root": "/repo/demo",
                "worktree_id": "wt-alpha"
            }),
            false,
            None,
        );
        assert!(!result.is_error);

        let memories = store.search_fts("alpha", 10, 0, Some("demo")).unwrap();
        assert!(memories.is_empty());
    }

    #[test]
    fn test_store_command_output_rejects_missing_schema_version() {
        let store = test_store();

        let result = tool_store_command_output(
            &store,
            &json!({
                "command": "cargo test",
                "output": "alpha output",
                "project": "demo"
            }),
            false,
            None,
        );

        assert!(result.is_error);
        assert_eq!(
            result.content[0].text,
            "missing required field: schema_version"
        );
    }

    #[test]
    fn test_store_command_output_rejects_unknown_schema_version() {
        let store = test_store();

        let result = tool_store_command_output(
            &store,
            &json!({
                "schema_version": "2.0",
                "command": "cargo test",
                "output": "alpha output",
                "project": "demo"
            }),
            false,
            None,
        );

        assert!(result.is_error);
        assert_eq!(
            result.content[0].text,
            "unsupported command output schema_version: 2.0 (expected 1.0)"
        );
    }

    #[test]
    fn test_store_command_output_persists_runtime_session_id_and_returns_it_with_chunks() {
        let store = test_store();

        let result = tool_store_command_output(
            &store,
            &json!({
                "schema_version": "1.0",
                "command": "cargo test",
                "output": "alpha output",
                "project": "demo",
                "runtime_session_id": "claude-session-42"
            }),
            false,
            None,
        );
        assert!(!result.is_error);

        let stored: Value = serde_json::from_str(&result.content[0].text).unwrap();
        let doc_id = DocumentId::from(stored["document_id"].as_str().unwrap());
        let document = store.get_document(&doc_id).unwrap().unwrap();
        assert_eq!(
            document.runtime_session_id.as_deref(),
            Some("claude-session-42")
        );

        let chunks = tool_get_command_chunks(
            &store,
            &json!({
                "document_id": doc_id.to_string(),
                "offset": 0,
                "limit": 5
            }),
        );
        assert!(!chunks.is_error);
        let payload: Value = serde_json::from_str(&chunks.content[0].text).unwrap();
        assert_eq!(
            payload["runtime_session_id"].as_str(),
            Some("claude-session-42")
        );
    }

    #[test]
    fn test_search_all_rejects_partial_identity_pair() {
        let store = test_store();

        let result = tool_search_all(
            &store,
            None,
            &json!({
                "query": "target",
                "project_root": "/repo/demo"
            }),
            false,
            Some("demo"),
        );

        assert!(result.is_error);
        assert_eq!(
            result.content[0].text,
            "project_root and worktree_id must be provided together"
        );
    }

    #[test]
    fn test_search_all_scopes_memories_and_keeps_docs_project_scoped() {
        let store = test_store();
        store
            .store(
                Memory::builder(
                    "architecture".to_string(),
                    "Alpha scoped target".to_string(),
                    Importance::Medium,
                )
                .project("demo".to_string())
                .worktree("/repo/demo".to_string())
                .build(),
            )
            .unwrap();
        store
            .store(
                Memory::builder(
                    "architecture".to_string(),
                    "Beta other worktree target".to_string(),
                    Importance::Medium,
                )
                .project("demo".to_string())
                .worktree("/repo/other".to_string())
                .build(),
            )
            .unwrap();
        store
            .store(
                Memory::builder(
                    "architecture".to_string(),
                    "Shared scoped target".to_string(),
                    Importance::Medium,
                )
                .project(hyphae_store::SHARED_PROJECT.to_string())
                .build(),
            )
            .unwrap();

        let now = chrono::Utc::now();
        let mut doc = Document {
            id: DocumentId::new(),
            source_path: "docs/scoped.md".to_string(),
            source_type: SourceType::Markdown,
            chunk_count: 1,
            created_at: now,
            updated_at: now,
            project: Some("demo".to_string()),
            runtime_session_id: None,
        };
        doc.project = Some("demo".to_string());
        let doc_id = doc.id.clone();
        store.store_document(doc).unwrap();
        store
            .store_chunks(vec![Chunk {
                id: ChunkId::new(),
                document_id: doc_id,
                chunk_index: 0,
                content: "Scoped target document chunk".to_string(),
                metadata: ChunkMetadata {
                    source_path: "docs/scoped.md".to_string(),
                    source_type: SourceType::Markdown,
                    heading: None,
                    line_start: None,
                    line_end: None,
                    language: None,
                },
                embedding: None,
                created_at: chrono::Utc::now(),
            }])
            .unwrap();

        let result = tool_search_all(
            &store,
            None,
            &json!({
                "query": "target",
                "project_root": "/repo/demo",
                "worktree_id": "wt-alpha"
            }),
            false,
            Some("demo"),
        );

        assert!(!result.is_error);
        let output = &result.content[0].text;
        assert!(output.contains("Alpha scoped"));
        assert!(output.contains("Shared scoped"));
        assert!(output.contains("docs/scoped.md"));
        assert!(!output.contains("Beta other"));
    }
}
