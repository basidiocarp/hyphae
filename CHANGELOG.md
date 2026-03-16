# Changelog

## v0.3.1

### Added
- **Code graph import**: `hyphae_import_code_graph` MCP tool receives symbol graphs from Rhizome and stores them as memoirs with idempotent upsert (create/update/skip unchanged)
- **Code intelligence queries**: `hyphae_code_query` MCP tool with 5 query types: `symbols` (list/filter by labels), `callers`, `callees`, `implementors` (link traversal), and `structure` (BFS neighborhood depth 2)
- **Bulk memoir operations**: `upsert_concepts`, `upsert_links`, `prune_concepts` on `MemoirStore` trait for batch code graph imports with transactional atomicity
- **Symbol pruning**: Automatically removes concepts and cascades link cleanup when symbols are deleted from the codebase

## v0.3.0

### Added
- **Shell completions**: `hyphae completions <bash|zsh|fish|powershell>` generates shell completions via clap_complete
- **Init command**: `hyphae init` auto-detects editors (Claude Code, Cursor, VS Code, Zed, Windsurf, Amp, Claude Desktop, Codex CLI) and writes MCP server config with backup and merge
- **Multi-project support**: Namespace memories and documents by project with `--project` flag, config `store.default_project`, or auto-detection from git repo name
- **File watcher**: `hyphae watch <path>` monitors filesystem and auto-re-ingests changed files with debounced events and graceful shutdown
- **Project filtering**: All search and list operations optionally filter by project; `None` returns all (backward compatible)
- **MCP project scoping**: `hyphae serve --project <name>` scopes all MCP tool operations to a project namespace
- **TTL/expiry on memories**: `expires_at` field with auto-expiry for ephemeral importance (default 4 hours), `prune_expired()` to clean up
- **Ephemeral importance level**: New `Importance::Ephemeral` variant for short-lived context like sprint goals or temporary notes
- **Command output chunking**: `ByStructuredOutput` chunking strategy with auto-detection for test results, build errors, diffs, and log output
- **Command output MCP tools**: `hyphae_store_command_output` stores chunked command output with ephemeral TTL; `hyphae_get_command_chunks` retrieves chunks with pagination
- **Search pagination**: All search methods now accept `offset` parameter for paginated retrieval

### Changed
- CLI restructured with early-return commands (completions, config, init) that skip store/embedder initialization
- `MemoryStore` and `ChunkStore` traits now accept `project: Option<&str>` and `offset: usize` on search methods
- Schema auto-migrates to add `project`, `expires_at` columns on `memories` table and `project` on `documents`
- Hybrid search FTS query now includes all columns (`updated_at`, `project`, `expires_at`) for correct row mapping
- Removed cargo audit workflow (unmaintained transitive deps from fastembed are not actionable)

### CI/CD
- Add concurrency groups to all workflows to cancel stale runs on new pushes
- Add MSRV (1.85) check job
- Remove duplicate security-audit job from CI
- Fix coverage workflow running tests twice; now uses single `--json` invocation
- Combine binary-size and startup-time into single performance job
- Replace `cargo install` with `taiki-e/install-action` for hyperfine and cross (pre-built binaries)
- Add `rust-cache` and `--locked` to release builds for speed and reproducibility

## v0.2.0

### Added
- **RAG document ingestion**: `hyphae ingest <path>` ingests files/directories into a searchable vector store
- **Document chunking**: Automatic chunking with Sliding Window (text), By Heading (markdown), By Function (code) strategies
- **Unified search**: `hyphae search-all <query>` searches across memories and document chunks using Reciprocal Rank Fusion (RRF)
- **MCP RAG tools**: `hyphae_ingest_file`, `hyphae_search_docs`, `hyphae_list_sources`, `hyphae_forget_source`, `hyphae_search_all` (23 tools total)
- **New crate**: `hyphae-ingest` — file readers + chunking logic, no database dependency

## v0.1.0

### New Features

- **Two memory models**: Episodic memories with time-based decay and semantic memoirs as permanent knowledge graphs
- **18 MCP tools**: Full Model Context Protocol server over stdio — 9 memory tools + 9 memoir tools for any MCP-compatible AI agent
- **29 CLI commands**: Complete command-line interface for storing, recalling, searching, and managing memories and memoirs
- **Hybrid search**: 30% BM25 full-text (FTS5) + 70% cosine similarity (sqlite-vec) for high-quality recall
- **Local embeddings**: BGE-small-en-v1.5 via fastembed — zero API calls, zero cloud dependency
- **Rule-based fact extraction**: Automatically extract structured facts from conversation text without LLM calls
- **Importance-based decay**: Critical memories never fade, low-importance notes decay naturally over time
- **Knowledge graphs**: Build permanent concept maps with typed relations, labels, and graph traversal
- **One-command setup**: `hyphae init` auto-detects and configures Claude Code, Cursor, VS Code, Windsurf, Zed, Amp, and more
- **Single-file storage**: Everything in one SQLite database — portable, backupable, no external services

### Architecture

- 4-crate workspace: `hyphae-core` (types/traits), `hyphae-store` (SQLite), `hyphae-mcp` (JSON-RPC server), `hyphae-cli` (commands)
- Feature-gated embeddings: build with `--no-default-features` for fast iteration without the embedding model
- Auto-migrations on startup — schema evolves without manual steps
- Compact MCP mode for shorter responses and lower token usage

### Quality

- 211 tests across all crates (28 core, 93 store, 69 MCP, 21 CLI)
- Input validation on all MCP tool parameters (bounds checking, length limits, required fields)
- Transaction safety for all multi-table store operations
- NaN-safe numeric types (Weight, Confidence)
- CI pipeline: fmt, clippy, cross-platform tests (Linux/macOS/Windows), code coverage, performance guards, security audit
- Multi-target release pipeline (linux musl, macOS, Windows)
