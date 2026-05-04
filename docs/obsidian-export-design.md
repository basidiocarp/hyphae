# Obsidian Export Design

## Status

Proposed. The design document is the primary deliverable for the initial handoff. Implementation of `hyphae export obsidian` follows as a separate step once the design is ratified. The existing `hyphae export` command produces a structured JSON archive (`HyphaeArchive`); the Obsidian export is a parallel path that produces Markdown files rather than replacing the archive format.

---

## Source Of Truth

Hyphae is the canonical store. The Obsidian vault is a human-readable projection. Key consequences:

- Never write back from Obsidian into Hyphae. There is no import path.
- Re-running the export overwrites vault files for the same content identifiers; this is safe because Hyphae owns the authoritative state.
- Deleting or editing a note in Obsidian has no effect on Hyphae.
- The vault is regenerated on demand, not kept in sync automatically.

---

## Exported Note Types

| Type | Source | Vault folder |
|------|--------|-------------|
| Memory | `memories` table — summary, topic, keywords, importance | `memories/<project>/` |
| Lesson | `lessons` table — pattern, examples, source event | `lessons/` |
| Decision | memories with `topic` matching `decisions/*` | `decisions/<project>/` |
| Memoir index | one note per memoir — name, description, concept list | `memoirs/<memoir_name>/` |
| Memoir concept | one note per concept — definition, links | `memoirs/<memoir_name>/<concept_name>.md` |
| Session summary | `sessions` table — scope, start/end, topic count | `sessions/<year>/<session_id>.md` |
| Audit finding | memories with `topic` matching `audit/*` | `audit/` |

Notes not exported: raw command output chunks, embedding vectors, binary artifacts, raw transcript fragments (unless `--include-raw` is explicitly passed).

---

## Markdown Layout

### Vault folder structure

```
<vault>/
├── memories/
│   └── <project>/
│       └── <memory_id>.md
├── lessons/
│   └── <lesson_id>.md
├── decisions/
│   └── <project>/
│       └── <memory_id>.md
├── memoirs/
│   └── <memoir_name>/
│       ├── _index.md        # memoir overview and concept list
│       └── <concept_name>.md
├── sessions/
│   └── <year>/
│       └── <session_id>.md
└── audit/
    └── <memory_id>.md
```

### Filename rules

- Use the stable Hyphae ID as the filename base where one exists (memory_id, session_id, etc.).
- Sanitize names for filesystem safety: replace `/`, `\`, `:`, `*`, `?`, `"`, `<`, `>`, `|` with `-`.
- Memoir concept notes use the concept name lowercased and hyphenated (e.g. `hyphae-store.md`).
- Never truncate IDs — they must be stable across re-exports for Obsidian link resolution.

### Frontmatter

Every note starts with YAML frontmatter:

```yaml
---
hyphae_id: <id>
type: memory | lesson | decision | memoir | concept | session | audit
project: <project root or "global">
topic: <topic string, if applicable>
importance: critical | high | medium | low | ephemeral
tags: [<keyword1>, <keyword2>, ...]
created_at: <ISO 8601>
updated_at: <ISO 8601>
source: hyphae-export-v1
---
```

For memoir concepts, also include:

```yaml
memoir: <memoir_name>
relations:
  - relation: depends_on
    target: <concept_name>
```

### Note body

- **Memory**: `## Summary\n<summary>\n\n## Keywords\n<comma-separated keywords>`
- **Lesson**: `## Pattern\n<pattern>\n\n## Examples\n<examples>`
- **Memoir index**: `## Description\n<description>\n\n## Concepts\n<bulleted list with links>`
- **Concept**: `## Definition\n<definition>\n\n## Relations\n<table of relation + target>`
- **Session**: `## Scope\n<scope>\n\n## Duration\n<start>–<end>\n\n## Topics\n<topic list>`

Obsidian `[[wikilink]]` format is used for concept→concept references within the same memoir subfolder.

---

## Redaction Rules

The following are never written to the vault unless explicitly overridden:

| Category | Rule |
|----------|------|
| Raw command output | Omit. Stored in hyphae as chunked output; not suitable for vault notes. |
| Raw transcript fragments | Omit unless `--include-raw` flag is passed. |
| Embedding vectors | Never written. Binary float arrays have no meaning in Markdown. |
| API keys and tokens | Strip any value matching common secret patterns (Bearer `[A-Za-z0-9+/]{20,}`, `sk-[A-Za-z0-9]{20,}`, etc.) before writing. |
| PII | Not auto-detected; operators are responsible for not storing PII in memories. A `--redact-pattern` flag can supply additional regex patterns to strip. |
| Worktree paths | Included in frontmatter only; not expanded into note body text. |

Redaction happens at export time in memory — nothing is written to the hyphae store.

---

## Implementation Notes (for follow-on)

The implementation should be a new subcommand `hyphae export obsidian` with this interface:

```bash
hyphae export obsidian <vault-path>
  [--project <project-root>]
  [--topic <topic-prefix>]
  [--since <ISO date>]
  [--include-memoirs]
  [--include-sessions]
  [--include-raw]
  [--redact-pattern <regex>]
  [--dry-run]            # print what would be written, write nothing
  [--overwrite]          # allow overwriting existing vault files
  [--clean]              # remove notes not present in current export set
```

The dry-run mode prints one line per note that would be created or updated (`CREATE <path>`, `UPDATE <path>`, `SKIP <path>`). No files are written.

Implementation lives in `hyphae-cli/src/commands/export_obsidian.rs` alongside the existing `export.rs`. It reads from the same `SqliteStore` and reuses the `ArchiveFilter` infrastructure where possible.

---

## Contract Needs

A `septa/obsidian-export-manifest-v1.schema.json` is not needed for the initial design. The vault output is self-describing via frontmatter. If Cap ever wants to read vault metadata (e.g. to render a "last export" status), define a manifest at that point.

---

## Cap Relationship

Cap is an operator console, not a second-brain product. The relationship is:

- Cap may link to vault notes by path once a vault path is configured (future: `Settings > Vault path`).
- Cap does not own vault generation, vault display, or vault sync.
- The operator trigger for export is a CLI command or a scheduled cron; Cap does not initiate exports.
- If Cap ever previews vault content, it reads Markdown files directly — it does not re-implement the export logic.

---

## Known v1 Gaps

The following items are in scope per the design table but not implemented in the initial CLI release. Each is a follow-on handoff.

| Gap | Reason deferred |
|-----|-----------------|
| `lessons/` export | Requires `list_lessons` query; lesson table access not yet plumbed through `SqliteStore` public API |
| `sessions/<year>/` export | Requires `list_sessions` query; same boundary issue |
| Concept `## Relations` body section | `Concept` struct does not yet carry a `links` field at the CLI layer; relations live in a join table queried separately via `MemoirStore::list_links` |
| Concept `relations:` frontmatter block | Same dependency as above |

---

## Open Questions

1. **Incremental export** — should re-export skip notes whose `updated_at` has not changed? The `--clean` flag handles deletion; incremental writes reduce I/O but require tracking last-export timestamps. Defer to implementation.
2. **Multi-vault** — should one Hyphae instance support exporting to multiple named vaults? Probably not in v1; one vault per export run.
3. **Obsidian plugin** — a future Obsidian community plugin could call `hyphae export obsidian` on a schedule. Out of scope here; the CLI is the integration surface.
4. **Concept links across memoirs** — wikilinks only work within the same vault folder. Cross-memoir links require absolute paths (`[[memoirs/other-memoir/concept.md]]`). Decide at implementation time whether to use relative wikilinks or absolute paths.
