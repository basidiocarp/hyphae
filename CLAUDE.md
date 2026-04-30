# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

Hyphae is a persistent memory system for AI coding agents. It is a five-crate Rust workspace that builds one binary, exposes 40 MCP tools by default (plus one conditional `hyphae_memory_embed_all` when an embedder is available), and stores memory in SQLite with FTS5 and sqlite-vec. The current `cargo test -- --list` surface is 619 tests. Hyphae owns memory, retrieval, sessions, and document indexing; it does not own shell filtering, code intelligence, or lifecycle capture.

---

## Operating Model

- Do not execute code or shell commands. Hyphae stores, indexes, and retrieves data.
- Do not assume cross-machine sync. Storage is local-first and SQLite-backed.
- Do not treat decay as immediate deletion. Decay affects ranking, not automatic removal.
- Do not auto-ingest files without an explicit call or external trigger.
- Do not collapse code intelligence into memoir storage. Rhizome still owns code analysis.

---

## Failure Modes

- **Embeddings disabled or unavailable**: falls back to FTS-only search.
- **SQLite locked**: waits briefly via SQLite busy timeout, then fails with a clear storage error.
- **Embedding model unavailable**: vector-backed search cannot initialize until the model is available.
- **Database corruption**: storage commands fail until repaired.
- **Out of disk space**: writes fail even though reads may continue.

---

## State Locations

| What | Path |
|------|------|
| SQLite database | `~/.local/share/hyphae/hyphae.db` (`HYPHAE_DB`) |
| Config file | `~/.config/hyphae/config.toml` (`HYPHAE_CONFIG`) |
| Embedding cache | fastembed or model cache location |
| Log output | stderr (`HYPHAE_LOG`) |

---

## Build & Test Commands

```bash
cargo build --release
cargo build --release --no-default-features
cargo install --path crates/hyphae-cli

cargo test
cargo test -p hyphae-store
cargo test test_name
cargo test --ignored

cargo clippy
cargo fmt --check
cargo fmt
```

---

## Architecture

```text
hyphae-cli ───────► hyphae-ingest ──► hyphae-core
   │                     ▲                ▲
   ├────► hyphae-store ──┼────────────────┘
   └────► hyphae-mcp ────┘
```

- **hyphae-core**: domain types, store traits, and embedder abstractions. No
  I/O, transport, or operator surfaces. Keep this crate narrow.
- **hyphae-store**: SQLite implementation of memory and memoir storage.
- **hyphae-ingest**: chunking and file-ingestion logic.
- **hyphae-mcp**: MCP server and tool handlers.
- **hyphae-cli**: CLI commands, extraction, config, and operator-facing surfaces.

---

## Core Abstraction

```rust
pub trait MemoryStore {
    fn store_memory(&self, memory: &Memory) -> Result<MemoryId>;
    fn search_memories(&self, query: &str, limit: usize) -> Result<Vec<Memory>>;
    fn decay_memories(&self) -> Result<usize>;
}
```

`MemoryStore` is the center of the storage model. The SQLite store implements it, the CLI and MCP layers consume it, and most behavior changes eventually land here or in the memoir store that mirrors it for graph data.

---

## Key Design Decisions

- **SQLite with FTS and vectors**: keeps the storage model local, portable, and expressive enough for hybrid search.
- **Dual memory model**: episodic memory and permanent memoirs solve different retrieval problems.
- **Single binary from multiple crates**: keeps boundaries clean without forcing multi-process runtime complexity.
- **CLI and MCP contract coverage**: read models and payload shapes matter
  enough that CLI and MCP contract tests are a primary regression surface.
- **Versioned boundaries**: when a surface crosses a repo boundary, make the
  version explicit in the payload or schema instead of relying on implicit
  shape changes.

---

## Key Files

| File | Purpose |
|------|---------|
| `crates/hyphae-core/src/store.rs` | Core memory storage trait definitions |
| `crates/hyphae-core/src/memoir_store.rs` | Memoir graph storage traits |
| `crates/hyphae-store/src/` | SQLite-backed implementations |
| `crates/hyphae-mcp/src/tools/` | MCP tool handlers |
| `crates/hyphae-cli/src/commands/` | CLI command surfaces |

---

## MCP Tools

The following `mcp__hyphae__*` tools are available for Claude Code:

**Memory (episodic recall)**:
- `mcp__hyphae__hyphae_memory_recall`: search prior-session decisions and context
- `mcp__hyphae__hyphae_memory_store`: save a decision or error fix for later
- `mcp__hyphae__hyphae_memory_consolidate`: merge redundant memories
- `mcp__hyphae__hyphae_memory_invalidate`: mark a memory obsolete
- `mcp__hyphae__hyphae_memory_health`: check storage and decay status
- `mcp__hyphae__hyphae_memory_stats`: view memory distribution and topic breakdown

**Memoir (knowledge graphs)**:
- `mcp__hyphae__hyphae_memoir_create`: start a persistent knowledge graph
- `mcp__hyphae__hyphae_memoir_add_concept`: add a named node to a memoir
- `mcp__hyphae__hyphae_memoir_link`: connect concepts with relationships
- `mcp__hyphae__hyphae_memoir_refine`: deepen concept definitions
- `mcp__hyphae__hyphae_memoir_search`: query a memoir
- `mcp__hyphae__hyphae_memoir_inspect`: explore memoir structure
- `mcp__hyphae__hyphae_memoir_show`: display a memoir or concept

**Session management**:
- `mcp__hyphae__hyphae_session_start`: open a session
- `mcp__hyphae__hyphae_session_end`: close a session
- `mcp__hyphae__hyphae_session_context`: retrieve session metadata

**Cross-project and utility**:
- `mcp__hyphae__hyphae_recall_global`: search all projects when local recall is empty
- `mcp__hyphae__hyphae_extract_lessons`: extract patterns from error fixes
- `mcp__hyphae__hyphae_artifact_query`, `mcp__hyphae__hyphae_artifact_store`: store large outputs
- `mcp__hyphae__hyphae_gather_context`: assemble ambient context for a task
- `mcp__hyphae__hyphae_onboard`: initialize Hyphae with preferences
- `mcp__hyphae__hyphae_ingest_file`: bulk-ingest files into memory

**Ingestion and search**:
- `mcp__hyphae__hyphae_search_docs`: search ingested documents
- `mcp__hyphae__hyphae_search_all`: search all memory categories
- `mcp__hyphae__hyphae_list_sources`: list ingested file sources
- `mcp__hyphae__hyphae_forget_source`: remove ingested source
- `mcp__hyphae__hyphae_import_code_graph`: import code structure from Rhizome
- `mcp__hyphae__hyphae_code_query`: query imported code graphs

**Advanced**:
- `mcp__hyphae__hyphae_evaluate`: evaluate and rank memories by relevance
- `mcp__hyphae__hyphae_promote_to_memoir`: convert a memory to persistent memoir
- `mcp__hyphae__hyphae_memory_forget`: permanently delete a memory
- `mcp__hyphae__hyphae_memory_list_invalidated`: list obsolete memories
- `mcp__hyphae__hyphae_memory_list_topics`: list all memory topics
- `mcp__hyphae__hyphae_memory_update`: update a stored memory
- `mcp__hyphae__hyphae_store_command_output`: store command output for later retrieval
- `mcp__hyphae__hyphae_get_command_chunks`: retrieve stored command outputs
- `mcp__hyphae__hyphae_memoir_list`: list all memoirs
- `mcp__hyphae__hyphae_memoir_search_all`: search across all memoirs

Conditional (when embedder is configured):
- `mcp__hyphae__hyphae_memory_embed_all`: re-embed all memories for semantic search

---

## Communication Contracts

### Inbound (this project receives)

| Contract | Source | Protocol | Schema |
|----------|--------|----------|--------|
| `command-output-v1` | Mycelium | MCP `hyphae_store_command_output` | `septa/command-output-v1.schema.json` |
| `code-graph-v1` | Rhizome | MCP `hyphae_import_code_graph` | `septa/code-graph-v1.schema.json` |
| `session-event-v1` | Cortina | CLI `hyphae session end` and related writes | `septa/session-event-v1.schema.json` |

Hyphae emits first-party CLI read contracts that sibling tools consume. The main public surfaces are its MCP tools plus versioned CLI payloads for activity, analytics, lessons, and session timeline.

### Outbound (this project sends)

| Contract | Consumer | Protocol | Source |
|----------|----------|----------|--------|
| `hyphae-activity-v1` | Cap | CLI `hyphae activity` | `crates/hyphae-cli/src/commands/activity.rs` |
| `hyphae-analytics-v1` | Cap | CLI `hyphae analytics` | `crates/hyphae-cli/src/commands/analytics.rs` |
| `hyphae-lessons-v1` | Cap | CLI `hyphae lessons` | `crates/hyphae-cli/src/commands/lessons.rs` |
| `hyphae-session-timeline-v1` | Cap | CLI `hyphae session timeline` | `crates/hyphae-cli/src/commands/session.rs` |

**Receiver source files:**
- `crates/hyphae-mcp/src/tools/ingest.rs`
- `crates/hyphae-mcp/src/tools/memoir.rs`
- `crates/hyphae-mcp/src/tools/schema.rs`

Breaking change impact: command-output ingest, code-graph import, or session capture breaks at the boundary.

### Shared Dependencies

- **spore**: check `../ecosystem-versions.toml` before upgrading.
- **rusqlite 0.39**: Hyphae pins the bundled SQLite client at the workspace level.
- **CLI read contracts**: Cap shells out to Hyphae CLI commands and validates the returned versioned payloads.
- **JSON-RPC surface**: MCP callers depend on stable tool names and result shapes.

### Contract Validation

When changing output shapes that cross a project boundary, validate against septa:

```bash
cd ../septa && bash validate-all.sh
```

Check that this tool's schemas still pass before closing the change.

---

## Feature Flags

- `embeddings` (default: on): enables vector search and local embedding support; disable for faster iteration when search semantics are not under test.
- `vendored-openssl` (default: off): used for more portable builds.

---

## Testing Strategy

- CLI payload and MCP contract tests are the main defense for output and API-shape regressions.
- Store and ingest behavior should be tested with real fixtures where possible.
- Integration tests are marked `#[ignore]` and require a more complete environment.
- Search and schema changes should be checked against both MCP and CLI surfaces.
