pub mod artifact;
pub mod chunk;
pub mod chunk_store;
pub mod embedder;
pub mod error;
pub mod eviction;
#[cfg(feature = "embeddings")]
pub mod fastembed_embedder;
pub mod git_context;
pub mod http_embedder;
pub mod identity;
pub mod ids;
pub mod llm_client;
pub mod memoir;
pub mod memoir_store;
pub mod memory;
pub mod sanitize;
pub mod secrets;
pub mod store;
pub mod tier;

pub use artifact::{Artifact, ArtifactType, UnknownArtifactType};
pub use chunk::{Chunk, ChunkMetadata, ChunkSearchResult, Document, SourceType};
pub use chunk_store::ChunkStore;
pub use embedder::Embedder;
pub use error::{HyphaeError, HyphaeResult};
pub use eviction::{DefaultEvictionPolicy, EvictionPolicy};
#[cfg(feature = "embeddings")]
pub use fastembed_embedder::FastEmbedder;
pub use git_context::{GitContext, current_git_hash, detect_git_context_from};
pub use http_embedder::HttpEmbedder;
pub use identity::{
    BACKUP_EXPORT_SCHEMA_VERSION, BackupExportManifest, SCOPED_IDENTITY_SCHEMA_VERSION,
    ScopedIdentity,
};
pub use ids::*;
pub use llm_client::consolidate_via_llm;
pub use memoir::{
    ApplicabilityRule, Authority, Concept, ConceptLink, Confidence, Decay, InputSpec,
    KnowledgeDomain, Label, Memoir, MemoirMeta, MemoirSource, MemoirStats, MemoirVersion,
    QueryContext, RecallResult, Relation, RuleOp,
};
pub use memoir_store::{ConceptInput, LinkInput, MemoirStore, UpsertReport};
pub use memory::{
    ConsolidationConfig, ConsolidationTopicRule, DEFAULT_CONSOLIDATION_THRESHOLD, Importance,
    Memory, MemoryBuilder, MemorySource, SearchQuery, SearchType, SessionHost, StoreStats,
    TopicHealth, Weight,
};
pub use sanitize::{SanitizedQuery, sanitize_query};
pub use secrets::detect_secrets;
pub use store::{MemoryStore, SearchOrder, TopicMemoryOrder};
pub use tier::MemoryTier;
