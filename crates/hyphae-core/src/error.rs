use thiserror::Error;

#[derive(Debug, Error)]
pub enum HyphaeError {
    #[error("memory not found: {0}")]
    NotFound(String),

    #[error("database error: {0}")]
    Database(String),

    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("config error: {0}")]
    Config(String),

    #[error("embedding error: {0}")]
    Embedding(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("validation error: {0}")]
    Validation(String),

    #[error("ingest error: {0}")]
    Ingest(String),

    #[error("lock was poisoned")]
    LockPoisoned,
}

pub type HyphaeResult<T> = Result<T, HyphaeError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hyphae_error_display() {
        assert_eq!(
            HyphaeError::NotFound("mem-1".into()).to_string(),
            "memory not found: mem-1"
        );
        assert_eq!(
            HyphaeError::Database("conn failed".into()).to_string(),
            "database error: conn failed"
        );
        assert_eq!(
            HyphaeError::Config("missing key".into()).to_string(),
            "config error: missing key"
        );
        assert_eq!(
            HyphaeError::Embedding("model error".into()).to_string(),
            "embedding error: model error"
        );
        assert_eq!(
            HyphaeError::Validation("bad input".into()).to_string(),
            "validation error: bad input"
        );
        assert_eq!(HyphaeError::LockPoisoned.to_string(), "lock was poisoned");
    }

    #[test]
    fn test_io_error_from() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let err: HyphaeError = io_err.into();
        assert!(err.to_string().contains("file missing"));
        assert!(matches!(err, HyphaeError::Io(_)));
    }
}
