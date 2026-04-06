pub mod chunk;
pub mod chunk_store;
pub mod embedder;
pub mod error;
#[cfg(feature = "embeddings")]
pub mod fastembed_embedder;
pub mod git_context;
pub mod http_embedder;
pub mod ids;
pub mod memoir;
pub mod memoir_store;
pub mod memory;
pub mod secrets;
pub mod store;

pub use chunk::{Chunk, ChunkMetadata, ChunkSearchResult, Document, SourceType};
pub use chunk_store::ChunkStore;
pub use embedder::Embedder;
pub use error::{HyphaeError, HyphaeResult};
#[cfg(feature = "embeddings")]
pub use fastembed_embedder::FastEmbedder;
pub use git_context::{GitContext, detect_git_context_from};
pub use http_embedder::HttpEmbedder;
pub use ids::*;
pub use memoir::{Concept, ConceptLink, Confidence, Label, Memoir, MemoirStats, Relation};
pub use memoir_store::{ConceptInput, LinkInput, MemoirStore, UpsertReport};
pub use memory::{
    ConsolidationConfig, ConsolidationTopicRule, DEFAULT_CONSOLIDATION_THRESHOLD, Importance,
    Memory, MemoryBuilder, MemorySource, SessionHost, StoreStats, TopicHealth, Weight,
};
pub use secrets::detect_secrets;
pub use store::MemoryStore;
