# Search-All Identity-v1 Scoping Handoff

## Task

Implement the next `hyphae` retrieval slice: make the `search-all` surface
consistently identity-v1 aware across the relevant CLI and MCP paths.

Today the MCP `hyphae_search_all` tool already accepts `project_root` and
`worktree_id`, and the store has a scoped `search_all_scoped(...)` path. This
slice should tighten the surface so identity-v1 scoping is explicit, consistent,
and validated end to end.

## Ownership

Write scope:

- `hyphae/crates/hyphae-mcp/src/tools/ingest.rs`
- `hyphae/crates/hyphae-store/src/store/search.rs` only if a narrow store fix is
  needed
- `hyphae/crates/hyphae-mcp/src/tools/schema.rs` if schema/help needs updates
- `hyphae/crates/hyphae-cli/src/main.rs`
- `hyphae/crates/hyphae-cli/src/commands/docs.rs`
- `hyphae/docs/FEATURES.md`
- `hyphae/docs/CLI-REFERENCE.md`
- `hyphae/docs/MCP-TOOLS.md`

Read-only context:

- `hyphae/docs/ARCHITECTURE.md`
- `hyphae/docs/INTERNALS.md`
- `hyphae/crates/hyphae-mcp/src/tools/context.rs`
- `hyphae/crates/hyphae-mcp/src/tools/memory.rs`

You are not alone in the codebase. Do not revert others' edits. Adjust to the
current file contents if they changed while you were working.

## Goal

Ensure `search-all` behaves predictably when the identity-v1 pair is present:

- memory results should scope to the active worktree
- `_shared` memories should remain visible
- document chunks should remain project-scoped unless the store already supports
  stronger identity for docs
- partial identity input should be rejected consistently

This slice should also make the CLI surface able to express the same identity-v1
shape if it cannot today.

## Constraints

- Do not redesign the chunk store
- Do not add document worktree identity if it is not already modeled
- Do not broaden this into `gather_context` or `memory_recall`
- Keep the change focused on `search-all`
- Preserve graceful degradation when identity-v1 is absent

## Recommended Design

Things to verify and likely improve:

- CLI `hyphae search-all` argument surface for `project_root` + `worktree_id`
- MCP input validation parity with other identity-v1 tools
- docs describing exactly what is and is not scoped under identity-v1
- tests proving:
  - scoped worktree memory inclusion
  - `_shared` memory inclusion
  - other worktree memory exclusion
  - doc chunks remain project-scoped
  - partial identity pair is rejected

If the store layer is already correct, prefer keeping the change at CLI/MCP/docs
plus tests. Only change `hyphae-store` if you find a real bug there.

## Acceptance Criteria

- `search-all` has a clear identity-v1 story across CLI, MCP, and docs
- partial identity input is rejected consistently
- tests cover worktree-scoped memories and project-scoped docs
- no drift into broader context/recall logic

## Validation

Run:

```bash
cd /Users/williamnewton/projects/claude-mycelium/hyphae
cargo test -p hyphae-store -p hyphae-mcp -p hyphae-cli
cargo fmt --all --check
```

## Deliverable

Return:

- what changed
- files changed
- tests run
- any remaining limitation in identity-v1 scoping
