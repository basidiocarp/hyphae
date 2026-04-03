# Hyphae

Persistent memory for AI coding agents. One binary, zero dependencies, MCP-native.

## The problem

AI coding agents forget everything between sessions. Context window compaction wipes hours of accumulated understanding. The same architecture decisions get re-discussed, the same bugs get re-debugged, the same files get re-read.

Existing solutions either cost too much (Mem0 burns 2 LLM calls per message), lock you into one tool (MemGPT), or don't scale (CLAUDE.md files, context stuffing).

## What Hyphae does

Hyphae stores what your agent learns and gives it back when needed. Two memory models handle different kinds of knowledge.

Episodic memories capture events: decisions, errors, configurations, preferences. They decay over time unless accessed or marked important. A critical architecture decision stays forever; a one-time debug note fades in weeks.

Semantic memoirs capture structure: architecture graphs, concept relationships, domain models. Concepts are refined, never decayed. Link your API gateway to its database dependency and that relationship persists across every session.

Rule-based extraction detects architecture decisions, error resolutions, and configuration changes from conversation text—no API calls, no token cost, no latency.

Universal tool support covers 14 editors. `hyphae init` writes the MCP config for Claude Code, Cursor, VS Code, Windsurf, Zed, Amp, Amazon Q, Cline, Roo Code, Kilo Code, Codex CLI, OpenCode, Claude Desktop, and Gemini. Switch tools without losing memory.

## Use cases

### Session continuity

Friday's agent stores "chose PostgreSQL for JSONB" and "fixed connection pool exhaustion with PgBouncer." Monday's agent recalls both without re-reading any file.

Tested over 10 sessions on a real Rust project: factual recall jumped from 5% to 68%, context tokens dropped 44%, and agent turns dropped 29%.

### Shared knowledge

A memoir named `backend-arch` maps the service graph: user-service depends on postgres and redis, api-gateway depends on user-service and auth-middleware. A new team member's agent gets full context from day one.

### Multi-tool workflows

Backend work in Claude Code, frontend in Cursor, scripts in Codex. All three connect to the same Hyphae instance. Decisions made in one tool are recalled in another.

### Local models

Hyphae works without tool use via context injection:

```bash
context=$(hyphae recall-context "my-project")
ollama run qwen2.5:14b "$context\n\nQuestion: How does auth work?"
```

Recall improvement: +93% with qwen2.5:14b, +89% with mistral:7b.

## Performance

| Operation | Latency |
|-----------|---------|
| Store (no embedding) | 34 us |
| Store (with embedding) | 52 us |
| FTS search | 47 us |
| Vector search (KNN) | 590 us |
| Hybrid search | 951 us |
| Batch decay (1000 memories) | 5.8 ms |

Apple M1 Pro, in-memory SQLite, single-threaded.

### Agent efficiency

Real API calls on a 12-file Rust project (~550 lines):

| Metric | Without Hyphae | With Hyphae | Delta |
|--------|---------------|------------|-------|
| Factual recall (session 2+) | 5% | 68% | +63% |
| Context tokens (session 3) | 75k | 42k | -44% |
| Agent turns (session 2) | 5.7 | 4.0 | -29% |
| Cost per session | $0.030 | $0.025 | -17% |

### Local models (ollama)

Context injection, no tool use:

| Model | Without | With | Delta |
|-------|---------|------|-------|
| qwen2.5:14b | 4% | 97% | +93% |
| mistral:7b | 4% | 93% | +89% |
| llama3.1:8b | 4% | 93% | +89% |
| qwen2.5:7b | 4% | 90% | +86% |
| phi4:14b | 6% | 79% | +73% |
| llama3.2:3b | 0% | 76% | +76% |

All benchmarks use real API calls with fresh tempdirs and databases per run.

## Architecture

```
Single binary (hyphae)
  Storage: SQLite + FTS5 + sqlite-vec (cosine)
  Search: 30% BM25 + 70% cosine similarity
  Embeddings: fastembed, bge-small-en-v1.5 (384d)
  Protocol: MCP JSON-RPC 2.0 over stdio
  Extraction: rule-based pattern matching (zero LLM cost)
  Tests: 317 tests (unit, security, performance, UX, integration)
```

Local SQLite only—no cloud, no API key for storage, no Docker.

## Security

All SQL queries use parameterized statements. FTS queries are sanitized against injection. The embedding model runs locally. Tested against SQL injection, FTS injection, XSS, null bytes, and 500KB payloads.

## Getting started

```bash
brew tap basidiocarp/tap && brew install hyphae
hyphae init
```

Two commands configure permanent memory for your agent.

## Resources

- GitHub: [https://github.com/basidiocarp/hyphae](https://github.com/basidiocarp/hyphae)
- [Technical Architecture](ARCHITECTURE.md)
- [User Guide](GUIDE.md)
