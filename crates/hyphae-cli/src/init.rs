use anyhow::{Context, Result};
use clap::ValueEnum;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, ValueEnum)]
pub enum Editor {
    ClaudeCode,
    Cursor,
    VsCode,
    Zed,
    Windsurf,
    Amp,
    ClaudeDesktop,
    CodexCli,
}

impl std::fmt::Display for Editor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Editor::ClaudeCode => write!(f, "Claude Code"),
            Editor::Cursor => write!(f, "Cursor"),
            Editor::VsCode => write!(f, "VS Code"),
            Editor::Zed => write!(f, "Zed"),
            Editor::Windsurf => write!(f, "Windsurf"),
            Editor::Amp => write!(f, "Amp"),
            Editor::ClaudeDesktop => write!(f, "Claude Desktop"),
            Editor::CodexCli => write!(f, "Codex CLI"),
        }
    }
}

fn home_dir() -> Option<PathBuf> {
    std::env::var("HOME").ok().map(PathBuf::from)
}

pub fn detect_editors() -> Vec<Editor> {
    let mut editors = Vec::new();
    let Some(home) = home_dir() else {
        return editors;
    };

    if home.join(".claude.json").exists() {
        editors.push(Editor::ClaudeCode);
    }

    if home.join(".cursor").is_dir() {
        editors.push(Editor::Cursor);
    }

    #[cfg(target_os = "macos")]
    let vscode_path = home.join("Library/Application Support/Code");
    #[cfg(not(target_os = "macos"))]
    let vscode_path = home.join(".config/Code");
    if vscode_path.exists() {
        editors.push(Editor::VsCode);
    }

    if home.join(".zed").is_dir() {
        editors.push(Editor::Zed);
    }

    if home.join(".codeium/windsurf").is_dir() {
        editors.push(Editor::Windsurf);
    }

    if home.join(".config/amp").is_dir() {
        editors.push(Editor::Amp);
    }

    #[cfg(target_os = "macos")]
    let claude_desktop_path = home.join("Library/Application Support/Claude");
    #[cfg(not(target_os = "macos"))]
    let claude_desktop_path = home.join(".config/Claude");
    if claude_desktop_path.exists() {
        editors.push(Editor::ClaudeDesktop);
    }

    if home.join(".codex").is_dir() {
        editors.push(Editor::CodexCli);
    }

    editors
}

pub fn resolve_hyphae_binary() -> Result<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Ok(canonical) = exe.canonicalize() {
            return Ok(canonical);
        }
    }
    which_hyphae().context("could not locate hyphae binary in PATH")
}

fn which_hyphae() -> Option<PathBuf> {
    let path_var = std::env::var("PATH").ok()?;
    for dir in path_var.split(':') {
        let candidate = PathBuf::from(dir).join("hyphae");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

pub fn config_path_for(editor: &Editor) -> PathBuf {
    let home = home_dir().unwrap_or_else(|| PathBuf::from("/"));
    match editor {
        Editor::ClaudeCode => home.join(".claude.json"),
        Editor::Cursor => home.join(".cursor/mcp.json"),
        Editor::VsCode => {
            #[cfg(target_os = "macos")]
            {
                home.join("Library/Application Support/Code/User/settings.json")
            }
            #[cfg(not(target_os = "macos"))]
            {
                home.join(".config/Code/User/settings.json")
            }
        }
        Editor::Zed => home.join(".zed/settings.json"),
        Editor::Windsurf => home.join(".codeium/windsurf/mcp_config.json"),
        Editor::Amp => home.join(".config/amp/settings.json"),
        Editor::ClaudeDesktop => {
            #[cfg(target_os = "macos")]
            {
                home.join("Library/Application Support/Claude/claude_desktop_config.json")
            }
            #[cfg(not(target_os = "macos"))]
            {
                home.join(".config/Claude/claude_desktop_config.json")
            }
        }
        Editor::CodexCli => home.join(".codex/config.toml"),
    }
}

fn backup_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.bak", path.display()))
}

pub fn write_config(editor: &Editor, binary_path: &Path) -> Result<()> {
    let config_path = config_path_for(editor);

    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating directory {}", parent.display()))?;
    }

    match editor {
        Editor::CodexCli => write_toml_config(&config_path, binary_path),
        _ => write_json_config(editor, &config_path, binary_path),
    }
}

fn write_json_config(editor: &Editor, config_path: &Path, binary_path: &Path) -> Result<()> {
    let existing: serde_json::Value = if config_path.exists() {
        let content = std::fs::read_to_string(config_path)
            .with_context(|| format!("reading {}", config_path.display()))?;
        serde_json::from_str(&content)
            .with_context(|| format!("parsing JSON from {}", config_path.display()))?
    } else {
        serde_json::Value::Object(serde_json::Map::new())
    };

    if config_path.exists() {
        let bak = backup_path(config_path);
        std::fs::copy(config_path, &bak)
            .with_context(|| format!("creating backup {}", bak.display()))?;
    }

    let mut root = match existing {
        serde_json::Value::Object(map) => map,
        _ => serde_json::Map::new(),
    };

    let binary_str = binary_path.to_string_lossy().to_string();
    let hyphae_entry = match editor {
        Editor::VsCode => serde_json::json!({
            "command": binary_str,
            "args": ["serve"],
            "type": "stdio"
        }),
        Editor::Zed => serde_json::json!({
            "command": {
                "path": binary_str,
                "args": ["serve"]
            }
        }),
        _ => serde_json::json!({
            "command": binary_str,
            "args": ["serve"]
        }),
    };

    let top_key = match editor {
        Editor::VsCode => "servers",
        Editor::Zed => "context_servers",
        _ => "mcpServers",
    };

    let servers = root
        .entry(top_key)
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    if let serde_json::Value::Object(map) = servers {
        map.insert("hyphae".to_string(), hyphae_entry);
    }

    let json_str = serde_json::to_string_pretty(&serde_json::Value::Object(root))
        .context("serializing JSON config")?;
    std::fs::write(config_path, json_str)
        .with_context(|| format!("writing {}", config_path.display()))?;

    Ok(())
}

fn write_toml_config(config_path: &Path, binary_path: &Path) -> Result<()> {
    let existing: toml::Value = if config_path.exists() {
        let content = std::fs::read_to_string(config_path)
            .with_context(|| format!("reading {}", config_path.display()))?;
        toml::from_str(&content)
            .with_context(|| format!("parsing TOML from {}", config_path.display()))?
    } else {
        toml::Value::Table(toml::map::Map::new())
    };

    if config_path.exists() {
        let bak = backup_path(config_path);
        std::fs::copy(config_path, &bak)
            .with_context(|| format!("creating backup {}", bak.display()))?;
    }

    let mut root = match existing {
        toml::Value::Table(map) => map,
        _ => toml::map::Map::new(),
    };

    let binary_str = binary_path.to_string_lossy().to_string();

    let mut hyphae_table = toml::map::Map::new();
    hyphae_table.insert("command".to_string(), toml::Value::String(binary_str));
    hyphae_table.insert(
        "args".to_string(),
        toml::Value::Array(vec![toml::Value::String("serve".to_string())]),
    );

    let mcp_servers = root
        .entry("mcp_servers")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    if let toml::Value::Table(map) = mcp_servers {
        map.insert("hyphae".to_string(), toml::Value::Table(hyphae_table));
    }

    let toml_str =
        toml::to_string_pretty(&toml::Value::Table(root)).context("serializing TOML config")?;
    std::fs::write(config_path, toml_str)
        .with_context(|| format!("writing {}", config_path.display()))?;

    Ok(())
}

pub fn run_init(editor: Option<Editor>) -> Result<()> {
    let binary_path = resolve_hyphae_binary()?;

    if let Some(ed) = editor {
        write_config(&ed, &binary_path)?;
        println!("Configured hyphae for {ed}");
    } else {
        let detected = detect_editors();
        if detected.is_empty() {
            println!("No supported editors detected. Supported editors:");
            println!("  --editor claude-code");
            println!("  --editor cursor");
            println!("  --editor vs-code");
            println!("  --editor zed");
            println!("  --editor windsurf");
            println!("  --editor amp");
            println!("  --editor claude-desktop");
            println!("  --editor codex-cli");
            return Err(anyhow::anyhow!("no editors detected"));
        }
        for ed in detected {
            write_config(&ed, &binary_path)?;
            println!("Configured hyphae for {ed}");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn fake_binary() -> PathBuf {
        PathBuf::from("/usr/local/bin/hyphae")
    }

    #[test]
    fn test_merge_preserves_existing_mcp_servers() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("claude.json");

        let existing = serde_json::json!({
            "mcpServers": {
                "other-tool": {
                    "command": "/usr/bin/other",
                    "args": ["run"]
                }
            }
        });
        fs::write(
            &config_path,
            serde_json::to_string_pretty(&existing).unwrap(),
        )
        .unwrap();

        // Temporarily override config_path_for by writing directly
        write_json_config(&Editor::ClaudeCode, &config_path, &fake_binary()).unwrap();

        let content = fs::read_to_string(&config_path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&content).unwrap();

        // Both entries must be present
        assert!(value["mcpServers"]["other-tool"].is_object());
        assert!(value["mcpServers"]["hyphae"].is_object());
        assert_eq!(
            value["mcpServers"]["hyphae"]["command"].as_str().unwrap(),
            "/usr/local/bin/hyphae"
        );
    }

    #[test]
    fn test_creates_config_from_scratch() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("mcp.json");

        assert!(!config_path.exists());
        write_json_config(&Editor::Cursor, &config_path, &fake_binary()).unwrap();

        assert!(config_path.exists());
        let content = fs::read_to_string(&config_path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert_eq!(
            value["mcpServers"]["hyphae"]["command"].as_str().unwrap(),
            "/usr/local/bin/hyphae"
        );
        assert_eq!(
            value["mcpServers"]["hyphae"]["args"][0].as_str().unwrap(),
            "serve"
        );
    }

    #[test]
    fn test_backup_created_before_modification() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("settings.json");

        let original = serde_json::json!({ "existing": true });
        fs::write(
            &config_path,
            serde_json::to_string_pretty(&original).unwrap(),
        )
        .unwrap();

        write_json_config(&Editor::Zed, &config_path, &fake_binary()).unwrap();

        let bak_path = backup_path(&config_path);
        assert!(
            bak_path.exists(),
            "backup file should exist at {}",
            bak_path.display()
        );

        let bak_content = fs::read_to_string(&bak_path).unwrap();
        let bak_value: serde_json::Value = serde_json::from_str(&bak_content).unwrap();
        assert!(bak_value["existing"].as_bool().unwrap());
    }

    #[test]
    fn test_vscode_uses_servers_key() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("settings.json");

        write_json_config(&Editor::VsCode, &config_path, &fake_binary()).unwrap();

        let content = fs::read_to_string(&config_path).unwrap();
        let value: serde_json::Value = serde_json::from_str(&content).unwrap();
        assert!(value["servers"]["hyphae"].is_object());
        assert_eq!(
            value["servers"]["hyphae"]["type"].as_str().unwrap(),
            "stdio"
        );
    }

    #[test]
    fn test_codex_cli_writes_toml() {
        let dir = TempDir::new().unwrap();
        let config_path = dir.path().join("config.toml");

        write_toml_config(&config_path, &fake_binary()).unwrap();

        let content = fs::read_to_string(&config_path).unwrap();
        let value: toml::Value = toml::from_str(&content).unwrap();
        assert_eq!(
            value["mcp_servers"]["hyphae"]["command"].as_str().unwrap(),
            "/usr/local/bin/hyphae"
        );
    }
}
