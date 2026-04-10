# Hyphae Setup by Tool

Step-by-step MCP configuration for every AI coding tool Hyphae supports. In most cases `hyphae init` handles everything automatically—the sections below show what it does under the hood and how to configure manually when needed.

Paths below are examples for the relevant platform and editor combination. `hyphae init` resolves the correct config location automatically for each supported tool.

---

## Claude Code

**Setup:**
```bash
hyphae init  # Automatically configures ~/.claude.json
```

**Manual configuration:**
```bash
claude mcp add hyphae -- hyphae serve
```

**Config file:** `~/.claude.json`
```json
{
  "mcpServers": {
    "hyphae": {
      "command": "/path/to/hyphae",
      "args": ["serve"]
    }
  }
}
```

**Project-local instructions (optional):**
Add recall/store guidance through Lamella or your own `CLAUDE.md` workflow if you want reminder prompts alongside the MCP setup.

**PostToolUse hook (optional):**
```bash
hyphae init --mode hook
```
Installs the recommended Claude Code lifecycle hooks:
- `PostToolUse` for periodic context extraction after tool calls
- `PreCompact` for a lightweight capture marker before compaction
- `SessionEnd` for session-end breadcrumbs

**Compact mode recommended:** Claude Code benefits from compact mode to save tokens. Enable in `~/.config/hyphae/config.toml`:
```toml
[mcp]
compact = true
```

**Project-local instructions (optional):**
Use Lamella or maintain your own `CLAUDE.md` guidance if you want reminder instructions in addition to the MCP setup.

---

## Cursor

**Setup:**
```bash
hyphae init  # Automatically configures ~/.cursor/mcp.json
```

**Config file:** `~/.cursor/mcp.json`
```json
{
  "mcpServers": {
    "hyphae": {
      "command": "/path/to/hyphae",
      "args": ["serve"]
    }
  }
}
```

**Cursor rules (optional):**
Manage `.cursor/rules` through Lamella or your own editor policy if you want reminder rules alongside the MCP setup.

**After configuration:** Restart Cursor. The Hyphae tools appear in the MCP palette.

---

## VS Code / GitHub Copilot

**Setup:**
```bash
hyphae init  # Automatically configures ~/Library/.../Code/User/mcp.json
```

**Config file:**
- macOS: `~/Library/Application Support/Code/User/mcp.json`
- Linux: `~/.config/Code/User/mcp.json`

```json
{
  "servers": {
    "hyphae": {
      "command": "/path/to/hyphae",
      "args": ["serve"]
    }
  }
}
```

**Note:** VS Code uses `"servers"` instead of `"mcpServers"`. `hyphae init` handles this difference automatically.

---

## Windsurf

**Setup:**
```bash
hyphae init  # Automatically configures ~/.codeium/windsurf/mcp_config.json
```

**Config file:** `~/.codeium/windsurf/mcp_config.json`
```json
{
  "mcpServers": {
    "hyphae": {
      "command": "/path/to/hyphae",
      "args": ["serve"]
    }
  }
}
```

---

## Zed

**Setup:**
```bash
hyphae init  # Automatically configures ~/.zed/settings.json
```

Zed uses a different format with `context_servers`:
```json
{
  "context_servers": {
    "hyphae": {
      "command": {
        "path": "/path/to/hyphae",
        "args": ["serve"]
      },
      "settings": {}
    }
  }
}
```

---

## Amp

**Setup:**
```bash
hyphae init  # Automatically configures ~/.config/amp/settings.json
```

**Amp commands (optional):**
Manage custom Amp command files separately if you want reminder prompts in addition to the MCP setup.

---

## OpenAI Codex CLI

**Setup:**
```bash
hyphae init  # Automatically configures ~/.codex/config.toml
```

That config now includes both MCP registration and Codex lifecycle notifications:
```toml
notify = ["hyphae", "codex-notify"]

[mcp_servers.hyphae]
command = "/path/to/hyphae"
args = ["serve"]
```

`hyphae codex-notify` stores a compact turn summary for `agent-turn-complete` and normalized lifecycle notes for other Codex notify events.

**Config file (TOML):** `~/.codex/config.toml`
```toml
[mcp_servers.hyphae]
command = "/path/to/hyphae"
args = ["serve"]
```

---

## Claude Desktop

**Setup:**
```bash
hyphae init  # Automatically configures
```

**Config file:** `~/Library/Application Support/Claude/claude_desktop_config.json`
```json
{
  "mcpServers": {
    "hyphae": {
      "command": "/path/to/hyphae",
      "args": ["serve"]
    }
  }
}
```

---

## Other tools

Hyphae does not currently auto-configure Gemini, Amazon Q, Cline, Roo Code, Kilo Code, or OpenCode from `hyphae init`. If you need those integrations today, register the MCP server manually or manage the packaging through Lamella.

---

## See also

- [guide.md](guide.md) — Core guide: memory models, MCP tools reference, init modes
- [troubleshooting.md](troubleshooting.md) — Fix tool-detection issues, MCP connection errors, and more
