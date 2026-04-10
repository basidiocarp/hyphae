# Code-Context Term Expansion Validation Handoff

## Task

Validate the merged implementation of the code-context term expansion fix in
`hyphae`.

The implementation should have addressed the local integration gap where
context-aware recall expected code expansion from a query like:

- `previous verify_token failure`

but the merged tree still failed because code-context expansion searched memoir
concepts with the full natural-language query instead of extracted code terms.

Your job is to review the merged implementation, not to re-argue the intended
design in the abstract.

## Ownership

Read-only review scope:

- `hyphae/crates/hyphae-store/src/store/context.rs`
- `hyphae/crates/hyphae-mcp/src/tools/memory.rs`
- any touched docs among:
  - `hyphae/docs/mcp-tools.md`
  - `hyphae/docs/features.md`
  - `hyphae/docs/guide.md`

You may run tests, but do not edit files.

## Review Focus

Look for concrete issues in this order:

1. Correctness
   - does extracted-term expansion actually fix the failing heuristic case?
   - are extracted terms used instead of the raw prose query?
   - are results deduped and bounded?
2. Behavioral regressions
   - do non-code queries stay no-op?
   - does the recall path still degrade gracefully when no code memoir exists?
3. Scope discipline
   - did the implementation stay narrow?
   - is there any drift into broad search redesign or unrelated scoring changes?
4. Verification
   - is the full local integration surface green?

## Expected Validation

Run:

```bash
cd /Users/williamnewton/projects/basidiocarp/hyphae
cargo test -p hyphae-store -p hyphae-mcp -p hyphae-cli
cargo fmt --all --check
```

## Deliverable

Return either:

- findings first, ordered by severity with exact file references

or:

- explicit confirmation that you found no issues in the merged implementation,
  plus any residual limitations worth noting
