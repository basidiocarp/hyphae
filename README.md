# Hyphae

Persistent memory for AI coding agents. Single binary, zero runtime dependencies, MCP-native.

Part of the [Basidiocarp ecosystem](https://github.com/basidiocarp) — see the [Technical Overview](https://github.com/basidiocarp/.github/blob/main/profile/README.md#technical-overview) for how Hyphae fits with Mycelium, Rhizome, Cap, and Lamella.

## The Problem

AI agents forget everything between sessions. Architecture decisions, resolved bugs, project conventions; all lost when the context window compacts or a session ends.

## The Solution

Hyphae gives your agent permanent memory with two complementary models:

- **Memories** (episodic) — temporal storage with decay. Important decisions persist, trivial notes fade naturally.
- **Memoirs** (semantic) — permanent knowledge graphs. Concepts are refined, never forgotten.

## Installation

```bash
# Quick install (macOS / Linux)
curl -fsSL https://raw.githubusercontent.com/basidiocarp/hyphae/main/install.sh | sh

# Build from source
cargo install --path crates/hyphae-cli

# Build from source without embeddings (faster compile, smaller binary)
cargo build --release --no-default-features
```

Then configure your editors and agent runtimes:

```bash
hyphae init
```

This auto-detects and configures the supported integrations. Your setup now has access to 35 MCP tools.

## How It Works

### Memories — what happened

Store decisions, errors, preferences. They're organized by topic and decay over time unless accessed or marked important.

```
Agent stores:  "Switched from REST to gRPC for internal services"
               topic: decisions/backend, importance: high

Next session:  Agent recalls decisions/backend → knows about gRPC choice
               No re-reading architecture docs, no re-explaining decisions
```

### Memoirs — what's true

Build permanent knowledge graphs. Concepts link to each other with typed relations and are refined over time, never deleted.

```
Agent learns:  AuthService --uses--> JWT, JWT --requires--> RSA256
               PaymentService --depends-on--> AuthService

Next session:  Agent queries the "backend" memoir
               → sees the full dependency graph, knows what connects to what
```

## Key Features

- **Two memory models** — episodic (decay-based) + semantic (knowledge graphs)
- **RAG pipeline** — ingest files with automatic chunking, hybrid search, auto-context injection on session start
- **Training data export** — `hyphae export-training --format sft|dpo|alpaca` exports memories as training JSONL for fine-tuning
- **Session lifecycle tracking** — `hyphae session start|end|context` records structured coding sessions for later recall and operator views
- **Structured feedback signals** — `hyphae feedback signal` records corrections, recoveries, and session outcomes for recall-to-action tuning
- **Backup & restore** — `hyphae backup` / `hyphae restore` for database persistence and recovery
- **Secrets scanning** — 8 regex patterns detect API keys, tokens, and passwords during memory storage with non-blocking warnings
- **Evaluation framework** — `hyphae evaluate --days 14` measures agent improvement over time across 6 metrics
- **Feedback loop** — `hyphae_extract_lessons` reads captured corrections and error resolutions to surface actionable patterns
- **Auto-context injection** — MCP server injects recent sessions, decisions, and resolved errors into agent context on initialization
- **Zero LLM cost** — rule-based fact extraction, local embeddings, no API calls for storage
- **Local-first** — single SQLite file, no cloud, no network dependency
- **Hybrid search** — 30% BM25 full-text + 70% cosine similarity via sqlite-vec

## Technical Deep Dives

### Vector Database & Hybrid Search

| Layer | Technology | Purpose |
|-------|-----------|---------|
| Storage | SQLite (bundled) | Memories, memoirs, embeddings, chunks |
| Full-text search | FTS5 (BM25) | Keyword-based recall with relevance scoring |
| Vector search | sqlite-vec (HNSW) | Cosine similarity for semantic search |
| Hybrid pipeline | 30% FTS5 + 70% vector | Keyword precision + semantic understanding |
| Embeddings | fastembed (local, 384-dim) or HTTP (Ollama/OpenAI) | Text → vector conversion |

### Memory Decay Model

```
effective_rate = base_decay × importance_multiplier / (1 + access_count × 0.1)
```

| Importance | Multiplier | Behavior |
|-----------|-----------|----------|
| Critical | 0 (never decays) | Permanent knowledge |
| High | 0.5x | Slow decay |
| Medium | 1x | Normal decay |
| Low | 2x | Fast decay |

### RAG Pipeline

```
Ingestion → Chunking → Embedding → Storage → Hybrid Search → Context Injection
```

- **Ingestion**: `hyphae_ingest_file` with 3 chunking strategies (sliding window, by heading, by function)
- **Search**: `hyphae_search_docs` / `hyphae_search_all` with hybrid FTS5+vector
- **Auto-indexing**: Lamella hook triggers `hyphae ingest-file` when 3+ document files change
- **Auto-context**: MCP initialize response includes recent sessions, decisions, errors

### Feedback & Lesson Extraction

Lamella hooks capture corrections, errors, test failures, and PR reviews into Hyphae. The `hyphae_extract_lessons` tool reads these signals, groups by keyword overlap, and returns actionable patterns:

```
"When working with 'parsing', avoid tokens parse — resolved 3 times"
"Test failures in 'auth': avoid null check — fixed 2 times"
```

Hyphae also exposes a structured session lifecycle:

```bash
hyphae session start --project demo --task "refactor auth flow"
hyphae session end --id ses_... --summary "completed refactor" --file src/auth.rs --errors 0
hyphae session context --project demo
hyphae session status --id ses_...
hyphae feedback signal --session-id ses_... --type correction --value -1 --source cortina.post_tool_use

# Parallel runtimes can opt into separate active sessions per project.
hyphae session start --project demo --scope worker-a --task "run validation"
hyphae session context --project demo --scope worker-a
```

This is the bridge Cortina now uses when it turns hook activity into session
records instead of only storing free-form summaries. Hyphae now also records
recall events and structured outcome signals so those session results can be
correlated later instead of living only as topic memories. MCP callers can also
pass an explicit `session_id` to `hyphae_memory_recall` so recall attribution
stays attached to the right scoped session. Cortina now validates cached session
state with the structured `hyphae session status` surface instead of scraping
human-readable `session context` output.

## Performance

| Operation | Latency |
|-----------|---------|
| Store | 34 µs |
| FTS search | 47 µs |
| Hybrid search | 951 µs |
| Batch decay (1000) | 5.8 ms |

## Architecture

```
hyphae (single binary)
├── hyphae-core    Types, traits, embedder (no I/O)
├── hyphae-ingest  File readers + chunking logic (no database dependency)
├── hyphae-store   SQLite + FTS5 + sqlite-vec
├── hyphae-mcp     MCP server (31+ tools, JSON-RPC 2.0 over stdio)
└── hyphae-cli     CLI commands, config, extraction, benchmarks
```

## Documentation

- [User Guide](docs/GUIDE.md) — quickstart, memory models, configuration
- [Features](docs/FEATURES.md) — conceptual guides, topic hygiene, decay model
- [CLI Reference](docs/CLI-REFERENCE.md) — all CLI commands with examples
- [MCP Tools](docs/MCP-TOOLS.md) — all MCP tool definitions
- [Architecture](docs/ARCHITECTURE.md) — traits, schema, search pipeline, decay model
- [Setup by Tool](docs/SETUP-BY-TOOL.md) — per-editor configuration details
- [Troubleshooting](docs/TROUBLESHOOTING.md) — common issues and fixes

## License

MIT
