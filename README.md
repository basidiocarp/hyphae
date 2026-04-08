# Hyphae

Persistent memory for AI coding agents. Single binary, zero runtime
dependencies, MCP-native, and designed to keep useful context alive after the
window compacts.

Named after fungal hyphae, the branching filaments that connect and distribute
nutrients through the organism.

Part of the [Basidiocarp ecosystem](https://github.com/basidiocarp).

---

## The Problem

AI agents forget everything between sessions. Architecture decisions, resolved
bugs, project conventions, and prior corrections vanish when the transcript
compacts or the session ends.

## The Solution

Hyphae gives agents two memory models that do different jobs. Memories handle
the day-to-day flow of decisions, errors, and notes with decay; memoirs keep
the durable concept graph that should not disappear. On top of that, Hyphae
adds hybrid retrieval, document indexing, and session tracking.

---

## The Ecosystem

| Tool | Purpose |
|------|---------|
| **[hyphae](https://github.com/basidiocarp/hyphae)** | Persistent agent memory |
| **[canopy](https://github.com/basidiocarp/canopy)** | Multi-agent coordination runtime |
| **[cap](https://github.com/basidiocarp/cap)** | Web dashboard for the ecosystem |
| **[cortina](https://github.com/basidiocarp/cortina)** | Lifecycle signal capture and session attribution |
| **[lamella](https://github.com/basidiocarp/lamella)** | Skills, hooks, and plugins for coding agents |
| **[mycelium](https://github.com/basidiocarp/mycelium)** | Token-optimized command output |
| **[rhizome](https://github.com/basidiocarp/rhizome)** | Code intelligence via tree-sitter and LSP |
| **[spore](https://github.com/basidiocarp/spore)** | Shared transport and editor primitives |
| **[stipe](https://github.com/basidiocarp/stipe)** | Ecosystem installer and manager |
| **[volva](https://github.com/basidiocarp/volva)** | Execution-host runtime layer |

> **Boundary:** `hyphae` owns memory, retrieval, and session records. It does
> not own shell filtering, code intelligence, hook capture, UI, or installation.

---

## Quick Start

```bash
# Quick install
curl -fsSL https://raw.githubusercontent.com/basidiocarp/hyphae/main/install.sh | sh

# Recommended: full ecosystem setup
stipe init

# Alternative: hyphae-only setup
hyphae init
```

```bash
# Build from source
cargo install --path crates/hyphae-cli

# Smaller build without embeddings
cargo build --release --no-default-features
```

---

## How It Works

```text
Agent                   Hyphae                         Stored state
─────                   ──────                         ────────────
store memory      ─►    episodic memory         ─►    decaying memories
store concept     ─►    memoir graph            ─►    permanent concepts
query context     ─►    hybrid retrieval        ─►    ranked recall
end session       ─►    session lifecycle       ─►    outcomes and lessons
```

1. Store episodic memories: capture decisions, errors, preferences, and session notes with importance-aware decay.
2. Build memoirs: link durable concepts into permanent knowledge graphs.
3. Index documents: chunk files, embed them, and store them for hybrid RAG retrieval.
4. Track sessions: record task context, outcomes, files changed, and feedback signals.
5. Recall useful context: blend BM25 and vector search into ranked results for agents and UIs.

---

## Memory Models

| Model | Behavior | Best for |
|-------|----------|----------|
| Memories | Decay-based episodic storage | Decisions, errors, preferences, work notes |
| Memoirs | Permanent semantic graph | Concepts, relationships, architecture, domain knowledge |

## Hybrid Search Stack

| Layer | Technology | Purpose |
|-------|------------|---------|
| Storage | SQLite | Memories, memoirs, embeddings, session state |
| Full-text | FTS5 | Keyword recall with BM25 scoring |
| Vector | sqlite-vec | Semantic recall over embeddings |
| Blend | 30% FTS plus 70% vector | Keyword precision plus semantic similarity |

---

## What Hyphae Owns

- Episodic memory storage and decay
- Permanent knowledge memoirs
- Hybrid document and memory retrieval
- Session lifecycle records and outcome signals
- Training-data export and lesson extraction

## What Hyphae Does Not Own

- Shell output filtering: handled by `mycelium`
- Code intelligence and symbol graphs: handled by `rhizome`
- Hook capture and session intake: handled by `cortina`
- UI and operator dashboards: handled by `cap`

---

## Key Features

- Dual memory model: combines decay-based episodic memory with permanent semantic memoirs.
- RAG pipeline: ingests files, chunks them, embeds them, and serves them back through hybrid search.
- Structured sessions: records session start, end, context, and feedback signals.
- Lesson extraction: mines corrections and resolutions into reusable patterns.
- Local-first storage: runs from a single SQLite database with no cloud dependency.

---

## Architecture

```text
hyphae (single binary)
├── hyphae-core    types, traits, embedder logic
├── hyphae-ingest  file readers and chunking
├── hyphae-store   SQLite, FTS5, sqlite-vec
├── hyphae-mcp     MCP server and tool handlers
└── hyphae-cli     CLI commands and operator surfaces
```

```bash
hyphae session start --project demo --task "refactor auth flow"
hyphae session end --id <session_id> --summary "completed refactor"
hyphae feedback signal --session-id <session_id> --type correction --value -1
hyphae session context --project demo
```

---

## Performance

| Operation | Latency |
|-----------|---------|
| Store | 34 us |
| FTS search | 47 us |
| Hybrid search | 951 us |
| Batch decay (1000) | 5.8 ms |

## Logging

Hyphae reads `HYPHAE_LOG` first, then falls back to `RUST_LOG`. If neither is
set, it defaults to `warn`.

```bash
HYPHAE_LOG=debug hyphae doctor
HYPHAE_LOG=debug hyphae serve
```

`hyphae serve` keeps stdout reserved for newline-delimited MCP JSON-RPC
responses. Logs go to stderr so they do not corrupt the MCP transport.

---

## Documentation

- [docs/GUIDE.md](docs/GUIDE.md): quickstart, concepts, and configuration
- [docs/FEATURES.md](docs/FEATURES.md): feature overview and behavior
- [docs/CLI-REFERENCE.md](docs/CLI-REFERENCE.md): CLI commands and examples
- [docs/MCP-TOOLS.md](docs/MCP-TOOLS.md): MCP tool reference
- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md): internals, schema, and search pipeline
- [docs/SETUP-BY-TOOL.md](docs/SETUP-BY-TOOL.md): per-editor setup instructions
- [docs/TROUBLESHOOTING.md](docs/TROUBLESHOOTING.md): common issues and fixes
- [docs/FEEDBACK-LOOP-DESIGN.md](docs/FEEDBACK-LOOP-DESIGN.md): closed-loop learning design notes
- [docs/TRAINING-DATA.md](docs/TRAINING-DATA.md): export formats and training data guidance

## Development

```bash
cargo build --release
cargo test
cargo clippy
cargo fmt
```

## License

MIT
