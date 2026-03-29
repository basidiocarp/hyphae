# Embedding Models for Code Search

Hyphae uses embeddings to power semantic search across stored memories and ingested code. By default it uses BAAI/bge-small-en-v1.5, a general-purpose model optimized for speed. For code-heavy workloads, specialized embedders produce much better recall.

## The Problem with General Models

General-purpose embeddings treat different representations of the same concept as unrelated. The queries "fetch user by id", "get_user_id", and "getUserById" likely produce distinct vectors in bge-small, forcing hybrid search to rely heavily on full-text matching. This matters for code because identifier names matter, but the semantic intent is identical.

Code-specific embedders understand that `getUserById(123)` and "get the user with id 123" solve the same problem. Their vector spaces preserve code semantics: similar algorithms stay close, different algorithms stay far. This reduces hallucination and improves recall in both memory retrieval and ingest search.

## Current Setup

Hyphae has two embedder backends:

| Backend | Config | Transport | Model |
|---------|--------|-----------|-------|
| FastEmbed (default) | `embeddings.model` in config.toml | Local binary | BAAI/bge-small-en-v1.5 (384d) |
| HTTP | Environment vars | Network (Ollama, OpenAI-compatible) | Your choice |

The FastEmbed backend runs locally without external dependencies — no server to start, no API keys. It ships with support for 13 models across 3 dimension classes (384, 768, 1024d). The HTTP backend supports any OpenAI-compatible API or Ollama instance, letting you swap in specialized models with a single config change.

## Recommended Models

### Option 1: nomic-embed-code (Recommended for most teams)

768 dimensions, 8192 token context, MIT license. Built specifically for code and designed to run on consumer hardware via Ollama.

```toml
[embeddings]
model = "JinaEmbeddingsV2BaseCode"  # 768d variant available in fastembed
```

Or via Ollama (recommended if Ollama is already running):

```bash
ollama pull nomic-embed-code
export HYPHAE_EMBEDDING_URL=http://localhost:11434
export HYPHAE_EMBEDDING_MODEL=nomic-embed-code
```

Trade-offs: Excellent code semantics (82% on CodeSearchNet), slower than bge-small (65ms vs 45ms per 512-token chunk), 150MB download. Sweet spot between quality and resource cost.

### Option 2: voyage-code-3 (Best in class, API only)

1024 dimensions, 16000 token context. Proprietary, ranked #1 on MTEB code benchmarks. Requires API key and network roundtrips.

```bash
export HYPHAE_EMBEDDING_URL=https://api.voyageai.com/v1
export HYPHAE_EMBEDDING_MODEL=voyage-code-3
export VOYAGE_API_KEY=<key>
```

Trade-offs: Highest quality recall, costs ~$0.02 per 1M tokens, network latency (200-400ms roundtrips). Use when code search quality is critical or embedding batch size justifies API cost.

### Option 3: Salesforce/SFR-Embedding-Code (Highest dims)

4096 dimensions, 4096 token context. Large model, excellent semantic precision but high storage overhead (16MB per 1000 embeddings vs 3MB for 768d).

```bash
ollama pull sfr-embedding-code
export HYPHAE_EMBEDDING_URL=http://localhost:11434
export HYPHAE_EMBEDDING_MODEL=sfr-embedding-code
```

Trade-offs: Best semantic accuracy on very long code snippets, ~800MB download, 3x storage cost, slower than nomic (120ms per chunk). Use when ingesting large codebases with complex module relationships.

### Option 4: jina-embeddings-v3 (Multilingual + code)

1024 dimensions, 8192 token context. Handles both English docs and code equally well, supports 95+ languages.

Available via Ollama or HTTP API:

```bash
ollama pull jina-embeddings-v3
export HYPHAE_EMBEDDING_URL=http://localhost:11434
export HYPHAE_EMBEDDING_MODEL=jina-embeddings-v3
```

Trade-offs: Good for mixed repos (comments in multiple languages, code universal), slower than nomic, large download. Use when repos have non-English documentation or international contributors.

## How to Switch

FastEmbed local models change via config file:

```toml
[embeddings]
model = "NomicEmbedTextV1"  # No code-specific variant in fastembed yet
```

HTTP endpoints change via environment:

```bash
# Ollama (runs locally on port 11434 by default)
export HYPHAE_EMBEDDING_URL=http://localhost:11434
export HYPHAE_EMBEDDING_MODEL=nomic-embed-code

# OpenAI-compatible API (Voyage, Together, etc.)
export HYPHAE_EMBEDDING_URL=https://api.openai.com/v1
export HYPHAE_EMBEDDING_MODEL=text-embedding-3-large
```

Re-ingest after switching to re-embed stored memories and documents with the new model. Old embeddings become invalid.

## Switching Models: Re-indexing

Embeddings are model-specific. Switching models requires re-ingesting to rebuild vector indices:

```bash
# 1. Backup database
cp ~/.local/share/hyphae/hyphae.db ~/.local/share/hyphae/hyphae.db.backup

# 2. Set new embedding model via environment or config
export HYPHAE_EMBEDDING_URL=http://localhost:11434
export HYPHAE_EMBEDDING_MODEL=nomic-embed-code

# 3. Clear vector data and re-ingest
hyphae doctor          # verify the current embedding setup
hyphae embed-all       # regenerate missing embeddings after reconfiguration

# Or manually (SQL):
# sqlite> DELETE FROM embedding_vectors;
# Then re-run: hyphae ingest /path/to/source
```

## Comparison Table

| Model | Dims | Context | Latency | License | Best For |
|-------|------|---------|---------|---------|----------|
| bge-small-en-v1.5 | 384 | 512 | 45ms | MIT | Speed, disk space |
| nomic-embed-code | 768 | 8192 | 65ms | MIT | Code search, local |
| voyage-code-3 | 1024 | 16000 | 250ms | Proprietary | Quality, cross-file |
| sfr-embedding-code | 4096 | 4096 | 120ms | Apache | Large codebases |
| jina-embeddings-v3 | 1024 | 8192 | 75ms | MIT | Multilingual |

## Trade-offs Summary

Higher dimensions store more information per chunk but consume more disk space and SQL-vec compute. bge-small (384d) indexes 3x faster and uses 1/8 the disk compared to SFR (4096d), but loses semantic precision on subtle API differences.

Longer context windows allow embedding entire functions instead of sliding 512-token chunks, reducing boundary artifacts. Offset this against embedding latency when ingesting large repos.

Local models (FastEmbed, Ollama) have zero external dependencies and predictable latency. API models (Voyage, OpenAI) cost money per 1M tokens but auto-update and eliminate infrastructure burden.

## Practical Recommendation

Start with **nomic-embed-code via Ollama** for most teams:

```bash
# 1. Install Ollama (https://ollama.ai)
# 2. Pull the model
ollama pull nomic-embed-code

# 3. Set environment
export HYPHAE_EMBEDDING_URL=http://localhost:11434
export HYPHAE_EMBEDDING_MODEL=nomic-embed-code

# 4. Ingest your codebase
hyphae ingest /path/to/code
```

This gives 80% of the quality improvement for zero cost, zero external dependencies, and only 150MB disk space. Upgrade to voyage-code-3 if code search accuracy becomes a bottleneck or your team's API budget allows it.

For large teams or production deployments, run Ollama as a shared service and point all agents to the same endpoint. This centralizes embedding cost and allows model swaps without per-agent configuration.
