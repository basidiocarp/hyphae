use crate::error::HyphaeResult;

pub trait Embedder: Send + Sync {
    /// Embed a single text string into a vector.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the embedding model is unavailable or the
    /// request fails.
    fn embed(&self, text: &str) -> HyphaeResult<Vec<f32>>;

    /// Embed multiple texts in a single batch call.
    ///
    /// # Errors
    /// Returns `HyphaeError` if the embedding model is unavailable or the
    /// request fails.
    fn embed_batch(&self, texts: &[&str]) -> HyphaeResult<Vec<Vec<f32>>>;

    /// Return the output dimensionality of this embedder.
    fn dimensions(&self) -> usize;
}
