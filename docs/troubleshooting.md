# Hyphae Troubleshooting

Start with `hyphae init` and restart your client. Most new-install failures come from missing MCP registration, missing Claude hook setup, or a stale client process that never reloaded the new config.

## Fast Triage

| Symptom | First Command | What it usually means |
|---------|---------------|-----------------------|
| Agent never recalls or stores anything | `hyphae init` | MCP registration or Claude hook setup is missing |
| `hyphae recall` returns nothing | `hyphae stats` | The store is empty, the query is too narrow, or embeddings are missing |
| Responses stay long after compact mode was enabled | `hyphae config` | Compact mode is off or the client has not restarted |
| Embedding-related commands fail | `hyphae doctor` | Embeddings are disabled, not configured, or still downloading |
| SQLite errors show up during recall or stats | `hyphae doctor` | The database path is wrong, locked, or damaged |

## Setup and Registration Issues

### Agent does not use Hyphae tools

**Symptom:** The agent never recalls, stores, or mentions Hyphae even when asked.

**Diagnosis:** Hyphae is not registered as an MCP server, the Claude hook mode was never installed, or the client never reloaded the updated config.

**Fix:**

1. Run the setup flow again and read the per-tool output:
   ```bash
   hyphae init
   ```
   Healthy output names the tool it configured and the config file it changed.

2. If you want Claude lifecycle capture, install hook mode too:
   ```bash
   hyphae init --mode hook
   ```
   For Claude Code, `~/.claude/settings.json` should end up with `PostToolUse`, `PreCompact`, and `SessionEnd` entries.

3. Verify the server can answer a basic MCP initialize request, then restart your client:
   ```bash
   echo '{"jsonrpc":"2.0","id":1,"method":"initialize"}' | hyphae serve
   ```
   A healthy response includes JSON with `capabilities` and `serverInfo`.

### `hyphae init` does not detect my tool

**Symptom:** The tool you expected to configure does not appear in `hyphae init` output.

**Diagnosis:** Hyphae only auto-configures tools it can detect locally. Missing config files or unsupported clients will not show up in the automatic pass.

**Fix:**

1. Confirm the client is installed and has created its config file:
   ```bash
   hyphae init
   ```
   If the tool still does not appear, Hyphae did not detect a supported config target.

2. For Claude Code, make sure the client has been launched at least once:
   ```bash
   ls ~/.claude.json
   ```
   If that file is missing, Claude Code has not created its MCP config yet.

3. Add Hyphae manually if needed:
   ```bash
   claude mcp add hyphae -- hyphae serve
   ```
   For other tools, add the same command and args in their MCP config format.

### Compact mode does not activate

**Symptom:** MCP responses stay long even though you enabled compact mode.

**Diagnosis:** Compact mode is off in config, the MCP command does not include `--compact`, or the client is still using an old process.

**Fix:**

1. Inspect the loaded configuration:
   ```bash
   hyphae config
   ```
   Look for `[mcp] compact = true`.

2. Force compact mode at process start if you want to rule out config loading:
   ```bash
   hyphae serve --compact
   ```

3. Restart the client after any config or MCP command change.

## Recall and Memory Issues

### `hyphae recall` returns nothing

**Symptom:** Search returns `No memories found` or only empty-looking results.

**Diagnosis:** The store may be empty, the query may be too narrow, or the memories exist but have not been embedded yet.

**Fix:**

1. Check whether the store has any data at all:
   ```bash
   hyphae stats
   hyphae topics
   ```
   If both are empty, there is nothing to recall yet.

2. List stored items without the original query filters:
   ```bash
   hyphae list --all
   ```
   If results appear here but not in recall, the original query or filters were too narrow.

3. Regenerate embeddings if the text exists but similarity search is weak:
   ```bash
   hyphae embed
   ```

### Duplicate memories appear in one topic

**Symptom:** You see multiple near-identical memories under the same topic.

**Diagnosis:** CLI storage does not apply the same automatic dedup path as the MCP server with an active embedder, or embeddings were never backfilled after earlier writes.

**Fix:**

1. Backfill embeddings first:
   ```bash
   hyphae embed
   ```

2. Delete obvious duplicates:
   ```bash
   hyphae forget <id>
   ```

3. Consolidate the topic when the duplication is broader than one or two entries:
   ```bash
   hyphae consolidate --topic <topic>
   ```

### `hyphae memoir show` says the memoir is missing right after creation

**Symptom:** `hyphae memoir show <name>` fails right after `hyphae memoir create`.

**Diagnosis:** The name does not match exactly, or the create and show commands are pointing at different databases.

**Fix:**

1. Confirm the memoir name exactly as stored:
   ```bash
   hyphae memoir list
   ```

2. Retry with the exact case-sensitive name:
   ```bash
   hyphae memoir show <exact-name>
   ```

3. If the name is correct, check whether you are switching databases with config or `--db`:
   ```bash
   hyphae config
   ```

### `hyphae extract` says no facts were extracted

**Symptom:** `hyphae extract` returns `No facts extracted`.

**Diagnosis:** The text does not contain strong enough decision, architecture, or error signals for the current extraction threshold.

**Fix:**

1. Test the extractor on a sentence that should clearly match:
   ```bash
   echo "We decided to use PostgreSQL instead of MySQL" | hyphae extract --dry-run
   ```
   If that works, the extractor is fine and the original text was just too weak.

2. Lower the extraction threshold if you want more aggressive matching:
   ```toml
   [extraction]
   min_score = 2.0
   ```

3. Retry the original input after saving the config:
   ```bash
   hyphae extract --dry-run
   ```

## Embeddings, Database, and Performance

### Embeddings are slow on first launch

**Symptom:** The first `store`, `recall`, or `embed` takes much longer than later runs.

**Diagnosis:** Hyphae is downloading and warming the embedding model on first use.

This is expected behavior. The first run can take tens of seconds; later runs usually load from cache much faster.

```bash
hyphae doctor
```

### `embeddings feature not enabled`

**Symptom:** `hyphae embed` fails with a message about embeddings not being enabled.

**Diagnosis:** The current binary was built without the default embeddings feature.

**Fix:**

1. Rebuild with default features:
   ```bash
   cargo build --release
   ```

2. If you changed embedding settings and want a clean rebuild of vectors, re-embed:
   ```bash
   hyphae embed --force
   ```

3. If you are using a release binary, replace the custom build with the normal shipped binary.

### SQLite errors appear during recall or stats

**Symptom:** `hyphae stats`, `hyphae recall`, or related commands fail with a SQLite error.

**Diagnosis:** The configured database path is wrong, the file is damaged, or the current process is using a bad local state.

**Fix:**

1. Check the resolved configuration and database path:
   ```bash
   hyphae config
   ```

2. Back up the database before changing anything:
   ```bash
   cp ~/Library/Application\ Support/dev.hyphae.hyphae/memories.db ~/hyphae-backup.db
   ```
   On Linux, use `~/.local/share/dev.hyphae.hyphae/memories.db` instead.

3. Test with a clean temporary database to separate corruption from config issues:
   ```bash
   hyphae --db /tmp/hyphae-test.db stats
   ```
   If the temporary database works, the original database or its location is the problem.

### Recall gets slow with many memories

**Symptom:** Recall feels noticeably slower once the store is large.

**Diagnosis:** The store has grown, large topics are fragmented, or the command is returning more results than you actually need.

**Fix:**

1. Check overall size first:
   ```bash
   hyphae stats
   ```

2. Consolidate topics that have become cluttered:
   ```bash
   hyphae consolidate --topic <topic>
   ```

3. Prune stale memories or lower result counts on recall:
   ```bash
   hyphae prune
   hyphae recall "<query>" --limit 10
   ```

## Configuration and Retention

### Decay is too aggressive or not aggressive enough

**Symptom:** Memories disappear sooner than you expect, or they never seem to decay at all.

**Diagnosis:** The current decay settings do not match how often you want to prune and protect stored memory.

**Fix:**

1. Inspect the active configuration:
   ```bash
   hyphae config
   ```

2. Adjust the memory settings in `~/.config/hyphae/config.toml`:
   ```toml
   [memory]
   decay_rate = 0.98
   prune_threshold = 0.05
   ```

3. Preview pruning before deleting anything:
   ```bash
   hyphae prune --threshold 0.2 --dry-run
   ```

## Error Message Quick Reference

| Error | Cause | Fix |
|-------|-------|-----|
| `"No memories found"` | The store is empty, the query is too narrow, or embeddings are missing | Run `hyphae stats`, `hyphae list --all`, then `hyphae embed` if needed |
| `"embeddings feature not enabled"` | The current binary was built without embeddings | Rebuild with `cargo build --release` |
| `"memoir not found"` | The memoir name does not match or you are using a different database | Check `hyphae memoir list` and `hyphae config` |
| `"No facts extracted"` | The text did not cross the extraction threshold | Retry with `hyphae extract --dry-run` and lower `min_score` if needed |
| `SQLite error` | The database path is wrong, locked, or damaged | Check `hyphae config`, back up the DB, then test with `--db /tmp/hyphae-test.db` |

## Diagnostic Commands

**Enable debug logging:**
```bash
# All Hyphae debug output
HYPHAE_LOG=debug hyphae doctor

# Debug the MCP server directly
HYPHAE_LOG=debug hyphae serve
```

**Check version:**
```bash
hyphae --version
```

**Inspect current configuration:**
```bash
hyphae config
cat ~/.config/hyphae/config.toml
```

**Check state and health:**
```bash
hyphae doctor
hyphae stats
hyphae topics
```

## See also

- [guide.md](guide.md)
- [setup-by-tool.md](setup-by-tool.md)
- [cli-reference.md](cli-reference.md)
