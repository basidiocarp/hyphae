use std::sync::{Mutex, OnceLock};

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

use crate::embedder::Embedder;
use crate::error::{HyphaeError, HyphaeResult};

pub struct FastEmbedder {
    model: OnceLock<TextEmbedding>,
    init_lock: Mutex<()>,
    model_name: String,
    dims: usize,
}

/// Default model: BGE-small-en-v1.5 (384d, English, fast)
const DEFAULT_MODEL: &str = "BAAI/bge-small-en-v1.5";

/// Resolve a model string to (EmbeddingModel, dimensions).
fn resolve_model(name: &str) -> HyphaeResult<(EmbeddingModel, usize)> {
    let model: EmbeddingModel = name
        .parse()
        .map_err(|e: String| HyphaeError::Embedding(e))?;
    let dims = model_dimensions(&model);
    Ok((model, dims))
}

/// Known dimensions for fastembed models.
fn model_dimensions(model: &EmbeddingModel) -> usize {
    match model {
        EmbeddingModel::AllMiniLML6V2
        | EmbeddingModel::AllMiniLML6V2Q
        | EmbeddingModel::AllMiniLML12V2
        | EmbeddingModel::AllMiniLML12V2Q
        | EmbeddingModel::BGESmallENV15
        | EmbeddingModel::BGESmallENV15Q
        | EmbeddingModel::ParaphraseMLMiniLML12V2
        | EmbeddingModel::ParaphraseMLMiniLML12V2Q => 384,

        EmbeddingModel::BGEBaseENV15
        | EmbeddingModel::BGEBaseENV15Q
        | EmbeddingModel::ParaphraseMLMpnetBaseV2
        | EmbeddingModel::GTEBaseENV15
        | EmbeddingModel::GTEBaseENV15Q
        | EmbeddingModel::JinaEmbeddingsV2BaseCode => 768,

        EmbeddingModel::BGELargeENV15
        | EmbeddingModel::BGELargeENV15Q
        | EmbeddingModel::MxbaiEmbedLargeV1
        | EmbeddingModel::MxbaiEmbedLargeV1Q
        | EmbeddingModel::GTELargeENV15
        | EmbeddingModel::GTELargeENV15Q
        | EmbeddingModel::ModernBertEmbedLarge => 1024,

        EmbeddingModel::NomicEmbedTextV1
        | EmbeddingModel::NomicEmbedTextV15
        | EmbeddingModel::NomicEmbedTextV15Q => 768,

        EmbeddingModel::ClipVitB32 => 512,

        // Unsupported models (multilingual, Chinese) — fall back to 384
        _ => 384,
    }
}

/// Resolve the cache directory for embedding models.
///
/// Uses `~/.cache/hyphae/models/` so models are shared across databases
/// and not re-downloaded per project.
fn cache_directory() -> Option<std::path::PathBuf> {
    std::env::var("HOME")
        .ok()
        .map(|h| std::path::PathBuf::from(h).join(".cache/hyphae/models"))
}

impl FastEmbedder {
    /// Create with default model (bge-small-en-v1.5).
    pub fn new() -> HyphaeResult<Self> {
        Self::with_model(DEFAULT_MODEL)
    }

    /// Create with a specific model by name (e.g. "BAAI/bge-small-en-v1.5").
    pub fn with_model(model_name: &str) -> HyphaeResult<Self> {
        let (_, dims) = resolve_model(model_name)?;
        Ok(Self {
            model: OnceLock::new(),
            init_lock: Mutex::new(()),
            model_name: model_name.to_string(),
            dims,
        })
    }

    fn get_model(&self) -> HyphaeResult<&TextEmbedding> {
        if let Some(m) = self.model.get() {
            return Ok(m);
        }
        let _guard = self
            .init_lock
            .lock()
            .map_err(|_| HyphaeError::LockPoisoned)?;
        if let Some(m) = self.model.get() {
            return Ok(m);
        }
        eprintln!("Downloading embedding model ({})...", self.model_name);
        let (emb_model, _) = resolve_model(&self.model_name)?;
        let cache_dir = cache_directory();
        let mut opts = InitOptions::new(emb_model).with_show_download_progress(true);
        if let Some(dir) = &cache_dir {
            let _ = std::fs::create_dir_all(dir);
            opts = opts.with_cache_dir(dir.clone());
        }
        let model = TextEmbedding::try_new(opts)
            .map_err(|e| HyphaeError::Embedding(format!("failed to init model: {e}")))?;
        let _ = self.model.set(model);
        self.model
            .get()
            .ok_or_else(|| HyphaeError::Embedding("model not initialized".into()))
    }
}

impl Embedder for FastEmbedder {
    fn embed(&self, text: &str) -> HyphaeResult<Vec<f32>> {
        let model = self.get_model()?;
        let results = model
            .embed(vec![text], None)
            .map_err(|e| HyphaeError::Embedding(e.to_string()))?;
        results
            .into_iter()
            .next()
            .ok_or_else(|| HyphaeError::Embedding("empty embedding result".into()))
    }

    fn embed_batch(&self, texts: &[&str]) -> HyphaeResult<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let model = self.get_model()?;
        model
            .embed(texts.to_vec(), None)
            .map_err(|e| HyphaeError::Embedding(e.to_string()))
    }

    fn dimensions(&self) -> usize {
        self.dims
    }
}
