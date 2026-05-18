use crate::chunk::{Chunk, ChunkSearchResult, Document};
use crate::error::HyphaeResult;
use crate::ids::DocumentId;

pub trait ChunkStore {
    /// Store a document record and return its ID.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database write fails.
    fn store_document(&self, doc: Document) -> HyphaeResult<DocumentId>;

    /// Store chunks in bulk. Returns the number stored.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database write fails.
    fn store_chunks(&self, chunks: Vec<Chunk>) -> HyphaeResult<usize>;

    /// Fetch a document by ID.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database query fails.
    fn get_document(&self, id: &DocumentId) -> HyphaeResult<Option<Document>>;

    /// Fetch a document by its source path and optional project filter.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database query fails.
    fn get_document_by_path(
        &self,
        path: &str,
        project: Option<&str>,
    ) -> HyphaeResult<Option<Document>>;

    /// Fetch all chunks belonging to a document.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database query fails.
    fn get_chunks(&self, document_id: &DocumentId) -> HyphaeResult<Vec<Chunk>>;

    /// Delete a document and cascade to its chunks.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database write fails.
    fn delete_document(&self, id: &DocumentId) -> HyphaeResult<()>;

    /// List all documents, optionally filtered by project.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database query fails.
    fn list_documents(&self, project: Option<&str>) -> HyphaeResult<Vec<Document>>;

    /// Full-text search over chunk content.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database query fails.
    fn search_chunks_fts(
        &self,
        query: &str,
        limit: usize,
        offset: usize,
        project: Option<&str>,
    ) -> HyphaeResult<Vec<ChunkSearchResult>>;

    /// Vector similarity search over chunk embeddings.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database query fails.
    fn search_chunks_by_embedding(
        &self,
        embedding: &[f32],
        limit: usize,
        offset: usize,
        project: Option<&str>,
    ) -> HyphaeResult<Vec<ChunkSearchResult>>;

    /// Hybrid (FTS + vector) search over chunks.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database query fails.
    fn search_chunks_hybrid(
        &self,
        query: &str,
        embedding: &[f32],
        limit: usize,
        offset: usize,
        project: Option<&str>,
    ) -> HyphaeResult<Vec<ChunkSearchResult>>;

    /// Count indexed documents, optionally filtered by project.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database query fails.
    fn count_documents(&self, project: Option<&str>) -> HyphaeResult<usize>;

    /// Count indexed chunks, optionally filtered by project.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the database query fails.
    fn count_chunks(&self, project: Option<&str>) -> HyphaeResult<usize>;
}
