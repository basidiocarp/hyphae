//! Self-update command delegating to spore.

use anyhow::Result;

/// Check for updates and optionally download the latest Hyphae release from GitHub.
pub fn run(check_only: bool) -> Result<()> {
    spore::self_update::run(
        "hyphae",
        env!("CARGO_PKG_VERSION"),
        env!("CARGO_PKG_REPOSITORY"),
        check_only,
    )
}
