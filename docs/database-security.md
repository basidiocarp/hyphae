# Database Security Guidelines

This document outlines defensive practices for working with SQLite in Hyphae, with focus on parameterized queries and safe handling of JSON predicates.

## Parameterized Queries

Always use parameterized queries with `params![]` and `?N` placeholders. Never string-interpolate user data into SQL.

**Good:**
```rust
conn.query_row(
    "SELECT id FROM concepts WHERE memoir_id = ?1",
    params![memoir_id.as_ref()],
    |row| row.get::<_, String>(0),
)
```

**Bad:**
```rust
// NEVER do this
let sql = format!("SELECT id FROM concepts WHERE memoir_id = '{}'", memoir_id);
conn.query_row(&sql, params![], ...)
```

Parameterized queries prevent SQL injection and make query intent clear.

## JSON Predicates and NULL Handling

SQLite's `json_extract()` function returns NULL in two distinct cases:
1. A key is missing from the JSON object
2. A key is explicitly set to `null`

This is a footgun when building composite predicates (especially in GROUP BY or concatenation contexts). For example:

```rust
// DANGEROUS: if j.value has no '$.namespace' or explicit null, the || concatenation fails
"SELECT json_extract(j.value, '$.namespace') || ':' || json_extract(j.value, '$.value')"
```

If either extracted value is NULL, the concatenation produces NULL, causing type mismatches when the result is cast to `String` in Rust.

## The Fix: `json_type()` Guards

Use `json_type(col, '$.path') = 'text'` to ensure a key exists and contains a string (not missing, not null, and not another type):

```rust
// SAFE: only process rows where both keys exist and are string-typed
"SELECT json_extract(j.value, '$.namespace') || ':' || json_extract(j.value, '$.value')
 FROM concepts, json_each(concepts.labels) AS j
 WHERE json_type(j.value, '$.namespace') = 'text'
   AND json_type(j.value, '$.value') = 'text'
 GROUP BY 1"
```

The `json_type()` function returns a string describing the type (e.g., `"text"`, `"true"`, `"false"`, or the string `"null"`) when the path exists, or SQL NULL if the path does not exist. The `= 'text'` form is preferred over `IS NOT NULL` when the extracted value is always meant to be a string: it excludes missing keys, explicit JSON nulls (which `json_type` returns as the string `"null"`, not SQL NULL), and non-string types in a single robust check.

## Real Example: `memoir_stats`

The `memoir_stats` function aggregates label counts by concatenating namespace and value fields. When labels are written through the normal path (`Label { namespace: String, value: String }`), both keys are always present. However, malformed or legacy rows—inserted directly into the database—may have missing or explicitly-null keys.

Without the `json_type()` guards, a single malformed label row would cause the entire query to fail with `InvalidColumnType` at the Rust conversion layer.

**Before fix:**
```sql
SELECT json_extract(j.value, '$.namespace') || ':' || json_extract(j.value, '$.value'), COUNT(*)
FROM concepts, json_each(concepts.labels) AS j
WHERE memoir_id = ?1
GROUP BY 1
```

**After fix:**
```sql
SELECT json_extract(j.value, '$.namespace') || ':' || json_extract(j.value, '$.value'), COUNT(*)
FROM concepts, json_each(concepts.labels) AS j
WHERE memoir_id = ?1
  AND json_type(j.value, '$.namespace') = 'text'
  AND json_type(j.value, '$.value') = 'text'
GROUP BY 1
```

Malformed rows—including those with missing keys or explicit JSON nulls—are now excluded from the grouping, allowing the query to succeed.

## Defense in Depth

Apply `json_type()` guards:
- Whenever JSON predicates are used in WHERE clauses
- Before concatenating extracted values
- When grouping or aggregating on JSON fields
- Even when equality comparisons nominally exclude NULL (an explicit guard is clearer and catches the mistake early)

This pattern is idiomatic in SQLite and costs negligible performance while preventing silent data corruption or runtime failures.
