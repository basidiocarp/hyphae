use crate::chunk::{Chunk, ChunkSearchResult, Document};
use crate::error::HyphaeResult;
use crate::ids::DocumentId;

pub trait ChunkStore {
    fn store_document(&self, doc: Document) -> HyphaeResult<DocumentId>;
    fn store_chunks(&self, chunks: Vec<Chunk>) -> HyphaeResult<usize>;
    fn get_document(&self, id: &DocumentId) -> HyphaeResult<Option<Document>>;
    fn get_document_by_path(&self, path: &str) -> HyphaeResult<Option<Document>>;
    fn get_chunks(&self, document_id: &DocumentId) -> HyphaeResult<Vec<Chunk>>;
    fn delete_document(&self, id: &DocumentId) -> HyphaeResult<()>;
    fn list_documents(&self) -> HyphaeResult<Vec<Document>>;
    fn search_chunks_fts(&self, query: &str, limit: usize) -> HyphaeResult<Vec<ChunkSearchResult>>;
    fn search_chunks_by_embedding(
        &self,
        embedding: &[f32],
        limit: usize,
    ) -> HyphaeResult<Vec<ChunkSearchResult>>;
    fn search_chunks_hybrid(
        &self,
        query: &str,
        embedding: &[f32],
        limit: usize,
    ) -> HyphaeResult<Vec<ChunkSearchResult>>;
    fn count_documents(&self) -> HyphaeResult<usize>;
    fn count_chunks(&self) -> HyphaeResult<usize>;
}
