use serde_json::{Value, json};

/// Build the list of tool definitions. When `has_embedder` is false the
/// `hyphae_memory_embed_all` tool is omitted.
pub(super) fn tool_definitions_json(has_embedder: bool) -> Vec<Value> {
    let mut tools = vec![
        // --- Memory tools ---
        json!({
            "name": "hyphae_memory_store",
            "description": "Store important information in Hyphae long-term memory. Use to save decisions, preferences, project context, resolved errors — anything that should persist between sessions.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "topic": {
                        "type": "string",
                        "description": "Category/namespace (e.g. 'project-kexa', 'preferences', 'decisions-architecture', 'erreurs-resolues')"
                    },
                    "content": {
                        "type": "string",
                        "maxLength": 32768,
                        "description": "Information to memorize — be concise but complete"
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
            "description": "Search Hyphae long-term memory. Use to find past decisions, project context, preferences, or solutions to previously encountered problems.",
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
        // --- Memoir tools ---
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
            "description": "Inspect a concept and its graph neighborhood (BFS).",
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
    ];

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
