# Code-Context Term Expansion Handoff

## Task

Fix the local integration gap in `hyphae`'s context-aware recall slice by making
code-context expansion search on extracted code terms rather than the full raw
natural-language query.

Today this test fails in the merged tree:

- `tools::memory::tests::test_recall_heuristics_detect_session_and_code_expansion_terms`

The failing path is:

- `RecallHeuristics::detect(...)` in
  `hyphae/crates/hyphae-mcp/src/tools/memory.rs`
- `expand_with_code_context(...)` in
  `hyphae/crates/hyphae-store/src/store/context.rs`

The core issue is that `expand_with_code_context(...)` currently passes the full
query string into memoir concept FTS, so prose words like `previous` and
`failure` make the lookup too strict for symbol expansion.

## Ownership

Write scope:

- `hyphae/crates/hyphae-store/src/store/context.rs`
- `hyphae/crates/hyphae-mcp/src/tools/memory.rs`
- `hyphae/docs/mcp-tools.md` only if behavior wording needs a precise update
- `hyphae/docs/features.md` only if behavior wording needs a precise update
- `hyphae/docs/guide.md` only if behavior wording needs a precise update

Read-only context:

- `hyphae/crates/hyphae-store/src/store/search.rs`
- `hyphae/crates/hyphae-store/src/store/memoir_store.rs`
- `hyphae/crates/hyphae-mcp/src/tools/context.rs`
- `hyphae/docs/handoffs/CONTEXT-AWARE-RECALL.md`

You are not alone in the codebase. Do not revert others' edits. Adjust to the
current file contents if they changed while you were working.

## Goal

Keep the recall heuristic deterministic and cheap while improving code-context
expansion quality.

Desired behavior:

- detect likely code terms from the query
- use those extracted terms, not the full prose query, when expanding through
  `code:{project}` memoir concepts
- preserve graceful degradation when no code memoir exists or no code terms are
  present
- keep the heuristic narrow; do not build a new ranking subsystem

## Recommended Design

Implement a small, explicit extraction-and-search flow:

1. Extract likely code tokens from the query:
   - `snake_case`
   - `CamelCase`
   - path segments / file stems
   - other obvious code-shaped tokens only if already supported by existing
     helper logic
2. If no code tokens are found, return `Vec::new()`.
3. Search the `code:{project}` memoir using those extracted terms only.
4. Prefer a simple staged lookup:
   - exact concept-name matches first if cheap
   - FTS over extracted code tokens second
   - dedupe names and cap results
5. Keep `RecallHeuristics::detect(...)` using the expansion helper; do not move
   large policy logic into the MCP layer.

Avoid:

- using the full raw query for concept expansion
- broad fuzzy ranking work
- redesigning memoir FTS globally
- adding semantic search here

## Tests

Update or add focused tests proving:

- `"previous verify_token failure"` expands to `TokenValidator`
- non-code prose queries do not produce code-expansion terms
- extracted code terms remain bounded and deduped
- context-aware recall tests still pass after the change

If an existing failing test was asserting the right behavior, keep it and make
the implementation satisfy it.

## Acceptance Criteria

- the merged `cargo test -p hyphae-store -p hyphae-mcp -p hyphae-cli` surface is
  green
- code-context expansion uses extracted code terms rather than raw prose
- the behavior stays deterministic, cheap, and easy to explain
- no drift into broader search-system redesign

## Validation

Run:

```bash
cd /Users/williamnewton/projects/basidiocarp/hyphae
cargo test -p hyphae-store -p hyphae-mcp -p hyphae-cli
cargo fmt --all --check
```

## Deliverable

Return:

- what changed
- files changed
- tests run
- any remaining limitation in the code-term extraction heuristic
