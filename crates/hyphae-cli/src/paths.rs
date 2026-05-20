use std::path::PathBuf;

pub(crate) fn resolve_db_path(cli_db: Option<PathBuf>, configured_db: Option<&str>) -> PathBuf {
    cli_db
        .or_else(|| configured_db.map(PathBuf::from))
        .unwrap_or_else(default_db_path)
}

pub(crate) fn default_db_path() -> PathBuf {
    spore::paths::data_dir("basidiocarp")
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("hyphae/hyphae.db")
}

pub(crate) fn backup_dir() -> PathBuf {
    spore::paths::data_dir("basidiocarp")
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("hyphae/backups")
}

/// Move the legacy `~/.local/share/hyphae/` directory to the shared basidiocarp
/// root (`~/.local/share/basidiocarp/hyphae/`) on first startup after the path
/// change. No-op if the new location already exists or if the old location is
/// absent (fresh install). Falls back silently if the rename fails.
pub(crate) fn migrate_legacy_data_dir() {
    let new_base = spore::paths::data_dir("basidiocarp")
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("hyphae");
    let old_base = spore::paths::data_dir("hyphae").unwrap_or_else(|_| PathBuf::from("."));

    if new_base.exists() || !old_base.exists() {
        return;
    }

    if let Some(parent) = new_base.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            tracing::warn!("hyphae: could not create basidiocarp data dir: {e}");
            return;
        }
    }

    match std::fs::rename(&old_base, &new_base) {
        Ok(()) => tracing::info!(
            from = %old_base.display(),
            to = %new_base.display(),
            "hyphae: migrated data directory to shared basidiocarp root",
        ),
        Err(e) => tracing::warn!(
            from = %old_base.display(),
            to = %new_base.display(),
            "hyphae: migration rename failed ({e}); data remains at old location",
        ),
    }
}

pub(crate) fn default_config_path() -> Option<PathBuf> {
    spore::paths::config_path_with_env(
        "hyphae",
        "HYPHAE_CONFIG",
    ).ok()
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
    fn test_backup_dir_ends_with_backups() {
        let path = backup_dir();
        assert_eq!(
            path.file_name().and_then(|value| value.to_str()),
            Some("backups")
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
