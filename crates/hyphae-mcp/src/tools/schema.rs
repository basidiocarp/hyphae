use serde_json::{Value, json};

/// Build the list of tool definitions. When `has_embedder` is false the
/// `hyphae_memory_embed_all` tool is omitted.
pub(super) fn tool_definitions_json(has_embedder: bool) -> Vec<Value> {
    let mut tools = vec![
        // Constitution policy tool
        json!({
            "name": "hyphae_constitution_store",
            "title": "Store Constitution",
            "description": "Store a permanent governance policy that never decays and is excluded from consolidation. Use for rules that must persist indefinitely across all sessions, such as security policies or architectural constraints.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "content": {
                        "type": "string",
                        "maxLength": 32768,
                        "description": "The governance policy text to store permanently"
                    },
                    "topic": {
                        "type": "string",
                        "description": "Category for the policy. Defaults to 'constitution/<project>' when omitted."
                    },
                    "keywords": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Keywords to improve search"
                    },
                    "raw_excerpt": {
                        "type": "string",
                        "maxLength": 65536,
                        "description": "Optional verbatim excerpt (e.g. exact rule text)"
                    }
                },
                "required": ["content"]
            },
            "annotations": {
                "readOnlyHint": false,
                "destructiveHint": false,
                "idempotentHint": false
            }
        }),
        // Memory tools
        json!({
            "name": "hyphae_memory_store",
            "title": "Store Memory",
            "description": "Store important information in Hyphae long-term memory. Use to save decisions, preferences, project context, resolved errors — anything that should persist between sessions.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "topic": {
                        "type": "string",
                        "description": "Category/namespace (e.g. 'project', 'preferences', 'decisions-architecture', 'resolved-errors')"
                    },
                    "content": {
                        "type": "string",
                        "maxLength": 32768,
                        "description": "Information to memorize; be concise but complete"
                    },
                    "importance": {
                        "type": "string",
                        "enum": ["critical", "high", "medium", "low"],
                        "default": "medium",
                        "description": "critical=never auto-pruned (explicit forget/invalidate only), high=never auto-pruned (explicit forget/invalidate only), medium=normal decay, low=fast decay"
                    },
                    "keywords": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Keywords to improve search"
                    },
                    "raw_excerpt": {
                        "type": "string",
                        "maxLength": 65536,
                        "description": "Optional verbatim (code, exact error message, etc.)"
                    },
                    "branch": {
                        "type": "string",
                        "description": "Optional git branch for the memory. If omitted, Hyphae will try to detect it from the current working tree."
                    },
                    "worktree": {
                        "type": "string",
                        "description": "Optional git worktree root for the memory. If omitted, Hyphae will try to detect it from the current working tree."
                    }
                },
                "required": ["topic", "content"]
            },
            "annotations": {
                "readOnlyHint": false,
                "destructiveHint": false,
                "idempotentHint": false
            }
        }),
        json!({
            "name": "hyphae_memory_recall",
            "title": "Recall Memories",
            "description": "Search Hyphae long-term memory with context-aware recall. Use to find past decisions, project context, preferences, or solutions to previously encountered problems. Session-shaped queries boost session memories first, code-shaped queries can expand through code memoirs, and project-scoped recall merges the globally visible '_shared' knowledge pool after those context-specific hits.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Natural language search query"
                    },
                    "topic": {
                        "type": "string",
                        "description": "Filter by specific topic (optional)"
                    },
                    "limit": {
                        "type": "integer",
                        "default": 5,
                        "minimum": 1,
                        "maximum": 20,
                        "description": "Max number of results"
                    },
                    "keyword": {
                        "type": "string",
                        "description": "Filter results by keyword (exact match on memory keywords)"
                    },
                    "offset": {
                        "type": "integer",
                        "default": 0,
                        "minimum": 0,
                        "description": "Number of results to skip (for pagination)"
                    },
                    "session_id": {
                        "type": "string",
                        "description": "Optional explicit session ID from hyphae_session_start. Prefer this when multiple scoped sessions may be active for one project."
                    },
                    "project_root": {
                        "type": "string",
                        "description": "Optional repository root for identity v1 lookups. Use with worktree_id to scope memory recall to the active worktree. When supplied, worktree_id must also be provided (and vice versa)."
                    },
                    "worktree_id": {
                        "type": "string",
                        "description": "Optional worktree identifier for identity v1 lookups. Use with project_root to scope memory recall to the active worktree. When supplied, project_root must also be provided (and vice versa)."
                    },
                    "code_context": {
                        "type": "boolean",
                        "default": false,
                        "description": "When true, code-shaped queries can gather matching concepts from the project's code memoir (code:{project}) using extracted code terms before recall results are finalized. Only effective when a project is configured, and the expanded hits are merged ahead of the globally visible '_shared' fallback results."
                    },
                    "search_type": {
                        "type": "string",
                        "enum": ["semantic", "lexical", "fts", "keyword", "graph", "summary", "code", "hybrid"],
                        "description": "Retrieval strategy. One of: semantic (embedding similarity), lexical (FTS keyword), graph (memoir concept traversal), summary (one result per topic), code (keyword biased to code topics), hybrid (FTS + semantic rerank, default). Omitting defaults to hybrid."
                    },
                    "query_context": {
                        "type": "object",
                        "description": "Optional domain-scoped context. If domain_hint matches a known domain, applies domain rules before searching.",
                        "properties": {
                            "domain_hint": {
                                "type": "string",
                                "description": "Identifier of a knowledge domain to check applicability rules"
                            },
                            "known_inputs": {
                                "type": "object",
                                "description": "Known input values to check against domain requirements"
                            },
                            "min_confidence": {
                                "type": "number",
                                "minimum": 0,
                                "maximum": 1,
                                "description": "Minimum confidence threshold for results"
                            }
                        }
                    }
                },
                "required": ["query"]
            },
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true
            }
        }),
        json!({
            "name": "hyphae_memory_forget",
            "title": "Forget Memory",
            "description": "Delete a specific memory by its ID. Use when information is obsolete or incorrect.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "Memory ID to delete"
                    }
                },
                "required": ["id"]
            },
            "annotations": {
                "readOnlyHint": false,
                "destructiveHint": true,
                "idempotentHint": true
            }
        }),
        json!({
            "name": "hyphae_memory_invalidate",
            "title": "Invalidate Memory",
            "description": "Invalidate a specific memory without deleting it. Invalidated memories are hidden from normal recall by default but remain available for review.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "Memory ID to invalidate"
                    },
                    "reason": {
                        "type": "string",
                        "maxLength": 1024,
                        "description": "Optional reason the memory is no longer valid"
                    },
                    "superseded_by": {
                        "type": "string",
                        "description": "Optional replacement memory ID"
                    }
                },
                "required": ["id"]
            },
            "annotations": {
                "readOnlyHint": false,
                "destructiveHint": true,
                "idempotentHint": true
            }
        }),
        json!({
            "name": "hyphae_memory_list_invalidated",
            "title": "List Invalidated Memories",
            "description": "List invalidated memories for review. Use to audit stale or replaced memories that are hidden from normal recall.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "limit": {
                        "type": "integer",
                        "default": 20,
                        "minimum": 1,
                        "maximum": 100,
                        "description": "Max number of invalidated memories to return"
                    },
                    "offset": {
                        "type": "integer",
                        "default": 0,
                        "minimum": 0,
                        "description": "Number of invalidated memories to skip"
                    }
                }
            },
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true
            }
        }),
        json!({
            "name": "hyphae_memory_consolidate",
            "title": "Consolidate Memories",
            "description": "Consolidate all memories of a topic into a single summary. Useful when a topic accumulates too many entries.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "topic": {
                        "type": "string",
                        "description": "Topic to consolidate"
                    },
                    "summary": {
                        "type": "string",
                        "maxLength": 32768,
                        "description": "Consolidated summary to replace all memories in the topic"
                    }
                },
                "required": ["topic", "summary"]
            },
            "annotations": {
                "readOnlyHint": false,
                "destructiveHint": false,
                "idempotentHint": true
            }
        }),
        json!({
            "name": "hyphae_memory_list_topics",
            "title": "List Memory Topics",
            "description": "List all available topics in memory with their counts.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            },
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true
            }
        }),
        json!({
            "name": "hyphae_memory_stats",
            "title": "Memory Stats",
            "description": "Get global Hyphae memory statistics.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            },
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true
            }
        }),
        json!({
            "name": "hyphae_memory_update",
            "title": "Update Memory",
            "description": "Update an existing memory in-place. Use to correct, refresh, or extend a memory without creating a duplicate.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "Memory ID to update"
                    },
                    "content": {
                        "type": "string",
                        "maxLength": 32768,
                        "description": "New content (replaces existing summary)"
                    },
                    "importance": {
                        "type": "string",
                        "enum": ["critical", "high", "medium", "low"],
                        "description": "New importance level (optional, keeps existing if not set)"
                    },
                    "keywords": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "New keywords (optional, keeps existing if not set)"
                    }
                },
                "required": ["id", "content"]
            },
            "annotations": {
                "readOnlyHint": false,
                "destructiveHint": false,
                "idempotentHint": true
            }
        }),
        json!({
            "name": "hyphae_memory_health",
            "title": "Memory Health",
            "description": "Get health stats for all topics: entry count, staleness, consolidation needs. Use to audit memory hygiene.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "topic": {
                        "type": "string",
                        "description": "Check a specific topic (optional — checks all if omitted)"
                    }
                }
            },
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true
            }
        }),
        json!({
            "name": "hyphae_evaluate",
            "title": "Evaluate Memories",
            "description": "Evaluate agent improvement over time by comparing error rates, correction frequency, and resolution rates across time windows. Compares two equal time periods to show trends.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "days": {
                        "type": "integer",
                        "default": 14,
                        "minimum": 2,
                        "maximum": 365,
                        "description": "Total evaluation window in days (splits into two equal halves for comparison)"
                    }
                }
            },
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true
            }
        }),
        // Cross-project tools
        json!({
            "name": "hyphae_recall_global",
            "title": "Recall Global Memories",
            "description": "Search memories across linked projects and shared knowledge. Returns results grouped by project. By default, searches the _shared project, the caller's own project, and any projects linked to it. Use when knowledge may exist in linked projects, or to find cross-cutting patterns. Set unrestricted=true to search all projects without filtering (requires explicit intent).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Natural language search query"
                    },
                    "limit": {
                        "type": "integer",
                        "default": 10,
                        "minimum": 1,
                        "maximum": 100,
                        "description": "Max total results across allowed projects"
                    },
                    "unrestricted": {
                        "type": "boolean",
                        "default": false,
                        "description": "When true, search all projects without filtering. Default false restricts to caller's project, linked projects, and _shared."
                    }
                },
                "required": ["query"]
            },
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true
            }
        }),
        json!({
            "name": "hyphae_promote_to_memoir",
            "title": "Promote to Memoir",
            "description": "Analyze a memory topic for promotion to a structured memoir. Lists memories, suggests concepts from keywords, and provides step-by-step instructions. Use when a topic has accumulated 15+ memories that should be organized into a knowledge graph.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "topic": {
                        "type": "string",
                        "description": "The memory topic to analyze for promotion"
                    }
                },
                "required": ["topic"]
            },
            "annotations": {
                "readOnlyHint": false,
                "destructiveHint": false,
                "idempotentHint": true
            }
        }),
        // Memoir tools
        json!({
            "name": "hyphae_memoir_create",
            "title": "Create Memoir",
            "description": "Create a new memoir — a permanent knowledge container. Memoirs hold concepts that never decay.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Unique human-readable name for the memoir"
                    },
                    "description": {
                        "type": "string",
                        "description": "Description of what this memoir is for"
                    }
                },
                "required": ["name"]
            },
            "annotations": {
                "readOnlyHint": false,
                "destructiveHint": false,
                "idempotentHint": false
            }
        }),
        json!({
            "name": "hyphae_memoir_list",
            "title": "List Memoirs",
            "description": "List all memoirs with their concept counts.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            },
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true
            }
        }),
        json!({
            "name": "hyphae_memoir_show",
            "title": "Show Memoir",
            "description": "Show a memoir's stats, labels, and all its concepts.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Memoir name"
                    }
                },
                "required": ["name"]
            },
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true
            }
        }),
        json!({
            "name": "hyphae_memoir_add_concept",
            "title": "Add Concept to Memoir",
            "description": "Add a permanent concept to a memoir. Concepts are knowledge nodes that get refined, never decayed.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "memoir": {
                        "type": "string",
                        "description": "Memoir name"
                    },
                    "name": {
                        "type": "string",
                        "description": "Concept name (unique within memoir)"
                    },
                    "definition": {
                        "type": "string",
                        "maxLength": 32768,
                        "description": "Dense description of the concept"
                    },
                    "labels": {
                        "type": "string",
                        "description": "Comma-separated labels (namespace:value or plain tag). E.g. 'domain:arch,type:decision'"
                    },
                    "abstract_text": {
                        "type": "string",
                        "maxLength": 150,
                        "description": "Short abstract or summary (≤150 characters)"
                    },
                    "overview_text": {
                        "type": "string",
                        "maxLength": 500,
                        "description": "Overview paragraph providing context (≤500 characters)"
                    },
                    "block_type": {
                        "type": "string",
                        "description": "Role label for this concept: persona, context, project, error, decision, preference, or custom (default)",
                        "enum": ["persona", "context", "project", "error", "decision", "preference", "custom"]
                    }
                },
                "required": ["memoir", "name", "definition"]
            },
            "annotations": {
                "readOnlyHint": false,
                "destructiveHint": false,
                "idempotentHint": false
            }
        }),
        json!({
            "name": "hyphae_memoir_refine",
            "title": "Refine Memoir Concept",
            "description": "Refine an existing concept with a new, improved definition. Bumps revision and boosts confidence.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "memoir": {
                        "type": "string",
                        "description": "Memoir name"
                    },
                    "name": {
                        "type": "string",
                        "description": "Concept name"
                    },
                    "definition": {
                        "type": "string",
                        "maxLength": 32768,
                        "description": "New, refined definition"
                    },
                    "abstract_text": {
                        "type": "string",
                        "maxLength": 150,
                        "description": "Short abstract or summary (≤150 characters)"
                    },
                    "overview_text": {
                        "type": "string",
                        "maxLength": 500,
                        "description": "Overview paragraph providing context (≤500 characters)"
                    }
                },
                "required": ["memoir", "name", "definition"]
            },
            "annotations": {
                "readOnlyHint": false,
                "destructiveHint": false,
                "idempotentHint": true
            }
        }),
        json!({
            "name": "hyphae_memoir_search",
            "title": "Search Memoir",
            "description": "Full-text search concepts within a memoir.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "memoir": {
                        "type": "string",
                        "description": "Memoir name"
                    },
                    "query": {
                        "type": "string",
                        "description": "Search query"
                    },
                    "label": {
                        "type": "string",
                        "description": "Filter by label (e.g. 'domain:tech')"
                    },
                    "block_type": {
                        "type": "string",
                        "description": "Filter by block type: persona, context, project, error, decision, preference, or custom",
                        "enum": ["persona", "context", "project", "error", "decision", "preference", "custom"]
                    },
                    "limit": {
                        "type": "integer",
                        "default": 10,
                        "description": "Max results"
                    }
                },
                "required": ["memoir", "query"]
            },
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true
            }
        }),
        json!({
            "name": "hyphae_memoir_link",
            "title": "Link Memoir Concepts",
            "description": "Create a directed, typed edge between two concepts in the same memoir.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "memoir": {
                        "type": "string",
                        "description": "Memoir name"
                    },
                    "from": {
                        "type": "string",
                        "description": "Source concept name"
                    },
                    "to": {
                        "type": "string",
                        "description": "Target concept name"
                    },
                    "relation": {
                        "type": "string",
                        "enum": ["part_of", "depends_on", "related_to", "contradicts", "refines", "alternative_to", "caused_by", "instance_of", "superseded_by"],
                        "description": "Relation type"
                    }
                },
                "required": ["memoir", "from", "to", "relation"]
            },
            "annotations": {
                "readOnlyHint": false,
                "destructiveHint": false,
                "idempotentHint": true
            }
        }),
        json!({
            "name": "hyphae_memoir_unlink",
            "title": "Unlink Memoir Concepts",
            "description": "Remove a specific directed edge between two concepts in a memoir.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "memoir": {
                        "type": "string",
                        "description": "Memoir name"
                    },
                    "from": {
                        "type": "string",
                        "description": "Source concept name"
                    },
                    "to": {
                        "type": "string",
                        "description": "Target concept name"
                    },
                    "relation": {
                        "type": "string",
                        "enum": ["part_of", "depends_on", "related_to", "contradicts", "refines", "alternative_to", "caused_by", "instance_of", "superseded_by"],
                        "description": "Relation type to remove"
                    }
                },
                "required": ["memoir", "from", "to", "relation"]
            },
            "annotations": {
                "readOnlyHint": false,
                "destructiveHint": true,
                "idempotentHint": true
            }
        }),
        json!({
            "name": "hyphae_memoir_inspect",
            "title": "Inspect Memoir",
            "description": "Inspect a concept and its graph neighborhood using Breadth-First Search (BFS).",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "memoir": {
                        "type": "string",
                        "description": "Memoir name"
                    },
                    "name": {
                        "type": "string",
                        "description": "Concept name"
                    },
                    "depth": {
                        "type": "integer",
                        "default": 1,
                        "description": "BFS depth"
                    }
                },
                "required": ["memoir", "name"]
            },
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true
            }
        }),
        json!({
            "name": "hyphae_memoir_search_all",
            "title": "Search All Memoirs",
            "description": "Full-text search concepts across all memoirs.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query"
                    },
                    "limit": {
                        "type": "integer",
                        "default": 10,
                        "description": "Max results"
                    }
                },
                "required": ["query"]
            },
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true
            }
        }),
        json!({
            "name": "hyphae_memoir_history",
            "title": "Memoir History",
            "description": "View the version history of a memoir. Shows recent changes with author, git hash, and summary.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "memoir": {
                        "type": "string",
                        "description": "Name of the memoir"
                    },
                    "limit": {
                        "type": "integer",
                        "default": 20,
                        "minimum": 1,
                        "maximum": 1000,
                        "description": "Number of versions to return"
                    }
                },
                "required": ["memoir"]
            },
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true
            }
        }),
        json!({
            "name": "hyphae_import_code_graph",
            "title": "Import Code Graph",
            "description": "Import a code symbol graph from Rhizome (or similar tools) into Hyphae as a memoir. Creates or updates the memoir 'code:{project}' with concepts (symbols) and links (relationships). Idempotent — safe to re-import after incremental changes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "schema_version": {
                        "type": "string",
                        "const": "1.0",
                        "description": "Required contract version for code-graph imports. Hyphae rejects missing or unknown versions rather than accepting drifted payloads."
                    },
                    "project": {
                        "type": "string",
                        "description": "Project name. Creates/updates memoir 'code:{project}'."
                    },
                    "project_root": {
                        "type": "string",
                        "description": "Optional repository root for future code-graph identity matching. Rhizome sends this together with worktree_id when identity v1 is active; Hyphae currently keeps memoir ownership keyed by project."
                    },
                    "worktree_id": {
                        "type": "string",
                        "description": "Optional worktree identifier paired with project_root for future code-graph identity matching. Partial identity input is ignored by callers and should not be sent."
                    },
                    "nodes": {
                        "type": "array",
                        "description": "List of code symbols (concepts) to import.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "name": {
                                    "type": "string",
                                    "description": "Unique symbol name within the project (e.g. function or type name)"
                                },
                                "labels": {
                                    "type": "array",
                                    "items": { "type": "string" },
                                    "description": "Symbol kind tags (e.g. 'function', 'struct', 'public', 'async'). Stored with namespace 'code'."
                                },
                                "description": {
                                    "type": "string",
                                    "description": "Human-readable description or signature of the symbol"
                                },
                                "metadata": {
                                    "type": "object",
                                    "description": "Optional extra metadata (ignored by Hyphae, reserved for future use)"
                                }
                            },
                            "required": ["name"]
                        }
                    },
                    "edges": {
                        "type": "array",
                        "description": "List of directed relationships between symbols.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "source": {
                                    "type": "string",
                                    "description": "Source symbol name (must appear in nodes)"
                                },
                                "target": {
                                    "type": "string",
                                    "description": "Target symbol name (must appear in nodes)"
                                },
                                "relation": {
                                    "type": "string",
                                    "description": "Relationship type (e.g. 'calls', 'depends_on', 'implements', 'part_of'). Defaults to 'related_to'."
                                },
                                "weight": {
                                    "type": "number",
                                    "minimum": 0.0,
                                    "maximum": 1.0,
                                    "default": 1.0,
                                    "description": "Edge strength (0.0–1.0). Defaults to 1.0."
                                }
                            },
                            "required": ["source", "target"]
                        }
                    },
                    "prune": {
                        "type": "boolean",
                        "default": true,
                        "description": "If true (default), remove concepts whose names are not in this import (deleted or renamed symbols). Set to false for incremental partial imports."
                    }
                },
                "required": ["schema_version", "project", "nodes", "edges"]
            },
            "annotations": {
                "readOnlyHint": false,
                "destructiveHint": false,
                "idempotentHint": true
            }
        }),
        json!({
            "name": "hyphae_code_query",
            "title": "Query Code Graph",
            "description": "Query a code symbol graph stored in a memoir. Supports symbol listing, call graph analysis, and neighborhood exploration.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": {
                        "type": "string",
                        "description": "Project name. Looks up memoir 'code:{project}'."
                    },
                    "query_type": {
                        "type": "string",
                        "enum": ["symbols", "callers", "callees", "implementors", "structure"],
                        "description": "Type of query: 'symbols' (list concepts), 'callers' (who calls symbol), 'callees' (who symbol calls), 'implementors' (who implements symbol), 'structure' (neighborhood subgraph)"
                    },
                    "symbol": {
                        "type": "string",
                        "description": "Symbol name. Required for callers/callees/implementors/structure; optional for symbols."
                    },
                    "labels": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Filter concepts by labels (e.g. ['function', 'public']). Only used with 'symbols' query. Returns intersection of all label filters."
                    }
                },
                "required": ["project", "query_type"]
            },
            "annotations": {
                "readOnlyHint": true,
                "destructiveHint": false,
                "idempotentHint": true
            }
        }),
    ];

    // Context gathering
    tools.push(json!({
        "name": "hyphae_gather_context",
        "title": "Gather Context",
        "description": "Gather relevant context for a task from across all Hyphae stores (memories, errors, sessions, code). Returns ranked results within a token budget together with a scoped_identity envelope so downstream tools can tell which project/worktree/scope produced the context.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "Task description to gather context for (e.g. 'refactor auth middleware')"
                },
                "project": {
                    "type": "string",
                    "description": "Project name to scope the search (optional, uses configured project if omitted). Required when project_root and worktree_id are supplied so structured session lookup stays bounded."
                },
                "project_root": {
                    "type": "string",
                    "description": "Optional repository root for session identity v1 lookups. Use with worktree_id and project to scope structured session results to one worktree. When supplied alongside worktree_id, project must also be provided."
                },
                "worktree_id": {
                    "type": "string",
                    "description": "Optional worktree identifier for session identity v1 lookups. Use with project_root and project to avoid mixing sibling worktrees in structured session context. When supplied alongside project_root, project must also be provided."
                },
                "scope": {
                    "type": "string",
                    "description": "Optional worker or runtime scope filter for structured session context. Use with project_root and worktree_id when multiple parallel workers share one worktree."
                },
                "token_budget": {
                    "type": "integer",
                    "default": 2000,
                    "minimum": 100,
                    "maximum": 50000,
                    "description": "Maximum tokens to include in context (rough estimate: 4 chars per token)"
                },
                "include": {
                    "type": "array",
                    "items": {
                        "type": "string",
                        "enum": ["memories", "errors", "sessions", "code"]
                    },
                    "description": "Which sources to include (default: all). Options: memories, errors, sessions, code"
                }
            },
            "required": ["task"]
        },
        "annotations": {
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true
        }
    }));

    // Shared context tools
    tools.push(json!({
        "name": "hyphae_context_put",
        "title": "Put Context",
        "description": "Write or overwrite a value in shared cross-agent context. Returns the entry_id of the stored entry.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "string",
                    "description": "Session ID for scoping the context"
                },
                "agent_id": {
                    "type": "string",
                    "description": "Agent identifier that wrote this context (optional, defaults to empty string)"
                },
                "key": {
                    "type": "string",
                    "description": "Key to store the value under"
                },
                "value": {
                    "description": "JSON value to store (any valid JSON type: object, array, string, number, boolean, or null)"
                }
            },
            "required": ["session_id", "key", "value"]
        },
        "annotations": {
            "readOnlyHint": false,
            "destructiveHint": false,
            "idempotentHint": false
        }
    }));

    tools.push(json!({
        "name": "hyphae_context_get",
        "title": "Get Context",
        "description": "Retrieve the most recent value for a key from shared cross-agent context. Returns the entry or {found: false} if the key has never been written.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "string",
                    "description": "Session ID to scope the lookup"
                },
                "key": {
                    "type": "string",
                    "description": "Key to retrieve"
                }
            },
            "required": ["session_id", "key"]
        },
        "annotations": {
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true
        }
    }));

    // RAG tools
    tools.push(json!({
        "name": "hyphae_ingest_file",
        "title": "Ingest File",
        "description": "Ingest a file or directory into Hyphae's document store for RAG search. Chunks the content and stores it for later retrieval.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Absolute or relative path to a file or directory to ingest"
                },
                "recursive": {
                    "type": "boolean",
                    "default": false,
                    "description": "If path is a directory, recurse into subdirectories"
                }
            },
            "required": ["path"]
        },
        "annotations": {
            "readOnlyHint": false,
            "destructiveHint": false,
            "idempotentHint": true
        }
    }));
    tools.push(json!({
        "name": "hyphae_search_docs",
        "title": "Search Documents",
        "description": "Search ingested documents using hybrid (vector + FTS) or Full-text Search (FTS) search. Returns ranked chunks with source paths and scores.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural language search query"
                },
                "limit": {
                    "type": "integer",
                    "default": 10,
                    "minimum": 1,
                    "maximum": 100,
                    "description": "Maximum number of results to return"
                },
                "offset": {
                    "type": "integer",
                    "default": 0,
                    "minimum": 0,
                    "description": "Number of results to skip (for pagination)"
                }
            },
            "required": ["query"]
        },
        "annotations": {
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true
        }
    }));
    tools.push(json!({
        "name": "hyphae_list_sources",
        "title": "List Sources",
        "description": "List all ingested document sources with their type, chunk count, and ingestion date.",
        "inputSchema": {
            "type": "object",
            "properties": {}
        },
        "annotations": {
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true
        }
    }));
    tools.push(json!({
        "name": "hyphae_forget_source",
        "title": "Forget Source",
        "description": "Remove an ingested document source and all its chunks from the store.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Source path of the document to delete (as shown by hyphae_list_sources)"
                }
            },
            "required": ["path"]
        },
        "annotations": {
            "readOnlyHint": false,
            "destructiveHint": true,
            "idempotentHint": true
        }
    }));
    tools.push(json!({
        "name": "hyphae_search_all",
        "title": "Search All",
        "description": "Unified cross-store search across memories and ingested documents. Returns ranked results using Reciprocal Rank Fusion. When project_root and worktree_id are supplied together, memory results are scoped to the active worktree and _shared memories are still included. Document chunks remain project-scoped.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural language search query"
                },
                "project_root": {
                    "type": "string",
                    "description": "Optional repository root for identity v1 lookups. Use with worktree_id to scope memory results to the active worktree. When supplied, worktree_id must also be provided (and vice versa)."
                },
                "worktree_id": {
                    "type": "string",
                    "description": "Optional worktree identifier for identity v1 lookups. Use with project_root to scope memory results to the active worktree. When supplied, project_root must also be provided (and vice versa)."
                },
                "limit": {
                    "type": "integer",
                    "default": 10,
                    "minimum": 1,
                    "maximum": 50,
                    "description": "Total results across both stores"
                },
                "include_docs": {
                    "type": "boolean",
                    "default": true,
                    "description": "Whether to include document chunks in results"
                },
                "offset": {
                    "type": "integer",
                    "default": 0,
                    "minimum": 0,
                    "description": "Number of results to skip (for pagination)"
                }
            },
            "required": ["query"]
        },
        "annotations": {
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true
        }
    }));

    // Command output tools
    tools.push(json!({
        "name": "hyphae_store_command_output",
        "title": "Store Command Output",
        "description": "Store command output as chunked documents with ephemeral importance. Automatically detects output type (test results, build errors, diffs, logs) and chunks accordingly.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "schema_version": {
                    "type": "string",
                    "const": "1.0",
                    "description": "Contract version for command-output-v1. Receivers reject missing or unknown versions."
                },
                "command": {
                    "type": "string",
                    "description": "The command that produced this output (e.g. 'cargo test', 'git diff')"
                },
                "output": {
                    "type": "string",
                    "description": "The raw command output to store"
                },
                "project": {
                    "type": "string",
                    "description": "Project name for scoping (optional)"
                },
                "project_root": {
                    "type": "string",
                    "description": "Optional repository root for command-output identity v1. When paired with worktree_id, Hyphae namespaces the stored command-output source path so same-command captures do not collide across worktrees or projects. Partial identity input is ignored and legacy replacement behavior is preserved."
                },
                "worktree_id": {
                    "type": "string",
                    "description": "Optional worktree identifier for command-output identity v1. Use with project_root to namespace stored command output; partial identity input is ignored and legacy replacement behavior is preserved."
                },
                "runtime_session_id": {
                    "type": "string",
                    "description": "Optional external runtime session id propagated from the calling agent environment. Hyphae stores it on the command-output document so chunk retrieval can be correlated back to the originating runtime session."
                },
                "ttl_hours": {
                    "type": "integer",
                    "default": 4,
                    "minimum": 1,
                    "maximum": 168,
                    "description": "Hours before the summary memory expires (default 4)"
                }
            },
            "required": ["schema_version", "command", "output"]
        },
        "annotations": {
            "readOnlyHint": false,
            "destructiveHint": false,
            "idempotentHint": false
        }
    }));
    tools.push(json!({
        "name": "hyphae_get_command_chunks",
        "title": "Get Command Chunks",
        "description": "Retrieve chunks from a stored command output document by document_id with pagination.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "document_id": {
                    "type": "string",
                    "description": "Document ID returned by hyphae_store_command_output"
                },
                "offset": {
                    "type": "integer",
                    "default": 0,
                    "minimum": 0,
                    "description": "Number of chunks to skip"
                },
                "limit": {
                    "type": "integer",
                    "default": 5,
                    "minimum": 1,
                    "maximum": 20,
                    "description": "Maximum number of chunks to return"
                }
            },
            "required": ["document_id"]
        },
        "annotations": {
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true
        }
    }));

    // Session lifecycle tools
    tools.push(json!({
        "name": "hyphae_session_start",
        "title": "Start Session",
        "description": "Start a new coding session. Creates a session record that tracks project work. Call at the beginning of a task to enable session lifecycle tracking. Returns a session_id plus a scoped_identity envelope for downstream coordination.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "project": {
                    "type": "string",
                    "description": "Project identifier (e.g. repo name or workspace path)"
                },
                "task": {
                    "type": "string",
                    "description": "Brief description of the task being worked on (optional)"
                },
                "project_root": {
                    "type": "string",
                    "description": "Optional repository root for session identity v1. Use with worktree_id to identify a specific structured session. When scope is also set, scope participates in identity matching so parallel sessions stay distinct."
                },
                "worktree_id": {
                    "type": "string",
                    "description": "Optional worktree identifier for session identity v1. Use with project_root to identify a specific structured session. When scope is also set, scope participates in identity matching so parallel sessions stay distinct."
                },
                "scope": {
                    "type": "string",
                    "description": "Optional worker or runtime scope. When paired with project_root and worktree_id, it prevents distinct parallel sessions from collapsing onto one identity."
                },
                "runtime_session_id": {
                    "type": "string",
                    "description": "Optional external runtime session id propagated from the calling agent environment. Hyphae stores it as metadata so downstream consumers can correlate Hyphae sessions with Mycelium history and Canopy evidence."
                }
            },
            "required": ["project"]
        },
        "annotations": {
            "readOnlyHint": false,
            "destructiveHint": false,
            "idempotentHint": false
        }
    }));

    tools.push(json!({
        "name": "hyphae_session_end",
        "title": "End Session",
        "description": "End a coding session and store a summary in the session record. Updates the session with completion data. Call when finishing a task.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "session_id": {
                    "type": "string",
                    "description": "Session ID returned by hyphae_session_start"
                },
                "summary": {
                    "type": "string",
                    "description": "Brief summary of what was accomplished"
                },
                "files_modified": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "List of files that were modified during the session"
                },
                "errors_encountered": {
                    "type": "integer",
                    "description": "Number of errors encountered during the session",
                    "default": 0
                }
            },
            "required": ["session_id"]
        },
        "annotations": {
            "readOnlyHint": false,
            "destructiveHint": false,
            "idempotentHint": true
        }
    }));

    tools.push(json!({
        "name": "hyphae_session_context",
        "title": "Get Session Context",
        "description": "Get recent session history for a project. Returns the last N sessions with their summaries, tasks, and status, plus a scoped_identity envelope that makes the queried identity explicit.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "project": {
                    "type": "string",
                    "description": "Project identifier to query sessions for"
                },
                "project_root": {
                    "type": "string",
                    "description": "Optional repository root for session identity v1 lookups. Use with worktree_id to select structured sessions. Add scope to narrow results to one parallel worker when needed."
                },
                "worktree_id": {
                    "type": "string",
                    "description": "Optional worktree identifier for session identity v1 lookups. Use with project_root to select structured sessions. Add scope to narrow results to one parallel worker when needed."
                },
                "scope": {
                    "type": "string",
                    "description": "Optional worker or runtime scope filter. Use when multiple scoped sessions exist for the same worktree."
                },
                "limit": {
                    "type": "integer",
                    "default": 5,
                    "minimum": 1,
                    "maximum": 50,
                    "description": "Maximum number of recent sessions to return"
                }
            },
            "required": ["project"]
        },
        "annotations": {
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true
        }
    }));

    // Artifact tools
    tools.push(json!({
        "name": "hyphae_artifact_store",
        "title": "Store Artifact",
        "description": "Store a typed artifact (compact summary, council record, project understanding) into persistent artifact storage",
        "inputSchema": {
            "type": "object",
            "properties": {
                "artifact_type": {
                    "type": "string",
                    "enum": ["compact_summary", "council_lifecycle", "project_understanding"],
                    "description": "Type of artifact to store"
                },
                "project": {
                    "type": "string",
                    "description": "Project name to associate with the artifact"
                },
                "payload": {
                    "type": "string",
                    "description": "JSON content of the artifact"
                },
                "source_id": {
                    "type": "string",
                    "description": "Optional source identifier (e.g. session id, handoff slug)"
                }
            },
            "required": ["artifact_type", "project", "payload"]
        },
        "annotations": {
            "readOnlyHint": false,
            "destructiveHint": false,
            "idempotentHint": true
        }
    }));
    tools.push(json!({
        "name": "hyphae_artifact_query",
        "title": "Query Artifacts",
        "description": "Query stored artifacts by type and project",
        "inputSchema": {
            "type": "object",
            "properties": {
                "artifact_type": {
                    "type": "string",
                    "enum": ["compact_summary", "council_lifecycle", "project_understanding"],
                    "description": "Type of artifact to query"
                },
                "project": {
                    "type": "string",
                    "description": "Project name to scope the query"
                },
                "limit": {
                    "type": "integer",
                    "minimum": 1,
                    "maximum": 100,
                    "description": "How many results to return"
                }
            },
            "required": ["artifact_type", "project"]
        },
        "annotations": {
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true
        }
    }));

    // Onboarding tool
    tools.push(json!({
        "name": "hyphae_onboard",
        "title": "Onboard Hyphae",
        "description": "Get a quick overview of the Hyphae memory system for onboarding. Returns total memories, memoirs, topics, available tools, and a quick-start guide. No parameters required.",
        "inputSchema": {
            "type": "object",
            "properties": {}
        },
        "annotations": {
            "readOnlyHint": false,
            "destructiveHint": false,
            "idempotentHint": true
        }
    }));

    if has_embedder {
        tools.push(json!({
            "name": "hyphae_memory_embed_all",
            "title": "Embed All Memories",
            "description": "Generate embeddings for all memories that don't have one yet. Use this to backfill vector search capability.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "topic": {
                        "type": "string",
                        "description": "Only embed memories in this topic (optional)"
                    }
                }
            },
            "annotations": {
                "readOnlyHint": false,
                "destructiveHint": false,
                "idempotentHint": true
            }
        }));
    }

    tools.push(json!({
        "name": "hyphae_extract_lessons",
        "title": "Extract Lessons",
        "description": "Extract actionable lessons from accumulated corrections, error resolutions, and test fixes. Returns patterns that help avoid repeating past mistakes.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "limit": {
                    "type": "integer",
                    "default": 10,
                    "minimum": 1,
                    "maximum": 50,
                    "description": "Maximum number of lessons to extract"
                }
            }
        },
        "annotations": {
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true
        }
    }));

    tools.push(json!({
        "name": "hyphae_reflexion_record",
        "title": "Record Reflexion",
        "description": "Store a structured reflexion entry capturing error type, root cause, fix applied, and abstract pattern for future recall.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "error_type": {
                    "type": "string",
                    "description": "Category of error (e.g. 'logic', 'type', 'runtime', 'integration')"
                },
                "root_cause": {
                    "type": "string",
                    "description": "Concise description of the root cause of the error"
                },
                "fix_applied": {
                    "type": "string",
                    "description": "Description of the fix that resolved the error"
                },
                "abstract_pattern": {
                    "type": "string",
                    "description": "Reusable abstract pattern extracted from the error and fix"
                },
                "project": {
                    "type": "string",
                    "description": "Project name to scope the record (optional)"
                },
                "confidence": {
                    "type": "string",
                    "enum": ["low", "medium", "high"],
                    "default": "medium",
                    "description": "Confidence level that this pattern generalizes"
                }
            },
            "required": ["error_type", "root_cause", "fix_applied", "abstract_pattern"]
        },
        "annotations": {
            "readOnlyHint": false,
            "destructiveHint": false,
            "idempotentHint": false
        }
    }));

    tools.push(json!({
        "name": "hyphae_reflexion_search",
        "title": "Search Reflexion",
        "description": "Search reflexion records by query, returning structured entries sorted by confidence then recency.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural language search query"
                },
                "error_type": {
                    "type": "string",
                    "description": "Filter results to a specific error type (optional)"
                },
                "limit": {
                    "type": "integer",
                    "default": 10,
                    "minimum": 1,
                    "maximum": 100,
                    "description": "Maximum number of results to return"
                }
            },
            "required": ["query"]
        },
        "annotations": {
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true
        }
    }));

    // Knowledge domain tools
    tools.push(json!({
        "name": "hyphae_domain_upsert",
        "title": "Upsert Domain",
        "description": "Create or update a knowledge domain that describes when and how to recall information with specific applicability rules and required inputs",
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": {
                    "type": "string",
                    "description": "Unique identifier for this domain"
                },
                "description": {
                    "type": "string",
                    "description": "Human-readable description of what this domain covers"
                },
                "applies_when": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "field": { "type": "string" },
                            "op": {
                                "type": "string",
                                "enum": ["exists", "equals", "contains", "greater_than"]
                            },
                            "value": { "type": ["string", "number", "boolean"] }
                        },
                        "required": ["field", "op", "value"]
                    },
                    "description": "Applicability rules that must be satisfied (empty = always applicable)"
                },
                "required_inputs": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" },
                            "description": { "type": "string" },
                            "required": { "type": "boolean" }
                        },
                        "required": ["name", "description", "required"]
                    },
                    "description": "Input specifications that consumers must provide"
                },
                "query_template": {
                    "type": "string",
                    "description": "Optional query template for domain-specific searches"
                },
                "authority": {
                    "type": "string",
                    "enum": ["primary", "derived", "historical"],
                    "default": "primary",
                    "description": "Authority level of this domain"
                },
                "freshness_ttl_secs": {
                    "type": "integer",
                    "description": "Time-to-live for domain knowledge in seconds (optional)"
                },
                "boundary_note": {
                    "type": "string",
                    "description": "Note about the scope or boundaries of this domain"
                }
            },
            "required": ["id", "description"]
        },
        "annotations": {
            "readOnlyHint": false,
            "destructiveHint": false,
            "idempotentHint": true
        }
    }));

    tools.push(json!({
        "name": "hyphae_domain_list",
        "title": "List Domains",
        "description": "List all defined knowledge domains",
        "inputSchema": {
            "type": "object",
            "properties": {}
        },
        "annotations": {
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true
        }
    }));

    tools
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gather_context_documents_project_requirement_for_full_identity() {
        let tools = tool_definitions_json(false);
        let gather_tool = tools
            .iter()
            .find(|tool| tool["name"] == "hyphae_gather_context")
            .expect("gather context tool");

        assert!(gather_tool["inputSchema"]["allOf"].is_null());
        assert!(
            gather_tool["inputSchema"]["properties"]["project_root"]["description"]
                .as_str()
                .expect("project_root description")
                .contains("project must also be provided")
        );
        assert!(
            gather_tool["inputSchema"]["properties"]["worktree_id"]["description"]
                .as_str()
                .expect("worktree_id description")
                .contains("project must also be provided")
        );
        assert_eq!(
            gather_tool["inputSchema"]["properties"]["scope"]["type"],
            "string"
        );
    }

    #[test]
    fn test_recall_documents_identity_fields_in_pairs() {
        let tools = tool_definitions_json(false);
        let recall_tool = tools
            .iter()
            .find(|tool| tool["name"] == "hyphae_memory_recall")
            .expect("memory recall tool");

        assert!(recall_tool["inputSchema"]["allOf"].is_null());
        assert!(
            recall_tool["inputSchema"]["properties"]["project_root"]["description"]
                .as_str()
                .expect("project_root description")
                .contains("worktree_id must also be provided")
        );
        assert!(
            recall_tool["inputSchema"]["properties"]["worktree_id"]["description"]
                .as_str()
                .expect("worktree_id description")
                .contains("project_root must also be provided")
        );
    }

    #[test]
    fn test_search_all_documents_identity_fields_in_pairs() {
        let tools = tool_definitions_json(false);
        let search_tool = tools
            .iter()
            .find(|tool| tool["name"] == "hyphae_search_all")
            .expect("search-all tool");

        assert!(search_tool["inputSchema"]["allOf"].is_null());
        assert!(
            search_tool["inputSchema"]["properties"]["project_root"]["description"]
                .as_str()
                .expect("project_root description")
                .contains("worktree_id must also be provided")
        );
        assert!(
            search_tool["inputSchema"]["properties"]["worktree_id"]["description"]
                .as_str()
                .expect("worktree_id description")
                .contains("project_root must also be provided")
        );
    }
}
