# Hyphae Agent Notes

## Purpose

Hyphae owns persistent memory, retrieval, sessions, and document indexing. Work here should keep core domain types narrow, storage in the store crate, ingest in the ingest crate, and transport surfaces in CLI or MCP crates. Hyphae stores and retrieves knowledge; it should not absorb shell filtering or code-intelligence behavior.

---

## Source of Truth

- `crates/hyphae-core/`: domain types, storage traits, and embedder abstractions.
- `crates/hyphae-store/`: SQLite-backed memory and memoir implementations.
- `crates/hyphae-ingest/`: chunking and ingest logic.
- `crates/hyphae-mcp/`: MCP tool handlers and schemas.
- `crates/hyphae-cli/`: CLI commands and operator surfaces.
- `../septa/`: authoritative schemas for inbound cross-repo payloads.
- `../ecosystem-versions.toml`: shared dependency pins.

If a cross-repo Hyphae boundary changes, update `../septa/` first.

---

## Before You Start

Before writing code, verify:

1. **Owning crate**: keep domain types in core, storage in store, ingest in ingest, and transport surfaces in CLI or MCP.
2. **Contracts**: if Mycelium, Rhizome, or Cortina payloads change, read the matching `../septa/` files first.
3. **Feature surface**: decide whether the change affects CLI output, MCP tools, store behavior, or all of them.
4. **Validation target**: choose the narrowest crate or integration tests that prove the change.

---

## Preferred Commands

Use these for most work:

```bash
cargo build --release
cargo test
```

For targeted work:

```bash
cargo build --release --no-default-features
cargo test -p hyphae-store
cargo test --ignored
cargo clippy
cargo fmt --check
```

---

## Repo Architecture

Hyphae is healthiest when the core model, storage, ingest pipeline, and transport surfaces stay separate.

Key boundaries:

- `hyphae-core`: no I/O, transport, or operator behavior.
- `hyphae-store`: SQLite ownership and query behavior.
- `hyphae-ingest`: chunking and document ingestion.
- `hyphae-mcp`: MCP tool dispatch.
- `hyphae-cli`: CLI read and write surfaces.

Current direction:

- Keep CLI and MCP read models aligned through shared domain types.
- Keep cross-repo payloads explicit and versioned.
- Keep hybrid search and ingestion changes tested against real fixtures where possible.

---

## Working Rules

- Do not move storage or I/O behavior into `hyphae-core`.
- Treat CLI and MCP payloads as first-class regression surfaces.
- When a boundary crosses repos, update `../septa/`, the receiver, and the emitter together.
- Prefer real fixtures and stored examples over synthetic stand-ins for search and ingest behavior.
- Keep memoir and episodic-memory behavior distinct unless the change intentionally spans both.
- Validate septa contracts after changing any cross-project payload: `cd septa && bash validate-all.sh`

---

## Multi-Agent Patterns

For substantial Hyphae work, default to two agents:

**1. Primary implementation worker**
- Owns the touched crate or feature slice
- Keeps the write scope inside Hyphae unless a real contract update requires `../septa/`

**2. Independent validator**
- Reviews the broader shape instead of redoing the implementation
- Specifically looks for core-layer leakage, contract drift, MCP-vs-CLI shape mismatches, and storage regressions

Add a docs worker when `README.md`, `CLAUDE.md`, `AGENTS.md`, or public docs changed materially.

---

## Skills to Load

Use these for most work in this repo:

- `basidiocarp-rust-repos`: repo-local Rust workflow and validation habits
- `systematic-debugging`: before fixing unexplained retrieval or storage failures
- `writing-voice`: when touching README or docs prose

Use these when the task needs them:

- `test-writing`: when behavior changes need stronger coverage
- `basidiocarp-workspace-router`: when the change may spill into `septa`, `mycelium`, `rhizome`, or `cortina`
- `tool-preferences`: when exploration should stay tight

---

## Done Means

A task is not complete until:

- [ ] The change is in the right crate and layer
- [ ] The narrowest relevant validation has run, when practical
- [ ] Related schemas, fixtures, docs, or transport surfaces are updated if they should move together
- [ ] Any skipped validation or follow-up work is stated clearly in the final response

If validation was skipped, say so clearly and explain why.
