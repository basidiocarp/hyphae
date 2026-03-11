# Hyphae

Persistent memory for AI coding agents. Single binary, zero runtime dependencies, MCP-native.

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

Then configure your AI tools:

```bash
hyphae init
```

This auto-detects and configures.

That's it. Your agent now has access to 18 MCP tools for storing and recalling context.

## How It Works

### Memories — what happened

Store decisions, errors, preferences. They're organized by topic and decay over time unless accessed or marked important.

```
Agent stores:  "Switched from REST to gRPC for internal services"
               topic: decisions-backend, importance: high

Next session:  Agent recalls decisions-backend → knows about gRPC choice
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
- **Zero LLM cost** — rule-based fact extraction, local embeddings, no API calls for storage
- **Local-first** — single SQLite file, no cloud, no network dependency
- **Hybrid search** — 30% BM25 full-text + 70% cosine similarity via sqlite-vec

## Performance

| Operation | Latency |
|-----------|---------|
| Store | 34 µs |
| FTS search | 47 µs |
| Hybrid search | 951 µs |
| Batch decay (1000) | 5.8 ms |

Benchmarked impact: +63% factual recall, -44% context tokens, -29% agent turns by session 3.

## Architecture

```
hyphae (single binary)
├── hyphae-core    Types, traits, embedder (no I/O)
├── hyphae-store   SQLite + FTS5 + sqlite-vec
├── hyphae-mcp     MCP server (JSON-RPC 2.0 over stdio)
└── hyphae-cli     29 CLI commands, config, extraction, benchmarks
```

## Documentation

- [User Guide](docs/GUIDE.md) — quickstart, memory models, configuration
- [Features](docs/FEATURES.md) — conceptual guides, topic hygiene, decay model
- [CLI Reference](docs/CLI-REFERENCE.md) — all 29 commands with examples
- [MCP Tools](docs/MCP-TOOLS.md) — all 18 MCP tool definitions
- [Architecture](docs/ARCHITECTURE.md) — traits, schema, search pipeline, decay model
- [Setup by Tool](docs/SETUP-BY-TOOL.md) — per-editor configuration details
- [Troubleshooting](docs/TROUBLESHOOTING.md) — common issues and fixes
- [Product Overview](docs/PRODUCT.md) — use cases, benchmarks, differentiators

## License

MIT
