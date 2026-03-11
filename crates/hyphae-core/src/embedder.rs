use crate::error::HyphaeResult;

pub trait Embedder: Send + Sync {
    fn embed(&self, text: &str) -> HyphaeResult<Vec<f32>>;
    fn embed_batch(&self, texts: &[&str]) -> HyphaeResult<Vec<Vec<f32>>>;
    fn dimensions(&self) -> usize;
}
