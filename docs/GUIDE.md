# Hyphae User Guide

## What is Hyphae?

Hyphae is persistent memory for AI coding agents. It stores what your agent learns across sessions so architecture decisions, resolved bugs, and project conventions survive context window compaction.

## Quick Start

### 1. Install

```bash
# Homebrew
brew tap basidiocarp/tap && brew install hyphae

# Quick install
curl -fsSL https://raw.githubusercontent.com/basidiocarp/hyphae/main/install.sh | sh

# From source
cargo install --path crates/hyphae-cli
```

### 2. Setup

```bash
hyphae init
```

This auto-detects your AI tools and configures the MCP server. Supports 14 tools: Claude Code, Claude Desktop, Cursor, Windsurf, VS Code, Gemini, Zed, Amp, Amazon Q, Cline, Roo Code, Kilo Code, Codex CLI, OpenCode.

### 3. Use

That's it. Your agent now has access to 18 MCP tools. It uses them automatically based on the server instructions.

## Two Memory Models

Hyphae has two complementary memory systems — use both.

### Memories (Episodic)

For things that happen: decisions, errors, configurations, preferences. Organized by **topic**. Memories decay over time unless accessed or marked important.

```bash
# Store a decision
hyphae store -t "project-api" -c "Chose REST over GraphQL for v1 simplicity" -i high

# Store an error resolution
hyphae store -t "errors-resolved" -c "CORS issue fixed by adding origin header in nginx" -i medium -k "cors,nginx"

# Store a critical fact (never forgotten)
hyphae store -t "credentials" -c "Production DB is on port 5433, not 5432" -i critical

# Recall relevant context
hyphae recall "API design choices"
hyphae recall "nginx" --topic "errors-resolved"
hyphae recall "database" --keyword "postgres"
```

**Importance levels:**

| Level | Decay | Auto-prune | When to use |
|-------|-------|------------|-------------|
| `critical` | Never | Never | Core architecture, credentials, must-know facts |
| `high` | Slow (0.5x) | Never | Important decisions, recurring patterns |
| `medium` | Normal (1.0x) | Yes | Context, configurations, one-time fixes |
| `low` | Fast (2.0x) | Yes | Temporary notes, exploration results |

Decay is access-aware: memories recalled often decay slower. Formula: `decay / (1 + access_count × 0.1)`.

### Memoirs (Semantic)

For structured knowledge that should be permanent: architecture as a graph, concept relationships, domain models. Concepts are never decayed — they get refined.

```bash
# Create a knowledge container
hyphae memoir create -n "backend-arch" -d "Backend architecture decisions"

# Add concepts with labels
hyphae memoir add-concept -m "backend-arch" -n "user-service" \
  -d "Handles user registration, authentication, and profile management" \
  -l "domain:auth,type:microservice"

hyphae memoir add-concept -m "backend-arch" -n "postgres" \
  -d "Primary datastore for user and transaction data" \
  -l "type:database"

hyphae memoir add-concept -m "backend-arch" -n "redis" \
  -d "Session cache and rate limiting" \
  -l "type:database,domain:infra"

# Link concepts
hyphae memoir link -m "backend-arch" --from "user-service" --to "postgres" -r depends-on
hyphae memoir link -m "backend-arch" --from "user-service" --to "redis" -r depends-on

# Refine a concept (increments revision, increases confidence)
hyphae memoir refine -m "backend-arch" -n "user-service" \
  -d "Handles registration, auth (JWT + OAuth2), profile, and 2FA"

# Search within a memoir
hyphae memoir search -m "backend-arch" "authentication"
hyphae memoir search -m "backend-arch" "service" --label "domain:auth"

# Search across ALL memoirs
hyphae memoir search-all "database"

# Explore concept neighborhood (BFS traversal)
hyphae memoir inspect -m "backend-arch" "user-service" -D 2
```

**9 relation types:** `part_of`, `depends_on`, `related_to`, `contradicts`, `refines`, `alternative_to`, `caused_by`, `instance_of`, `superseded_by`.

Use `superseded_by` to mark obsolete facts instead of deleting them — the history is valuable.

## Topic Organization

Good topic naming helps recall. Suggested patterns:

| Pattern | Example | Use for |
|---------|---------|---------|
| `decisions-{project}` | `decisions-api` | Architecture and design choices |
| `errors-resolved` | `errors-resolved` | Bug fixes with their solutions |
| `preferences` | `preferences` | User coding style, tool preferences |
| `context-{project}` | `context-frontend` | Project-specific knowledge |
| `conventions-{project}` | `conventions-api` | Code style, naming, file structure |
| `credentials` | `credentials` | Ports, URLs, service names (use `critical`) |

## Memory Lifecycle

### Consolidation

When a topic accumulates many entries, consolidate them into a dense summary:

```bash
# See which topics need consolidation
hyphae health

# Consolidate (replaces all entries with one summary)
hyphae consolidate --topic "errors-resolved"

# Keep originals alongside the consolidated summary
hyphae consolidate --topic "errors-resolved" --keep-originals
```

Hyphae warns when a topic has >7 entries via the MCP `hyphae_memory_store` response.

### Decay and Pruning

```bash
# Manually apply decay (normally runs automatically on recall, every 24h)
hyphae decay
hyphae decay --factor 0.9    # Custom decay factor

# Preview what would be pruned
hyphae prune --threshold 0.2 --dry-run

# Actually prune
hyphae prune --threshold 0.1
```

### Health Check

```bash
hyphae stats                          # Global overview (counts, avg weight, date range)
hyphae topics                         # List all topics with entry counts
hyphae health                         # Per-topic hygiene report
hyphae health --topic "decisions-api" # Single topic
```

The health report flags:
- Topics needing consolidation (>7 entries)
- Stale entries (low weight, many accesses but not reinforced)
- Topics with no recent activity

## Auto-Extraction

Hyphae extracts facts from text without any LLM cost:

```bash
# Pipe any text
echo "Fixed the CORS bug by adding Access-Control-Allow-Origin to nginx.conf" | hyphae extract -p my-project

# Extract from a file
cat session-log.txt | hyphae extract -p my-project

# Preview without storing
echo "Switched from MySQL to PostgreSQL for JSONB support" | hyphae extract -p api --dry-run
```

Detected signals: architecture patterns, error resolutions, decisions, configurations, refactors, deployments.

## Context Injection

Inject relevant memories at session start:

```bash
hyphae recall-context "my-project backend API"
hyphae recall-context "authentication" --limit 20
```

Returns a formatted block ready for prompt prepending. Used by the SessionStart hook for automatic context loading.

## Embedding Configuration

Default: English-only embeddings for fast semantic search.

```bash
hyphae config    # Show current settings
```

Edit `~/.config/hyphae/config.toml`:

```toml
[embeddings]
# Default (recommended)
model = "BAAI/bge-small-en-v1.5"              # 384d, English, fastest

# More accurate
# model = "BAAI/bge-base-en-v1.5"             # 768d, English, more accurate

# Best accuracy
# model = "BAAI/bge-large-en-v1.5"            # 1024d, English, best accuracy

# Code-optimized
# model = "jinaai/jina-embeddings-v2-base-code"  # 768d, optimized for code
```

Changing the model automatically migrates the vector index on next startup (existing embeddings are cleared). Regenerate with:

```bash
hyphae embed                     # Embed all memories without embeddings
hyphae embed --force             # Re-embed everything
hyphae embed --topic "decisions" # Only one topic
```

## MCP Tools Reference

### Memory tools (9)

| Tool                        | What it does |
|-----------------------------|-------------|
| `hyphae_memory_store`       | Store a memory. Auto-dedup: >85% similar in same topic → update. Warns at >7 entries. |
| `hyphae_memory_recall`      | Search by query. Filters: `topic`, `keyword`, `limit`. Auto-decay if >24h. |
| `hyphae_memory_update`      | Edit content, importance, or keywords of an existing memory by ID. |
| `hyphae_memory_forget`      | Delete a memory by ID. |
| `hyphae_memory_consolidate` | Replace all memories of a topic with a single summary. |
| `hyphae_memory_list_topics` | List all topics with entry counts. |
| `hyphae_memory_stats`       | Total memories, topics, average weight, date range. |
| `hyphae_memory_health`      | Per-topic audit: staleness, consolidation needs, access patterns. |
| `hyphae_memory_embed_all`   | Backfill embeddings for memories that don't have one. |

### Memoir tools (9)

| Tool                        | What it does |
|-----------------------------|-------------|
| `hyphae_memoir_create`      | Create a named knowledge container. |
| `hyphae_memoir_list`        | List all memoirs with concept counts. |
| `hyphae_memoir_show`        | Show memoir details, stats, and all concepts. |
| `hyphae_memoir_add_concept` | Add a concept with definition and labels. |
| `hyphae_memoir_refine`      | Update a concept's definition (increments revision, boosts confidence). |
| `hyphae_memoir_search`      | Full-text search within a memoir, optionally filtered by label. |
| `hyphae_memoir_search_all`  | Search across all memoirs at once. |
| `hyphae_memoir_link`        | Create a typed relation between two concepts. |
| `hyphae_memoir_inspect`     | Inspect a concept and its graph neighborhood (BFS to depth N). |

## Init Modes

```bash
hyphae init                  # Auto-detect and configure MCP for all found tools
hyphae init --mode skill     # Install slash commands and rules
hyphae init --mode hook      # Install Claude Code PostToolUse hook for auto-extraction
hyphae init --mode cli       # Show manual CLI setup instructions
```

### Skills

`hyphae init --mode skill` installs:
- **Claude Code**: `/recall` and `/remember` slash commands
- **Cursor**: `.cursor/rules/hyphae.mdc` rule file
- **Roo Code**: `.roo/rules/hyphae.md` rule file
- **Amp**: `/hyphae-recall` and `/hyphae-remember` commands

## Compact Mode

For token-constrained environments:

```bash
hyphae serve --compact
```

Produces shorter MCP responses (~40% fewer tokens):
- Store: `ok:<id>` instead of `Stored memory: <id> [+ consolidation hint]`
- Recall: `[topic] summary` per line instead of multi-line verbose format

## Database

Single SQLite file with WAL mode. No external services.

```
macOS:   ~/Library/Application Support/dev.hyphae.hyphae/memories.db
Linux:   ~/.local/share/dev.hyphae.hyphae/memories.db
```

Override: `--db <path>` flag or `Hyphae_DB` environment variable.

## Benchmarking

```bash
# Storage performance (in-memory, single-threaded)
hyphae bench --count 1000

# Knowledge retention: can the agent recall facts across sessions?
hyphae bench-recall --model haiku --runs 5

# Agent efficiency: turns, tokens, cost with/without Hyphae
hyphae bench-agent --sessions 10 --model haiku --runs 3
```

All benchmarks use real API calls, no mocks. Each run uses its own tempdir and fresh DB.

## Quick walkthrough

### Install

```bash
brew tap basidiocarp/tap && brew install hyphae
```

Without Homebrew:

```bash
curl -fsSL https://raw.githubusercontent.com/basidiocarp/hyphae/main/install.sh | sh
```

### Configure

```bash
hyphae init
```

This detects your AI tools (Claude Code, Cursor, VS Code, etc.) and writes the MCP config for each.

### Store and recall

```bash
hyphae store -t "test" -c "My first Hyphae memory" -i high
hyphae recall "first memory"
```

The memory appears with its ID, topic, weight, and content. Verify with `hyphae topics` and `hyphae stats`.

### Test with your agent

Restart your AI tool and ask it to "recall the Hyphae context." It should call `hyphae_memory_recall` automatically. If not, see [TROUBLESHOOTING.md](TROUBLESHOOTING.md).

### From here

Store architecture decisions with `-i high`, invariant facts (ports, URLs) with `-i critical`, and bug fixes with descriptive keywords. When you have enough decisions in a topic, create a memoir to structure them as a graph.

---

## Next steps

- **Tool-specific setup** — MCP config files, slash commands, and hooks for every supported tool: [SETUP-BY-TOOL.md](SETUP-BY-TOOL.md)
- **Troubleshooting & FAQ** — Fix common issues and get answers to frequent questions: [TROUBLESHOOTING.md](TROUBLESHOOTING.md)

