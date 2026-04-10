use chrono::{TimeZone, Utc};
use criterion::{Criterion, black_box, criterion_group, criterion_main};
use hyphae_core::MemoryStore;
use hyphae_core::{
    Chunk, ChunkMetadata, ChunkStore, Document, Importance, Memory, MemorySource, SourceType,
    Weight,
};
use hyphae_store::SqliteStore;

const PROJECT: &str = "bench-project";
const WORKTREE: &str = "bench-worktree";
const MEMORIES: usize = 96;
const CHUNKS: usize = 48;
const DIMENSIONS: usize = 384;

fn fixed_ts(offset_minutes: i64) -> chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2025, 1, 1, 12, 0, 0)
        .single()
        .expect("valid timestamp")
        + chrono::Duration::minutes(offset_minutes)
}

fn query_embedding() -> Vec<f32> {
    vec![0.12; DIMENSIONS]
}

fn memory_embedding(index: usize) -> Vec<f32> {
    let value = if index % 4 == 0 {
        0.12
    } else if index % 4 == 1 {
        0.08
    } else if index % 4 == 2 {
        0.04
    } else {
        0.16
    };
    vec![value; DIMENSIONS]
}

fn chunk_embedding(index: usize) -> Vec<f32> {
    let value = if index % 3 == 0 {
        0.12
    } else if index % 3 == 1 {
        0.09
    } else {
        0.15
    };
    vec![value; DIMENSIONS]
}

fn make_memory(index: usize) -> Memory {
    let mut memory = Memory::new(
        "retrieval".to_string(),
        format!("bench retrieval memory {index} with ranking and vector signals"),
        Importance::Medium,
    );
    memory.created_at = fixed_ts(index as i64);
    memory.updated_at = memory.created_at;
    memory.last_accessed = memory.created_at;
    memory.access_count = (index % 7) as u32;
    memory.weight = Weight::new_clamped(0.35 + (index as f32 % 5.0) * 0.1);
    memory.keywords = vec!["bench".into(), "retrieval".into(), "ranking".into()];
    memory.source = MemorySource::Manual;
    memory.project = Some(PROJECT.to_string());
    memory.worktree = Some(WORKTREE.to_string());
    memory.embedding = Some(memory_embedding(index));
    memory
}

fn make_document(index: usize) -> Document {
    let created_at = fixed_ts(index as i64);
    Document {
        id: hyphae_core::DocumentId::new(),
        source_path: format!("docs/bench-{index}.md"),
        source_type: SourceType::Markdown,
        chunk_count: 1,
        created_at,
        updated_at: created_at,
        project: Some(PROJECT.to_string()),
        runtime_session_id: None,
    }
}

fn make_chunk(document_id: &hyphae_core::DocumentId, index: usize) -> Chunk {
    let created_at = fixed_ts(index as i64);
    Chunk {
        id: hyphae_core::ChunkId::new(),
        document_id: document_id.clone(),
        chunk_index: 0,
        content: format!("bench retrieval chunk {index} with ranking and vector signals"),
        metadata: ChunkMetadata {
            source_path: format!("docs/bench-{index}.md"),
            source_type: SourceType::Markdown,
            language: Some("markdown".into()),
            heading: Some(format!("Bench Section {index}")),
            line_start: Some(1),
            line_end: Some(8),
        },
        embedding: Some(chunk_embedding(index)),
        created_at,
    }
}

fn build_fixture() -> SqliteStore {
    let store = SqliteStore::in_memory().expect("in-memory store");

    for index in 0..MEMORIES {
        store.store(make_memory(index)).expect("seed memory");
    }

    for index in 0..CHUNKS {
        let document = make_document(index);
        let document_id = document.id.clone();
        let chunk = make_chunk(&document_id, index);
        store.store_document(document).expect("seed document");
        store.store_chunks(vec![chunk]).expect("seed chunks");
    }

    store
}

fn bench_search_hybrid_scoped(c: &mut Criterion) {
    let store = build_fixture();
    let embedding = query_embedding();

    c.bench_function("hyphae_store/search_hybrid_scoped", |b| {
        b.iter(|| {
            let results = store
                .search_hybrid_scoped(
                    black_box("bench retrieval ranking"),
                    black_box(embedding.as_slice()),
                    black_box(12),
                    black_box(0),
                    Some(PROJECT),
                    Some(WORKTREE),
                )
                .expect("hybrid search");
            black_box(results)
        })
    });
}

fn bench_search_all(c: &mut Criterion) {
    let store = build_fixture();
    let embedding = query_embedding();

    c.bench_function("hyphae_store/search_all", |b| {
        b.iter(|| {
            let results = store
                .search_all(
                    black_box("bench retrieval ranking"),
                    Some(black_box(embedding.as_slice())),
                    black_box(12),
                    black_box(0),
                    black_box(true),
                    Some(PROJECT),
                    None,
                )
                .expect("unified search");
            black_box(results)
        })
    });
}

criterion_group!(
    retrieval_hot_paths,
    bench_search_hybrid_scoped,
    bench_search_all
);
criterion_main!(retrieval_hot_paths);
