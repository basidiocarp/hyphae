use std::path::PathBuf;

pub(crate) fn resolve_db_path(cli_db: Option<PathBuf>, configured_db: Option<&str>) -> PathBuf {
    cli_db
        .or_else(|| configured_db.map(PathBuf::from))
        .unwrap_or_else(default_db_path)
}

pub(crate) fn default_db_path() -> PathBuf {
    spore::paths::data_dir("hyphae").join("hyphae.db")
}

pub(crate) fn default_config_path() -> Option<PathBuf> {
    Some(spore::paths::config_path_with_env(
        "hyphae",
        "HYPHAE_CONFIG",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_db_path_prefers_cli_argument() {
        let path = resolve_db_path(Some(PathBuf::from("/tmp/cli.db")), Some("/tmp/config.db"));
        assert_eq!(path, PathBuf::from("/tmp/cli.db"));
    }

    #[test]
    fn test_resolve_db_path_uses_config_when_cli_missing() {
        let path = resolve_db_path(None, Some("/tmp/config.db"));
        assert_eq!(path, PathBuf::from("/tmp/config.db"));
    }

    #[test]
    fn test_default_db_path_has_hyphae_db_name() {
        let path = default_db_path();
        assert_eq!(
            path.file_name().and_then(|value| value.to_str()),
            Some("hyphae.db")
        );
    }

    #[test]
    fn test_default_config_path_has_config_toml_name() {
        let path = default_config_path();
        assert_eq!(
            path.as_deref()
                .and_then(|value| value.file_name())
                .and_then(|value| value.to_str()),
            Some("config.toml")
        );
    }
}
