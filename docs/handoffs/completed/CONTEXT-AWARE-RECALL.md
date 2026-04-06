# Context-Aware Recall Handoff

## Task

Implement the next `hyphae` recall slice: make `hyphae_memory_recall` more
context-aware without broadening ownership beyond recall itself.

There is already useful logic in place:

- session-query boosting
- optional `code_context` expansion
- identity-v1 worktree scoping

This slice should unify and tighten those behaviors so recall uses the current
query context more deliberately and predictably.

## Ownership

Write scope:

- `hyphae/crates/hyphae-mcp/src/tools/memory.rs`
- `hyphae/crates/hyphae-store/src/store/context.rs` only if a small heuristic
  helper is needed
- `hyphae/crates/hyphae-mcp/src/tools/schema.rs` if input schema/help needs
  updates
- `hyphae/docs/MCP-TOOLS.md`
- `hyphae/docs/FEATURES.md`
- `hyphae/docs/GUIDE.md`

Read-only context:

- `hyphae/docs/ARCHITECTURE.md`
- `hyphae/docs/INTERNALS.md`
- `hyphae/crates/hyphae-mcp/src/tools/context.rs`
- `hyphae/crates/hyphae-store/src/store/context.rs`
- `hyphae/crates/hyphae-mcp/src/tools/mod.rs`

You are not alone in the codebase. Do not revert others' edits. Adjust to the
current file contents if they changed while you were working.

## Goal

Improve recall quality by making recall more aware of the query context while
staying inside `hyphae_memory_recall`.

Good target outcomes:

- code-heavy queries benefit from the existing code-context expansion in a more
  intentional way
- session/history queries continue to surface the right session memories first
- the behavior is documented as context-aware recall rather than an opaque set
  of heuristics

## Constraints

- Do not add orchestration or agent policy
- Do not add `search-all` work here
- Do not redesign memory scoring broadly unless needed for this slice
- Keep the change scoped to recall behavior and documentation
- Preserve graceful degradation when code memoirs or identity context are absent

## Recommended Design

Likely useful directions:

- factor current recall heuristics into clearer helper functions or stages
- make the query-context decisions explicit and testable
- if needed, add one or two narrow new heuristics, but avoid a large heuristic
  pile-up
- keep output shape stable unless a small documentation/schema note is needed

Tests should make the context-aware behavior obvious. For example:

- session-shaped query boosts `session/*` memories
- code-related query with `code_context: true` expands through `code:{project}`
- non-code queries do not trigger code expansion
- identity-v1 scoped recall still respects the active worktree

## Acceptance Criteria

- recall behavior is more clearly context-aware and better structured
- tests cover the key query-context branches
- docs explain the behavior in user-facing terms
- no drift into unrelated search or orchestration work

## Validation

Run:

```bash
cd /Users/williamnewton/projects/claude-mycelium/hyphae
cargo test -p hyphae-mcp -p hyphae-store -p hyphae-cli
cargo fmt --all --check
```

## Deliverable

Return:

- what changed
- files changed
- tests run
- any remaining limitations in the recall heuristics
