use std::path::PathBuf;

pub(crate) fn default_db_path() -> PathBuf {
    directories::ProjectDirs::from("", "", "hyphae")
        .map(|d| d.data_dir().join("hyphae.db"))
        .unwrap_or_else(|| {
            directories::BaseDirs::new()
                .map(|d| d.data_local_dir().join("hyphae").join("hyphae.db"))
                .unwrap_or_else(|| PathBuf::from(".local/share/hyphae/hyphae.db"))
        })
}

pub(crate) fn default_config_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("HYPHAE_CONFIG") {
        return Some(PathBuf::from(path));
    }

    directories::ProjectDirs::from("", "", "hyphae")
        .map(|d| d.config_dir().join("config.toml"))
        .or_else(|| {
            directories::BaseDirs::new().map(|d| {
                d.home_dir()
                    .join(".config")
                    .join("hyphae")
                    .join("config.toml")
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

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
