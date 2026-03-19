use serde_json::{Value, json};

/// Build the list of tool definitions. When `has_embedder` is false the
/// `hyphae_memory_embed_all` tool is omitted.
pub(super) fn tool_definitions_json(has_embedder: bool) -> Vec<Value> {
    let mut tools = vec![
        // Memory tools
        json!({
            "name": "hyphae_memory_store",
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
                        "description": "critical=never forgotten, high=slow decay, medium=normal, low=fast decay"
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
                    }
                },
                "required": ["topic", "content"]
            }
        }),
        json!({
            "name": "hyphae_memory_recall",
            "description": "Search Hyphae long-term memory. Use to find past decisions, project context, preferences, or solutions to previously encountered problems. Automatically includes results from the '_shared' knowledge pool alongside project-scoped results.",
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
                    "code_context": {
                        "type": "boolean",
                        "default": false,
                        "description": "When true, expands the search with code symbols from the project's code memoir (code:{project}). Only effective when a project is configured and the query looks code-related."
                    }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "hyphae_memory_forget",
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
            }
        }),
        json!({
            "name": "hyphae_memory_consolidate",
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
            }
        }),
        json!({
            "name": "hyphae_memory_list_topics",
            "description": "List all available topics in memory with their counts.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "hyphae_memory_stats",
            "description": "Get global Hyphae memory statistics.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "hyphae_memory_update",
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
            }
        }),
        json!({
            "name": "hyphae_memory_health",
            "description": "Get health stats for all topics: entry count, staleness, consolidation needs. Use to audit memory hygiene.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "topic": {
                        "type": "string",
                        "description": "Check a specific topic (optional — checks all if omitted)"
                    }
                }
            }
        }),
        // Cross-project tools
        json!({
            "name": "hyphae_recall_global",
            "description": "Search memories across ALL projects. Returns results grouped by project. Use when knowledge may exist in another project, or to find cross-cutting patterns. The special '_shared' project holds globally visible knowledge.",
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
                        "description": "Max total results across all projects"
                    }
                },
                "required": ["query"]
            }
        }),
        json!({
            "name": "hyphae_promote_to_memoir",
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
            }
        }),
        // Memoir tools
        json!({
            "name": "hyphae_memoir_create",
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
            }
        }),
        json!({
            "name": "hyphae_memoir_list",
            "description": "List all memoirs with their concept counts.",
            "inputSchema": {
                "type": "object",
                "properties": {}
            }
        }),
        json!({
            "name": "hyphae_memoir_show",
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
            }
        }),
        json!({
            "name": "hyphae_memoir_add_concept",
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
                    }
                },
                "required": ["memoir", "name", "definition"]
            }
        }),
        json!({
            "name": "hyphae_memoir_refine",
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
                    }
                },
                "required": ["memoir", "name", "definition"]
            }
        }),
        json!({
            "name": "hyphae_memoir_search",
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
                    "limit": {
                        "type": "integer",
                        "default": 10,
                        "description": "Max results"
                    }
                },
                "required": ["memoir", "query"]
            }
        }),
        json!({
            "name": "hyphae_memoir_link",
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
            }
        }),
        json!({
            "name": "hyphae_memoir_inspect",
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
            }
        }),
        json!({
            "name": "hyphae_memoir_search_all",
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
            }
        }),
        json!({
            "name": "hyphae_import_code_graph",
            "description": "Import a code symbol graph from Rhizome (or similar tools) into Hyphae as a memoir. Creates or updates the memoir 'code:{project}' with concepts (symbols) and links (relationships). Idempotent — safe to re-import after incremental changes.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "project": {
                        "type": "string",
                        "description": "Project name. Creates/updates memoir 'code:{project}'."
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
                "required": ["project", "nodes", "edges"]
            }
        }),
        json!({
            "name": "hyphae_code_query",
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
            }
        }),
    ];

    // Context gathering
    tools.push(json!({
        "name": "hyphae_gather_context",
        "description": "Gather relevant context for a task from across all Hyphae stores (memories, errors, sessions, code). Returns ranked results within a token budget. Use at the start of a task to bootstrap context.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "Task description to gather context for (e.g. 'refactor auth middleware')"
                },
                "project": {
                    "type": "string",
                    "description": "Project name to scope the search (optional, uses configured project if omitted)"
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
        }
    }));

    // RAG tools
    tools.push(json!({
        "name": "hyphae_ingest_file",
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
        }
    }));
    tools.push(json!({
        "name": "hyphae_search_docs",
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
        }
    }));
    tools.push(json!({
        "name": "hyphae_list_sources",
        "description": "List all ingested document sources with their type, chunk count, and ingestion date.",
        "inputSchema": {
            "type": "object",
            "properties": {}
        }
    }));
    tools.push(json!({
        "name": "hyphae_forget_source",
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
        }
    }));
    tools.push(json!({
        "name": "hyphae_search_all",
        "description": "Unified cross-store search across memories and ingested documents. Returns ranked results using Reciprocal Rank Fusion.",
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
        }
    }));

    // Command output tools
    tools.push(json!({
        "name": "hyphae_store_command_output",
        "description": "Store command output as chunked documents with ephemeral importance. Automatically detects output type (test results, build errors, diffs, logs) and chunks accordingly.",
        "inputSchema": {
            "type": "object",
            "properties": {
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
                "ttl_hours": {
                    "type": "number",
                    "default": 4,
                    "minimum": 1,
                    "maximum": 168,
                    "description": "Hours before the summary memory expires (default 4)"
                }
            },
            "required": ["command", "output"]
        }
    }));
    tools.push(json!({
        "name": "hyphae_get_command_chunks",
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
        }
    }));

    // Session lifecycle tools
    tools.push(json!({
        "name": "hyphae_session_start",
        "description": "Start a new coding session. Creates a session record that tracks project work. Call at the beginning of a task to enable session lifecycle tracking. Returns a session_id for use with hyphae_session_end.",
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
                }
            },
            "required": ["project"]
        }
    }));

    tools.push(json!({
        "name": "hyphae_session_end",
        "description": "End a coding session and store a summary. Updates the session record with completion data and optionally stores the summary as a persistent memory for future context. Call when finishing a task.",
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
        }
    }));

    tools.push(json!({
        "name": "hyphae_session_context",
        "description": "Get recent session history for a project. Returns the last N sessions with their summaries, tasks, and status. Use at the start of a new session to understand recent project context.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "project": {
                    "type": "string",
                    "description": "Project identifier to query sessions for"
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
        }
    }));

    // Onboarding tool
    tools.push(json!({
        "name": "hyphae_onboard",
        "description": "Get a quick overview of the Hyphae memory system for onboarding. Returns total memories, memoirs, topics, available tools, and a quick-start guide. No parameters required.",
        "inputSchema": {
            "type": "object",
            "properties": {}
        }
    }));

    if has_embedder {
        tools.push(json!({
            "name": "hyphae_memory_embed_all",
            "description": "Generate embeddings for all memories that don't have one yet. Use this to backfill vector search capability.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "topic": {
                        "type": "string",
                        "description": "Only embed memories in this topic (optional)"
                    }
                }
            }
        }));
    }

    tools
}
