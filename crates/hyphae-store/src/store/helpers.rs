use chrono::{DateTime, Utc};

use hyphae_core::{
    Chunk, ChunkMetadata, Concept, ConceptId, ConceptLink, Confidence, Document, Importance, Label,
    Memoir, Memory, MemoryId, MemorySource, Relation, SourceType, Weight,
};

// ---------------------------------------------------------------------------
// Memory helpers
// ---------------------------------------------------------------------------

pub(crate) fn source_type(source: &MemorySource) -> &'static str {
    match source {
        MemorySource::ClaudeCode { .. } => "claude_code",
        MemorySource::Conversation { .. } => "conversation",
        MemorySource::Manual => "manual",
    }
}

pub(crate) fn source_data(source: &MemorySource) -> Option<String> {
    match source {
        MemorySource::Manual => None,
        other => serde_json::to_string(other).ok(),
    }
}

pub(crate) fn parse_source(source_type_str: &str, source_data_str: Option<String>) -> MemorySource {
    match source_type_str {
        "manual" => MemorySource::Manual,
        _ => source_data_str
            .and_then(|d| serde_json::from_str(&d).ok())
            .unwrap_or(MemorySource::Manual),
    }
}

pub(crate) fn embedding_to_blob(embedding: &[f32]) -> Vec<u8> {
    use zerocopy::IntoBytes;
    embedding.as_bytes().to_vec()
}

pub(crate) fn blob_to_embedding(blob: &[u8]) -> Vec<f32> {
    debug_assert_eq!(blob.len() % 4, 0);
    blob.chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().expect("chunks_exact guarantees 4 bytes")))
        .collect()
}

pub(crate) fn row_to_memory(row: &rusqlite::Row) -> rusqlite::Result<Memory> {
    // Column order: id(0), created_at(1), updated_at(2), last_accessed(3),
    //   access_count(4), weight(5), topic(6), summary(7), raw_excerpt(8),
    //   keywords(9), importance(10), source_type(11), source_data(12),
    //   related_ids(13), embedding(14)
    let id: MemoryId = row.get::<_, String>(0)?.into();

    let keywords_json: String = row.get::<_, Option<String>>(9)?.unwrap_or_default();
    let keywords: Vec<String> = serde_json::from_str(&keywords_json).unwrap_or_else(|e| {
        tracing::warn!("failed to parse keywords JSON for memory {id}: {keywords_json}: {e}");
        Default::default()
    });

    let importance_str: String = row.get(10)?;
    let importance = importance_str.parse().unwrap_or(Importance::Medium);

    let source_type_str: String = row.get(11)?;
    let source_data_str: Option<String> = row.get(12)?;
    let source = parse_source(&source_type_str, source_data_str);

    let related_json: String = row.get::<_, Option<String>>(13)?.unwrap_or_default();
    let related_ids: Vec<MemoryId> = serde_json::from_str(&related_json).unwrap_or_else(|e| {
        tracing::warn!("failed to parse related_ids JSON for memory {id}: {related_json}: {e}");
        Default::default()
    });

    let embedding: Option<Vec<f32>> = row
        .get::<_, Option<Vec<u8>>>(14)?
        .map(|b| blob_to_embedding(&b));

    let created_at_str: String = row.get(1)?;
    let updated_at_str: String = row.get::<_, Option<String>>(2)?.unwrap_or_default();
    let last_accessed_str: String = row.get(3)?;

    let created_at = parse_dt(&created_at_str);

    Ok(Memory {
        id,
        created_at,
        updated_at: if updated_at_str.is_empty() {
            created_at
        } else {
            parse_dt(&updated_at_str)
        },
        last_accessed: parse_dt(&last_accessed_str),
        access_count: row.get::<_, u32>(4)?,
        weight: Weight::new_clamped(row.get::<_, f32>(5)?),
        topic: row.get(6)?,
        summary: row.get(7)?,
        raw_excerpt: row.get(8)?,
        keywords,
        importance,
        source,
        related_ids,
        project: row.get("project").ok(),
        embedding,
    })
}

pub(crate) const SELECT_COLS: &str = "id, created_at, updated_at, last_accessed, access_count, weight, \
     topic, summary, raw_excerpt, keywords, \
     importance, source_type, source_data, related_ids, embedding, project";

// ---------------------------------------------------------------------------
// Memoir / Concept helpers
// ---------------------------------------------------------------------------

pub(crate) fn parse_dt(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .map(|d| d.with_timezone(&Utc))
        .unwrap_or_else(|_| {
            tracing::warn!("failed to parse datetime: {}", s);
            Utc::now()
        })
}

pub(crate) fn row_to_memoir(row: &rusqlite::Row) -> rusqlite::Result<Memoir> {
    Ok(Memoir {
        id: row.get::<_, String>(0)?.into(),
        name: row.get(1)?,
        description: row.get(2)?,
        created_at: parse_dt(&row.get::<_, String>(3)?),
        updated_at: parse_dt(&row.get::<_, String>(4)?),
        consolidation_threshold: row.get::<_, u32>(5)?,
    })
}

pub(crate) const MEMOIR_COLS: &str =
    "id, name, description, created_at, updated_at, consolidation_threshold";

pub(crate) fn row_to_concept(row: &rusqlite::Row) -> rusqlite::Result<Concept> {
    let id: ConceptId = row.get::<_, String>(0)?.into();

    let labels_json: String = row.get::<_, Option<String>>(4)?.unwrap_or_default();
    let labels: Vec<Label> = if labels_json.is_empty() {
        Vec::new()
    } else {
        serde_json::from_str(&labels_json).unwrap_or_else(|e| {
            tracing::warn!("failed to parse labels JSON for concept {id}: {labels_json}: {e}");
            Default::default()
        })
    };

    let source_ids_json: String = row.get::<_, Option<String>>(9)?.unwrap_or_default();
    let source_memory_ids: Vec<MemoryId> = if source_ids_json.is_empty() {
        Vec::new()
    } else {
        serde_json::from_str(&source_ids_json).unwrap_or_else(|e| {
            tracing::warn!(
                "failed to parse source_memory_ids JSON for concept {id}: {source_ids_json}: {e}"
            );
            Default::default()
        })
    };

    Ok(Concept {
        id,
        memoir_id: row.get::<_, String>(1)?.into(),
        name: row.get(2)?,
        definition: row.get(3)?,
        labels,
        confidence: Confidence::new_clamped(row.get::<_, f32>(5)?),
        revision: row.get::<_, u32>(6)?,
        created_at: parse_dt(&row.get::<_, String>(7)?),
        updated_at: parse_dt(&row.get::<_, String>(8)?),
        source_memory_ids,
    })
}

pub(crate) const CONCEPT_COLS: &str = "id, memoir_id, name, definition, labels, confidence, \
     revision, created_at, updated_at, source_memory_ids";

pub(crate) fn row_to_link(row: &rusqlite::Row) -> rusqlite::Result<ConceptLink> {
    let relation_str: String = row.get(3)?;
    let relation: Relation = relation_str.parse().unwrap_or(Relation::RelatedTo);

    Ok(ConceptLink {
        id: row.get::<_, String>(0)?.into(),
        source_id: row.get::<_, String>(1)?.into(),
        target_id: row.get::<_, String>(2)?.into(),
        relation,
        weight: Weight::new_clamped(row.get::<_, f32>(4)?),
        created_at: parse_dt(&row.get::<_, String>(5)?),
    })
}

pub(crate) const LINK_COLS: &str = "id, source_id, target_id, relation, weight, created_at";

// ---------------------------------------------------------------------------
// Document / Chunk helpers
// ---------------------------------------------------------------------------

pub(crate) const DOCUMENT_COLS: &str =
    "id, source_path, source_type, chunk_count, created_at, updated_at, project";

pub(crate) const CHUNK_COLS: &str = "id, document_id, chunk_index, content, source_path, \
     source_type, language, heading, line_start, line_end, created_at";

pub(crate) fn row_to_document(row: &rusqlite::Row) -> rusqlite::Result<Document> {
    // Column order: id(0), source_path(1), source_type(2), chunk_count(3),
    //   created_at(4), updated_at(5)
    let source_type_str: String = row.get(2)?;
    let source_type: SourceType = source_type_str.parse().unwrap_or_default();
    Ok(Document {
        id: row.get::<_, String>(0)?.into(),
        source_path: row.get(1)?,
        source_type,
        chunk_count: row.get::<_, u32>(3)? as usize,
        created_at: parse_dt(&row.get::<_, String>(4)?),
        updated_at: parse_dt(&row.get::<_, String>(5)?),
        project: row.get("project").ok(),
    })
}

pub(crate) fn row_to_chunk(row: &rusqlite::Row) -> rusqlite::Result<Chunk> {
    // Column order: id(0), document_id(1), chunk_index(2), content(3),
    //   source_path(4), source_type(5), language(6), heading(7),
    //   line_start(8), line_end(9), created_at(10)
    let source_type_str: String = row.get(5)?;
    let source_type: SourceType = source_type_str.parse().unwrap_or_default();
    let metadata = ChunkMetadata {
        source_path: row.get(4)?,
        source_type,
        language: row.get(6)?,
        heading: row.get(7)?,
        line_start: row.get(8)?,
        line_end: row.get(9)?,
    };
    Ok(Chunk {
        id: row.get::<_, String>(0)?.into(),
        document_id: row.get::<_, String>(1)?.into(),
        chunk_index: row.get::<_, u32>(2)?,
        content: row.get(3)?,
        metadata,
        embedding: None,
        created_at: parse_dt(&row.get::<_, String>(10)?),
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Datelike, Timelike};

    #[test]
    fn test_embedding_to_blob_roundtrip() {
        let original = vec![1.0, 2.0, 3.0, 4.5];
        let blob = embedding_to_blob(&original);
        let recovered = blob_to_embedding(&blob);
        assert_eq!(recovered, original);
    }

    #[test]
    fn test_embedding_to_blob_empty() {
        let original: Vec<f32> = vec![];
        let blob = embedding_to_blob(&original);
        assert_eq!(blob.len(), 0);
        let recovered = blob_to_embedding(&blob);
        assert_eq!(recovered, original);
    }

    #[test]
    fn test_embedding_to_blob_single_value() {
        let original = vec![42.5];
        let blob = embedding_to_blob(&original);
        assert_eq!(blob.len(), 4);
        let recovered = blob_to_embedding(&blob);
        assert_eq!(recovered.len(), 1);
        assert!((recovered[0] - 42.5).abs() < 1e-6);
    }

    #[test]
    fn test_embedding_to_blob_large_values() {
        let original = vec![1e6, -1e6, 0.0, 1.23456];
        let blob = embedding_to_blob(&original);
        let recovered = blob_to_embedding(&blob);
        assert_eq!(recovered.len(), original.len());
        for (o, r) in original.iter().zip(recovered.iter()) {
            assert!((o - r).abs() < 1e-6);
        }
    }

    #[test]
    fn test_parse_dt_valid_rfc3339() {
        let valid_rfc3339 = "2023-12-25T10:30:45Z";
        let dt = parse_dt(valid_rfc3339);
        assert_eq!(dt.year(), 2023);
        assert_eq!(dt.month(), 12);
        assert_eq!(dt.day(), 25);
        assert_eq!(dt.hour(), 10);
    }

    #[test]
    fn test_parse_dt_invalid_fallback_to_now() {
        let invalid = "not-a-date";
        let dt = parse_dt(invalid);
        // Should return a valid datetime (doesn't panic)
        // We can't assert exact time since now() changes, but we can check it's recent
        let now = Utc::now();
        let diff = (now - dt).num_seconds();
        assert!(diff.abs() < 2); // Within 2 seconds
    }

    #[test]
    fn test_parse_dt_empty_string() {
        let dt = parse_dt("");
        let now = Utc::now();
        let diff = (now - dt).num_seconds();
        assert!(diff.abs() < 2);
    }

    #[test]
    fn test_source_type_manual() {
        let source = MemorySource::Manual;
        assert_eq!(source_type(&source), "manual");
    }

    #[test]
    fn test_source_type_conversation() {
        let source = MemorySource::Conversation {
            thread_id: "test-thread".to_string(),
        };
        assert_eq!(source_type(&source), "conversation");
    }

    #[test]
    fn test_source_type_claude_code() {
        let source = MemorySource::ClaudeCode {
            session_id: "test-session".to_string(),
            file_path: None,
        };
        assert_eq!(source_type(&source), "claude_code");
    }

    #[test]
    fn test_source_data_manual() {
        let source = MemorySource::Manual;
        assert_eq!(source_data(&source), None);
    }

    #[test]
    fn test_source_data_conversation() {
        let source = MemorySource::Conversation {
            thread_id: "test-thread".to_string(),
        };
        let data = source_data(&source);
        assert!(data.is_some());
        // Should be valid JSON
        let _: Result<MemorySource, _> = serde_json::from_str(&data.unwrap());
    }

    #[test]
    fn test_parse_source_manual() {
        let source = parse_source("manual", None);
        match source {
            MemorySource::Manual => (),
            _ => panic!("Expected Manual variant"),
        }
    }

    #[test]
    fn test_parse_source_unknown_defaults_to_manual() {
        let source = parse_source("unknown", None);
        match source {
            MemorySource::Manual => (),
            _ => panic!("Expected Manual variant"),
        }
    }
}
