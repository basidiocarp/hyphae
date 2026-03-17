//! Self-update command that checks GitHub releases and downloads the latest binary.
use anyhow::{Context, Result};
use std::io::{Read, Write};

const REPO: &str = env!("CARGO_PKG_REPOSITORY"); // "https://github.com/basidiocarp/hyphae"

/// Check for updates and optionally download the latest Hyphae release from GitHub.
pub fn run(check_only: bool) -> Result<()> {
    let current_version = env!("CARGO_PKG_VERSION");
    println!("Current version: v{current_version}");
    print!("Checking for updates... ");
    std::io::stdout().flush().ok();

    let latest = fetch_latest_release().context("Failed to check for updates")?;
    let latest_tag = latest["tag_name"]
        .as_str()
        .context("Missing tag_name in GitHub API response")?;

    // Strip leading 'v' for comparison
    let latest_version = latest_tag.trim_start_matches('v');
    println!("Latest version: {latest_tag}");

    if latest_version == current_version {
        println!("Already up to date.");
        return Ok(());
    }

    println!("Update available: v{current_version} -> {latest_tag}");

    if check_only {
        println!("Run `hyphae self-update` to install.");
        return Ok(());
    }

    let asset_name = target_asset_name().context("Unsupported platform for self-update")?;
    let download_url = find_asset_url(&latest["assets"], &asset_name)
        .with_context(|| format!("No release asset found for '{asset_name}'"))?;

    let current_exe = std::env::current_exe().context("Failed to locate current executable")?;

    println!("Downloading {asset_name}...");
    let archive_bytes =
        download_binary(&download_url).context("Failed to download update archive")?;

    println!("Extracting...");
    let binary_bytes = extract_binary(&archive_bytes, &asset_name)
        .context("Failed to extract binary from archive")?;

    replace_binary(&current_exe, &binary_bytes).context("Failed to replace binary")?;

    println!("Updated to {latest_tag}. Run `hyphae --version` to confirm.");
    Ok(())
}

fn fetch_latest_release() -> Result<serde_json::Value> {
    // Convert "https://github.com/owner/repo" -> "https://api.github.com/repos/owner/repo/releases/latest"
    let repo_path = REPO
        .trim_end_matches('/')
        .strip_prefix("https://github.com/")
        .context("CARGO_PKG_REPOSITORY is not a github.com URL")?;
    let api_url = format!("https://api.github.com/repos/{repo_path}/releases/latest");

    let agent = ureq::Agent::new_with_defaults();
    let response = agent
        .get(&api_url)
        .header(
            "User-Agent",
            &format!("hyphae/{}", env!("CARGO_PKG_VERSION")),
        )
        .header("Accept", "application/vnd.github+json")
        .call()
        .context("Failed to fetch latest release (check your internet connection)")?;

    let json: serde_json::Value = serde_json::from_reader(response.into_body().as_reader())
        .context("Invalid JSON from GitHub API")?;
    Ok(json)
}

fn target_asset_name() -> Option<String> {
    let (os_suffix, ext) = match std::env::consts::OS {
        "macos" => ("apple-darwin", ".tar.gz"),
        "linux" => ("unknown-linux-musl", ".tar.gz"),
        "windows" => ("pc-windows-msvc", ".zip"),
        _ => return None,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        _ => return None,
    };
    Some(format!("hyphae-{arch}-{os_suffix}{ext}"))
}

fn find_asset_url(assets: &serde_json::Value, name: &str) -> Option<String> {
    assets.as_array()?.iter().find_map(|asset| {
        let asset_name = asset["name"].as_str()?;
        if asset_name == name {
            asset["browser_download_url"].as_str().map(String::from)
        } else {
            None
        }
    })
}

fn download_binary(url: &str) -> Result<Vec<u8>> {
    let agent = ureq::Agent::new_with_defaults();
    let response = agent
        .get(url)
        .header(
            "User-Agent",
            &format!("hyphae/{}", env!("CARGO_PKG_VERSION")),
        )
        .call()
        .context("Download failed")?;

    let mut bytes = Vec::new();
    response
        .into_body()
        .as_reader()
        .read_to_end(&mut bytes)
        .context("Failed to read download response")?;

    if bytes.is_empty() {
        anyhow::bail!("Downloaded binary is empty");
    }
    Ok(bytes)
}

/// Extract the `hyphae` binary from a downloaded archive using system tools.
fn extract_binary(archive_bytes: &[u8], asset_name: &str) -> Result<Vec<u8>> {
    use std::process::Command;

    let tmp_dir = tempfile::tempdir().context("Failed to create temp directory")?;
    let archive_path = tmp_dir.path().join(asset_name);

    std::fs::write(&archive_path, archive_bytes).context("Failed to write archive to temp file")?;

    let binary_name = if cfg!(windows) {
        "hyphae.exe"
    } else {
        "hyphae"
    };

    if asset_name.ends_with(".tar.gz") {
        let status = Command::new("tar")
            .args(["xzf", &archive_path.to_string_lossy(), "-C"])
            .arg(tmp_dir.path())
            .status()
            .context("Failed to run tar (is it installed?)")?;
        if !status.success() {
            anyhow::bail!("tar extraction failed with exit code {}", status);
        }
    } else if asset_name.ends_with(".zip") {
        let status = Command::new("unzip")
            .args(["-o", &*archive_path.to_string_lossy(), "-d"])
            .arg(tmp_dir.path())
            .status()
            .context("Failed to run unzip (is it installed?)")?;
        if !status.success() {
            anyhow::bail!("unzip extraction failed with exit code {}", status);
        }
    } else {
        // Not an archive -- treat as raw binary
        return Ok(archive_bytes.to_vec());
    }

    let extracted = tmp_dir.path().join(binary_name);
    std::fs::read(&extracted).with_context(|| {
        format!(
            "Binary '{}' not found in archive. Contents: {:?}",
            binary_name,
            std::fs::read_dir(tmp_dir.path())
                .map(|entries| entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .collect::<Vec<_>>())
                .unwrap_or_default()
        )
    })
}

fn replace_binary(current_exe: &std::path::Path, binary_bytes: &[u8]) -> Result<()> {
    // Write to a temp file next to the current exe so rename is atomic (same filesystem)
    let parent = current_exe
        .parent()
        .context("Executable has no parent directory")?;
    let tmp_path = parent.join(".hyphae-update.tmp");

    let write_result = (|| -> Result<()> {
        let mut tmp = std::fs::File::create(&tmp_path).context("Failed to create temp file")?;
        tmp.write_all(binary_bytes)
            .context("Failed to write update to temp file")?;
        tmp.flush().context("Failed to flush temp file")?;
        Ok(())
    })();

    if let Err(e) = write_result {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e);
    }

    // Set executable permissions on Unix
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o755))
            .context("Failed to set executable permissions")?;
    }

    // Atomic rename
    std::fs::rename(&tmp_path, current_exe).map_err(|e| {
        let _ = std::fs::remove_file(&tmp_path);
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            anyhow::anyhow!(
                "Permission denied replacing binary at {}. Try: sudo hyphae self-update",
                current_exe.display()
            )
        } else {
            anyhow::anyhow!("Failed to replace binary: {e}")
        }
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_target_asset_name_known_platform() {
        let name = target_asset_name();
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("macos" | "linux" | "windows", "x86_64" | "aarch64") => {
                assert!(name.is_some());
                let n = name.unwrap();
                assert!(n.starts_with("hyphae-"));
                assert!(n.contains(std::env::consts::ARCH));
            }
            _ => {
                assert!(name.is_none());
            }
        }
    }

    #[test]
    fn test_target_asset_name_macos_aarch64() {
        let asset = "hyphae-aarch64-apple-darwin.tar.gz".to_string();
        assert!(asset.starts_with("hyphae-"));
        assert!(asset.contains("aarch64"));
        assert!(asset.contains("apple-darwin"));
        assert!(asset.ends_with(".tar.gz"));
    }

    #[test]
    fn test_find_asset_url_present() {
        let assets = serde_json::json!([
            {"name": "hyphae-x86_64-apple-darwin.tar.gz", "browser_download_url": "https://example.com/hyphae.tar.gz"},
            {"name": "hyphae-x86_64-unknown-linux-musl.tar.gz", "browser_download_url": "https://example.com/hyphae-linux.tar.gz"},
        ]);
        let url = find_asset_url(&assets, "hyphae-x86_64-apple-darwin.tar.gz");
        assert_eq!(url, Some("https://example.com/hyphae.tar.gz".to_string()));
    }

    #[test]
    fn test_find_asset_url_missing() {
        let assets = serde_json::json!([
            {"name": "hyphae-x86_64-unknown-linux-musl.tar.gz", "browser_download_url": "https://example.com/hyphae-linux.tar.gz"},
        ]);
        let url = find_asset_url(&assets, "hyphae-x86_64-apple-darwin.tar.gz");
        assert!(url.is_none());
    }

    #[test]
    fn test_find_asset_url_empty() {
        let assets = serde_json::json!([]);
        let url = find_asset_url(&assets, "hyphae-x86_64-apple-darwin");
        assert!(url.is_none());
    }
}
