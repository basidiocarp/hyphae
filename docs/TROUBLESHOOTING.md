# Hyphae Troubleshooting & FAQ

Common problems and their fixes, plus answers to frequently asked questions. If you've just installed Hyphae and something isn't working, start with #1—most issues trace back to the MCP config or a missing restart.

---

## Troubleshooting

### 1. The agent doesn't use Hyphae tools

**Symptom:** The agent neither recalls nor stores anything, even when asked.

**Solutions:**
- Run `hyphae init` and check the output for each tool
- Verify that the MCP config file exists (e.g., `~/.claude.json` for Claude Code)
- If you want Claude Code lifecycle capture, run `hyphae init --mode hook` and verify
  `~/.claude/settings.json` contains `PostToolUse`, `PreCompact`, and `SessionEnd` entries
- Test the server manually:
  ```bash
  echo '{"jsonrpc":"2.0","id":1,"method":"initialize"}' | hyphae serve
  ```
  You should see a JSON response with `capabilities` and `serverInfo`
- Verify that `hyphae serve` is in your PATH: `which hyphae`
- **Restart your AI tool** after running `hyphae init`

### 2. `hyphae recall` returns nothing

**Symptom:** Search returns "No memories found."

**Solutions:**
- `hyphae topics` — verify there are stored memories
- `hyphae stats` — check the total
- Try a broader query or remove topic/keyword filters
- `hyphae list --all` — list everything to verify content
- If memories exist but don't match: backfill embeddings with `hyphae embed`

### 3. Embeddings are slow on first launch

**Symptom:** Hyphae takes 30+ seconds on the first `store` or `recall`.

The embedding model (~45MB for bge-small-en-v1.5) is downloaded on first use. Subsequent runs load from cache in 1-2 seconds.

**Solutions:**
- This is normal the first time — wait for the download
- To speed things up: use a lighter model in `config.toml`:
  ```toml
  [embeddings]
  model = "BAAI/bge-small-en-v1.5"  # 384d, English only, fastest
  ```
- To compile without embeddings: `cargo build --no-default-features`

### 4. Duplicate memories appear

**Symptom:** Multiple near-identical memories in the same topic.

Auto-dedup only works via MCP (server with embedder). The `hyphae store` CLI does not have auto-dedup by default.

**Solutions:**
- Backfill embeddings: `hyphae embed`
- Delete duplicates manually: `hyphae forget <id>`
- Consolidate the topic: `hyphae consolidate -t <topic>`

### 5. Error "embeddings feature not enabled"

**Symptom:** `hyphae embed` fails with a message about the feature.

**Solution:** Recompile with the embeddings feature:
```bash
cargo build --release  # The "embeddings" feature is active by default
```

If you are using the pre-compiled binary from GitHub releases, embeddings are always included.

### 6. Database corruption

**Symptom:** `hyphae stats` or `hyphae recall` fails with a SQLite error.

**Solutions:**
- Locate the database:
  - macOS: `~/Library/Application Support/dev.hyphae.hyphae/memories.db`
  - Linux: `~/.local/share/dev.hyphae.hyphae/memories.db`
- Back up the `.db` file and its WAL files (`.db-wal`, `.db-shm`)
- Delete and rebuild if necessary — migration is automatic
- To test with a clean database: `hyphae --db /tmp/test.db stats`

### 7. `hyphae init` doesn't detect my tool

**Symptom:** The tool doesn't appear in `hyphae init` output.

**Solutions:**
- Verify that the tool is installed and its config file exists
- For Claude Code: `~/.claude.json` must exist (created on first launch)
- Manual configuration: `claude mcp add hyphae -- hyphae serve`
- For unsupported tools, add manually in their MCP config:
  ```json
  { "command": "/path/to/hyphae", "args": ["serve"] }
  ```

### 8. Decay is too aggressive / not aggressive enough

**Symptom:** Memories disappear too quickly, or accumulate without being cleaned up.

**Solutions:**
- Adjust in `~/.config/hyphae/config.toml`:
  ```toml
  [memory]
  decay_rate = 0.98      # Slower (default: 0.95)
  prune_threshold = 0.05 # Lower threshold (default: 0.1)
  ```
- Use `hyphae prune --dry-run --threshold 0.2` to preview
- Mark important memories as `high` or `critical` to protect them

### 9. Compact mode doesn't activate

**Symptom:** MCP responses remain long despite the configuration.

**Solutions:**
- Check `config.toml`:
  ```toml
  [mcp]
  compact = true
  ```
- Or force via flag: change `hyphae serve` to `hyphae serve --compact` in the MCP config
- Restart your AI tool after the change

### 10. Error "memoir not found" despite creating it

**Symptom:** `hyphae memoir show <name>` fails right after `hyphae memoir create`.

**Solutions:**
- Check the exact name: `hyphae memoir list`
- Names are case-sensitive: `Arch` != `arch`
- Verify that you're not using `--db` with a different path

### 11. Degraded performance with many memories

**Symptom:** `recall` becomes slow with >1000 memories.

**Solutions:**
- Hybrid search takes ~1ms per query for 1000 memories — that's normal
- Consolidate large topics: `hyphae consolidate -t <topic>`
- Prune stale memories: `hyphae prune`
- Reduce `limit` in searches

### 12. Extraction detects nothing

**Symptom:** `hyphae extract` returns "No facts extracted."

**Solutions:**
- The text must contain recognized signals (architecture keywords, errors, decisions)
- Test with explicit text:
  ```bash
  echo "We decided to use PostgreSQL instead of MySQL" | hyphae extract --dry-run
  ```
- Adjust the threshold in `config.toml`:
  ```toml
  [extraction]
  min_score = 2.0  # Lower = more facts extracted (default: 3.0)
  ```

---

## FAQ

### Q1: Does Hyphae send data over the internet?

**No.** Hyphae stores everything locally in a SQLite file. The embedding model runs locally (via fastembed/ONNX Runtime). No data leaves your machine. The only network access is the initial download of the embedding model (~100MB, one time only).

### Q2: Can I use Hyphae with multiple projects?

**Yes.** All projects share the same SQLite database. Use project-prefixed topics (e.g., `decisions-api`, `decisions-frontend`) to separate them. You can also use `--db <path>` to completely isolate databases.

### Q3: How do I backup/restore my memory?

Back up the SQLite file:
```bash
# macOS
cp ~/Library/Application\ Support/dev.hyphae.hyphae/memories.db ~/backup-hyphae.db

# Restore
cp ~/backup-hyphae.db ~/Library/Application\ Support/dev.hyphae.hyphae/memories.db
```

### Q4: Does Hyphae work with local models (ollama)?

**Yes.** Hyphae is a standard MCP server. It works with any MCP client, including those using local models. Benchmarks show up to +93% recall with qwen2.5:14b via ollama.

### Q5: What's the difference between `hyphae consolidate` (CLI) and `hyphae_memory_consolidate` (MCP)?

The CLI automatically merges summaries by concatenating with ` | `. The MCP asks the agent to provide the summary, which produces a smarter result because the agent understands the content and can synthesize it.

### Q6: Can I change the embedding model without losing my data?

**Yes.** Memories (text) are always preserved. Only the vectors are cleared and recreated. After changing the model in `config.toml`, run `hyphae embed --force` to regenerate all vectors.

### Q7: How many memories can Hyphae handle?

The SQLite database handles millions of rows without issue. Benchmarks show ~34us per store and ~951us per hybrid search for 1000 memories. Performance degrades linearly, not exponentially.

### Q8: How do I delete all my memory?

```bash
# macOS
rm ~/Library/Application\ Support/dev.hyphae.hyphae/memories.db*

# Linux
rm ~/.local/share/dev.hyphae.hyphae/memories.db*
```

The database is automatically recreated on next launch.

### Q9: Does Hyphae consume LLM tokens?

No, not for storage and recall. Hyphae calls no LLM API. The only tokens consumed are those of the agent calling the MCP tools—exactly like any other MCP tool. Compact mode (`--compact`) reduces these tokens by ~40%.

Extraction (Layer 0) is purely rule-based—zero LLM cost. Layer 1 (PreCompact, planned) will use ~500 tokens per session.

### Q10: Can I share my memory with my team?

Not directly (it's a local SQLite file). However, **memoirs** are designed to capture structured knowledge that can be exported and shared. An import/export feature is planned.

### Q11: Is auto-dedup reliable?

Auto-dedup uses hybrid similarity (BM25 + cosine) with an 85% threshold. It catches close duplicates reliably but lets through very different reformulations of the same fact. This is intentional: a duplicate is better than data loss.

### Q12: How does the MCP server "store nudge" work?

The server counts consecutive tool calls without `hyphae_memory_store`. After 10 calls, it adds a hint to the response:
```
[Hyphae: 12 tool calls since last store. Consider saving important context.]
```
The counter resets on each `hyphae_memory_store`. It's a subtle reminder so the agent doesn't forget to store.

---

## See also

- [GUIDE.md](GUIDE.md) — Core guide: memory models, MCP tools reference, benchmarking
- [SETUP-BY-TOOL.md](SETUP-BY-TOOL.md) — Per-tool MCP configuration for Claude Code, Cursor, VS Code, and more
