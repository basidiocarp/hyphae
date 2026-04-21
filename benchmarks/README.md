# Retrieval Benchmarks

This directory contains fixture-driven benchmarks for testing retrieval quality of the Hyphae memory store.

## Fixture Format

Each JSON fixture file contains:

- `description`: A human-readable description of what the fixture tests
- `memories`: An array of memory objects to seed into the store
  - `topic`: The memory's topic/category
  - `content`: The memory's text content
  - `importance`: The memory's importance level (`critical`, `high`, `medium`, `low`)
- `queries`: An array of query objects to test
  - `query`: The search query string
  - `expected_rank_1_contains`: (optional) String that should appear in the #1 ranked result
  - `expected_top_k_contains`: (optional) String that should appear somewhere in the top K results
  - `k`: (optional, default=1) The K value for top-K checking
  - `description`: A human-readable description of what the query tests

## Running Benchmarks

```bash
cd hyphae
hyphae bench-retrieval
```

To specify a custom fixtures directory:

```bash
hyphae bench-retrieval --fixtures-dir /path/to/fixtures
```

## Example Fixture

```json
{
  "description": "Recent error should surface at rank 1",
  "memories": [
    {"topic": "errors/resolved", "content": "cargo build failed: linker error on macos", "importance": "high"},
    {"topic": "context/project", "content": "basidiocarp workspace setup", "importance": "medium"}
  ],
  "queries": [
    {"query": "linker error cargo build", "expected_rank_1_contains": "linker error", "description": "error should rank first"}
  ]
}
```

## Fixture Design Guidelines

- Use realistic memory content that closely mirrors actual usage
- Seed with 3-10 memories per fixture to test ranking in a realistic scenario
- Include low-relevance "noise" memories to test that relevant memories surface above clutter
- Verify both exact rank (rank 1) and loose rank (top K) requirements where appropriate
- Include a mix of high, medium, and low importance levels to test weight effects

## Interpretation

- **PASS**: All queries in the fixture returned results matching the expected criteria
- **FAIL**: One or more queries did not return expected results
- **ERROR**: The fixture could not be parsed or run (check JSON format and file permissions)

A successful run prints the count of passed and failed fixtures and exits with code 0 only if all fixtures pass.
