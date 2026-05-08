use rusqlite::Connection;
use rusqlite_migration::{M, Migrations};

use hyphae_core::HyphaeError;

/// Baseline schema SQL — all CREATE TABLE IF NOT EXISTS statements and regular indexes.
/// Used as M0 in the migration vec and also run inside bootstrap_existing_db to ensure
/// any tables that were added after a database was first created are present.
/// FTS5 virtual tables and sqlite-vec tables are handled separately (no IF NOT EXISTS support).
const BASELINE_SQL: &str = "
            CREATE TABLE IF NOT EXISTS memories (
                id TEXT PRIMARY KEY,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT '',
                last_accessed TEXT NOT NULL,
                access_count INTEGER DEFAULT 0,
                weight REAL DEFAULT 1.0,

                topic TEXT NOT NULL,
                summary TEXT NOT NULL,
                raw_excerpt TEXT,
                keywords TEXT, -- JSON array

                importance TEXT NOT NULL,
                source_type TEXT NOT NULL,
                source_data TEXT, -- JSON

                related_ids TEXT, -- JSON array
                project TEXT,
                branch TEXT,
                worktree TEXT,
                expires_at TEXT,
                invalidated_at TEXT,
                invalidation_reason TEXT,
                superseded_by TEXT,
                agent_id TEXT,
                embedding BLOB,
                tier TEXT NOT NULL DEFAULT 'recall'
            );

            CREATE INDEX IF NOT EXISTS idx_memories_topic ON memories(topic);
            CREATE INDEX IF NOT EXISTS idx_memories_weight ON memories(weight);
            CREATE INDEX IF NOT EXISTS idx_memories_created ON memories(created_at);
            CREATE INDEX IF NOT EXISTS idx_memories_importance_weight ON memories(importance, weight);
            CREATE INDEX IF NOT EXISTS idx_memories_project ON memories(project);
            CREATE INDEX IF NOT EXISTS idx_memories_expires_at ON memories(expires_at);

            -- Memoir tables
            CREATE TABLE IF NOT EXISTS memoirs (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                description TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                consolidation_threshold INTEGER NOT NULL DEFAULT 50,
                author TEXT NOT NULL DEFAULT '',
                git_hash TEXT,
                parent_version_id TEXT,
                decay TEXT NOT NULL DEFAULT 'standard',
                authority TEXT NOT NULL DEFAULT 'primary',
                source TEXT NOT NULL DEFAULT 'agent',
                compiled_at TEXT,
                invalidated_at TEXT,
                invalidated_by TEXT,
                freshness_ttl_secs INTEGER
            );

            CREATE TABLE IF NOT EXISTS memoir_versions (
                version_id TEXT PRIMARY KEY,
                memoir_id TEXT NOT NULL REFERENCES memoirs(id) ON DELETE CASCADE,
                version_seq INTEGER NOT NULL,
                author TEXT NOT NULL DEFAULT '',
                git_hash TEXT,
                diff_summary TEXT NOT NULL DEFAULT '',
                created_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_memoir_versions_memoir ON memoir_versions(memoir_id, version_seq);

            CREATE TABLE IF NOT EXISTS concepts (
                id TEXT PRIMARY KEY,
                memoir_id TEXT NOT NULL REFERENCES memoirs(id) ON DELETE CASCADE,
                name TEXT NOT NULL,
                definition TEXT NOT NULL,
                labels TEXT NOT NULL DEFAULT '[]', -- JSON array of {namespace, value}
                confidence REAL NOT NULL DEFAULT 0.5,
                revision INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                source_memory_ids TEXT NOT NULL DEFAULT '[]', -- JSON array of strings
                community_id TEXT,
                abstract_text TEXT,
                overview_text TEXT,
                block_type TEXT NULL DEFAULT NULL,
                UNIQUE(memoir_id, name)
            );

            CREATE INDEX IF NOT EXISTS idx_concepts_memoir ON concepts(memoir_id);
            CREATE INDEX IF NOT EXISTS idx_concepts_name ON concepts(name);
            CREATE INDEX IF NOT EXISTS idx_concepts_confidence ON concepts(confidence);

            CREATE TABLE IF NOT EXISTS concept_links (
                id TEXT PRIMARY KEY,
                source_id TEXT NOT NULL REFERENCES concepts(id) ON DELETE CASCADE,
                target_id TEXT NOT NULL REFERENCES concepts(id) ON DELETE CASCADE,
                relation TEXT NOT NULL,
                weight REAL NOT NULL DEFAULT 1.0,
                link_count INTEGER NOT NULL DEFAULT 1,
                created_at TEXT NOT NULL,
                valid_from TEXT NOT NULL DEFAULT '',
                valid_to TEXT,
                UNIQUE(source_id, target_id, relation),
                CHECK(source_id != target_id)
            );

            CREATE INDEX IF NOT EXISTS idx_concept_links_source ON concept_links(source_id);
            CREATE INDEX IF NOT EXISTS idx_concept_links_target ON concept_links(target_id);

            -- Session lifecycle tracking
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                project TEXT NOT NULL,
                project_root TEXT,
                worktree_id TEXT,
                scope TEXT,
                runtime_session_id TEXT,
                task TEXT,
                started_at TEXT NOT NULL,
                ended_at TEXT,
                summary TEXT,
                files_modified TEXT,
                errors TEXT,
                status TEXT NOT NULL DEFAULT 'active'
            );

            CREATE INDEX IF NOT EXISTS idx_sessions_project ON sessions(project);
            CREATE INDEX IF NOT EXISTS idx_sessions_started_at ON sessions(started_at);
            CREATE INDEX IF NOT EXISTS idx_sessions_project_scope ON sessions(project, scope);
            CREATE INDEX IF NOT EXISTS idx_sessions_project_root_worktree ON sessions(project_root, worktree_id);
            CREATE INDEX IF NOT EXISTS idx_sessions_runtime_session_id ON sessions(runtime_session_id);

            -- Feedback loop tracking
            CREATE TABLE IF NOT EXISTS recall_events (
                id TEXT PRIMARY KEY,
                session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL,
                query TEXT NOT NULL,
                recalled_at TEXT NOT NULL,
                memory_ids TEXT NOT NULL,
                memory_count INTEGER NOT NULL,
                project TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_recall_events_session
                ON recall_events(session_id);
            CREATE INDEX IF NOT EXISTS idx_recall_events_recalled_at
                ON recall_events(recalled_at);

            CREATE TABLE IF NOT EXISTS outcome_signals (
                id TEXT PRIMARY KEY,
                session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL,
                recall_event_id TEXT REFERENCES recall_events(id) ON DELETE SET NULL,
                signal_type TEXT NOT NULL,
                signal_value INTEGER NOT NULL,
                occurred_at TEXT NOT NULL,
                source TEXT,
                project TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_outcome_signals_session
                ON outcome_signals(session_id);
            CREATE INDEX IF NOT EXISTS idx_outcome_signals_occurred_at
                ON outcome_signals(occurred_at);
            CREATE INDEX IF NOT EXISTS idx_outcome_signals_recall_event
                ON outcome_signals(recall_event_id);

            CREATE TABLE IF NOT EXISTS recall_effectiveness (
                memory_id TEXT NOT NULL,
                recall_event_id TEXT NOT NULL,
                effectiveness REAL NOT NULL,
                signal_count INTEGER NOT NULL,
                computed_at TEXT NOT NULL,
                PRIMARY KEY (memory_id, recall_event_id)
            );

            CREATE INDEX IF NOT EXISTS idx_recall_effectiveness_memory
                ON recall_effectiveness(memory_id);

            -- RAG tables
            CREATE TABLE IF NOT EXISTS documents (
                id TEXT PRIMARY KEY,
                source_path TEXT NOT NULL,
                source_type TEXT NOT NULL,
                chunk_count INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                project TEXT,
                runtime_session_id TEXT,
                content_hash TEXT,
                UNIQUE(project, source_path)
            );

            CREATE TABLE IF NOT EXISTS chunks (
                id TEXT PRIMARY KEY,
                document_id TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
                chunk_index INTEGER NOT NULL,
                content TEXT NOT NULL,
                source_path TEXT NOT NULL,
                source_type TEXT NOT NULL,
                language TEXT,
                heading TEXT,
                line_start INTEGER,
                line_end INTEGER,
                created_at TEXT NOT NULL,
                chunk_strategy TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_chunks_document_id ON chunks(document_id);
            CREATE INDEX IF NOT EXISTS idx_chunks_source_path ON chunks(source_path);
            CREATE INDEX IF NOT EXISTS idx_documents_project_source ON documents(project, source_path);
            CREATE INDEX IF NOT EXISTS idx_documents_project ON documents(project);
            CREATE INDEX IF NOT EXISTS idx_documents_runtime_session_id ON documents(runtime_session_id);

            -- Knowledge Domain Manifest Layer
            CREATE TABLE IF NOT EXISTS knowledge_domains (
                id TEXT PRIMARY KEY,
                description TEXT NOT NULL DEFAULT '',
                applies_when TEXT NOT NULL DEFAULT '[]',
                required_inputs TEXT NOT NULL DEFAULT '[]',
                query_template TEXT,
                authority TEXT NOT NULL DEFAULT 'primary',
                freshness_ttl_secs INTEGER,
                boundary_note TEXT,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE INDEX IF NOT EXISTS idx_knowledge_domains_created_at
                ON knowledge_domains(created_at);
            CREATE INDEX IF NOT EXISTS idx_knowledge_domains_authority
                ON knowledge_domains(authority);

            -- Metadata key-value table for internal state
            CREATE TABLE IF NOT EXISTS hyphae_metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            -- Project links table for cross-project relationships
            CREATE TABLE IF NOT EXISTS project_links (
                source_project TEXT NOT NULL,
                target_project TEXT NOT NULL,
                created_at TEXT NOT NULL,
                PRIMARY KEY (source_project, target_project),
                CHECK(source_project != target_project)
            );

            -- Artifact storage table
            CREATE TABLE IF NOT EXISTS artifacts (
                artifact_id    TEXT PRIMARY KEY,
                artifact_type  TEXT NOT NULL,
                project        TEXT,
                source_id      TEXT,
                payload        TEXT NOT NULL,
                created_at     TEXT NOT NULL,
                schema_version TEXT NOT NULL DEFAULT '1.0'
            );

            CREATE INDEX IF NOT EXISTS idx_artifacts_type ON artifacts(artifact_type);
            CREATE INDEX IF NOT EXISTS idx_artifacts_project ON artifacts(project);
            CREATE INDEX IF NOT EXISTS idx_artifacts_created_at ON artifacts(created_at);
            CREATE INDEX IF NOT EXISTS idx_artifacts_type_project ON artifacts(artifact_type, project);

            -- Audit log table
            CREATE TABLE IF NOT EXISTS audit_log (
                id TEXT PRIMARY KEY,
                timestamp TEXT NOT NULL,
                operation TEXT NOT NULL,
                memory_id TEXT NOT NULL,
                topic TEXT,
                content_hash TEXT,
                metadata_json TEXT
            );

            CREATE INDEX IF NOT EXISTS idx_audit_log_timestamp ON audit_log(timestamp);
            CREATE INDEX IF NOT EXISTS idx_audit_log_operation ON audit_log(operation);
            CREATE INDEX IF NOT EXISTS idx_audit_log_memory_id ON audit_log(memory_id);

            -- Shared cross-agent context
            CREATE TABLE IF NOT EXISTS shared_context (
                entry_id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                agent_id TEXT NOT NULL DEFAULT '',
                key TEXT NOT NULL,
                value TEXT NOT NULL,
                written_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_shared_context_session_key
                ON shared_context(session_id, key, written_at DESC);

            -- Reflexion records for structured error learning
            CREATE TABLE IF NOT EXISTS reflexion_records (
                id TEXT PRIMARY KEY,
                error_type TEXT NOT NULL,
                root_cause TEXT NOT NULL,
                fix_applied TEXT NOT NULL,
                abstract_pattern TEXT NOT NULL,
                project TEXT,
                confidence TEXT NOT NULL DEFAULT 'medium',
                created_at TEXT NOT NULL
            );

            CREATE INDEX IF NOT EXISTS idx_reflexion_records_error_type ON reflexion_records(error_type);
            CREATE INDEX IF NOT EXISTS idx_reflexion_records_confidence ON reflexion_records(confidence);
            CREATE INDEX IF NOT EXISTS idx_reflexion_records_created_at ON reflexion_records(created_at);
            CREATE INDEX IF NOT EXISTS idx_reflexion_records_project ON reflexion_records(project);
            CREATE INDEX IF NOT EXISTS idx_reflexion_records_confidence_created ON reflexion_records(confidence, created_at DESC);
            ";

fn migrations() -> Migrations<'static> {
    // M0: baseline schema — all CREATE TABLE IF NOT EXISTS and regular indexes.
    // FTS5 and sqlite-vec virtual tables are handled separately after migrations run.
    Migrations::new(vec![M::up(BASELINE_SQL)])
}

/// Ensure FTS5 tables and triggers exist (they can't be in migrations baseline)
fn ensure_fts_tables(conn: &Connection) -> Result<(), HyphaeError> {
    // Check if memories_fts exists
    let memories_fts_exists: bool = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='memories_fts'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

    if !memories_fts_exists {
        conn.execute_batch(
            "
            CREATE VIRTUAL TABLE memories_fts USING fts5(
                id,
                topic,
                summary,
                keywords,
                project UNINDEXED,
                content='memories',
                content_rowid='rowid'
            );

            CREATE TRIGGER memories_ai AFTER INSERT ON memories BEGIN
                INSERT INTO memories_fts(rowid, id, topic, summary, keywords, project)
                VALUES (new.rowid, new.id, new.topic, new.summary, new.keywords, new.project);
            END;

            CREATE TRIGGER memories_ad AFTER DELETE ON memories BEGIN
                INSERT INTO memories_fts(memories_fts, rowid, id, topic, summary, keywords, project)
                VALUES('delete', old.rowid, old.id, old.topic, old.summary, old.keywords, old.project);
            END;

            CREATE TRIGGER memories_au AFTER UPDATE ON memories BEGIN
                INSERT INTO memories_fts(memories_fts, rowid, id, topic, summary, keywords, project)
                VALUES('delete', old.rowid, old.id, old.topic, old.summary, old.keywords, old.project);
                INSERT INTO memories_fts(rowid, id, topic, summary, keywords, project)
                VALUES (new.rowid, new.id, new.topic, new.summary, new.keywords, new.project);
            END;
            ",
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;
    } else {
        // Check if memories_fts has project column, if not rebuild
        let has_fts_project: bool = conn
            .query_row(
                "SELECT COALESCE(sql, '') FROM sqlite_master WHERE type='table' AND name='memories_fts'",
                [],
                |r| r.get::<_, String>(0),
            )
            .map(|sql| sql.contains("project"))
            .unwrap_or(false);

        if !has_fts_project {
            // FTS5 tables cannot be ALTERed, so we must drop and recreate
            conn.execute_batch(
                "
                DROP TRIGGER IF EXISTS memories_ai;
                DROP TRIGGER IF EXISTS memories_ad;
                DROP TRIGGER IF EXISTS memories_au;
                DROP TABLE memories_fts;

                CREATE VIRTUAL TABLE memories_fts USING fts5(
                    id,
                    topic,
                    summary,
                    keywords,
                    project UNINDEXED,
                    content='memories',
                    content_rowid='rowid'
                );

                INSERT INTO memories_fts(rowid, id, topic, summary, keywords, project)
                SELECT rowid, id, topic, summary, keywords, project FROM memories;

                CREATE TRIGGER memories_ai AFTER INSERT ON memories BEGIN
                    INSERT INTO memories_fts(rowid, id, topic, summary, keywords, project)
                    VALUES (new.rowid, new.id, new.topic, new.summary, new.keywords, new.project);
                END;

                CREATE TRIGGER memories_ad AFTER DELETE ON memories BEGIN
                    INSERT INTO memories_fts(memories_fts, rowid, id, topic, summary, keywords, project)
                    VALUES('delete', old.rowid, old.id, old.topic, old.summary, old.keywords, old.project);
                END;

                CREATE TRIGGER memories_au AFTER UPDATE ON memories BEGIN
                    INSERT INTO memories_fts(memories_fts, rowid, id, topic, summary, keywords, project)
                    VALUES('delete', old.rowid, old.id, old.topic, old.summary, old.keywords, old.project);
                    INSERT INTO memories_fts(rowid, id, topic, summary, keywords, project)
                    VALUES (new.rowid, new.id, new.topic, new.summary, new.keywords, new.project);
                END;
                ",
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        }
    }

    // Check if concepts table exists (it may not in old databases)
    let concepts_table_exists: bool = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='concepts'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

    if concepts_table_exists {
        let concepts_fts_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='concepts_fts'",
                [],
                |row| row.get(0),
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        if !concepts_fts_exists {
            conn.execute_batch(
                "
                CREATE VIRTUAL TABLE concepts_fts USING fts5(
                    id,
                    name,
                    definition,
                    labels,
                    content='concepts',
                    content_rowid='rowid'
                );

                CREATE TRIGGER concepts_ai AFTER INSERT ON concepts BEGIN
                    INSERT INTO concepts_fts(rowid, id, name, definition, labels)
                    VALUES (new.rowid, new.id, new.name, new.definition, new.labels);
                END;

                CREATE TRIGGER concepts_ad AFTER DELETE ON concepts BEGIN
                    INSERT INTO concepts_fts(concepts_fts, rowid, id, name, definition, labels)
                    VALUES('delete', old.rowid, old.id, old.name, old.definition, old.labels);
                END;

                CREATE TRIGGER concepts_au AFTER UPDATE ON concepts BEGIN
                    INSERT INTO concepts_fts(concepts_fts, rowid, id, name, definition, labels)
                    VALUES('delete', old.rowid, old.id, old.name, old.definition, old.labels);
                    INSERT INTO concepts_fts(rowid, id, name, definition, labels)
                    VALUES (new.rowid, new.id, new.name, new.definition, new.labels);
                END;
                ",
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        }
    }

    // Check if reflexion_records table exists and create FTS if needed
    let reflexion_table_exists: bool = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='reflexion_records'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

    if reflexion_table_exists {
        let reflexion_fts_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='reflexion_fts'",
                [],
                |row| row.get(0),
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        if !reflexion_fts_exists {
            conn.execute_batch(
                "
                CREATE VIRTUAL TABLE reflexion_fts USING fts5(
                    id,
                    root_cause,
                    fix_applied,
                    abstract_pattern,
                    project UNINDEXED,
                    content='reflexion_records',
                    content_rowid='rowid'
                );

                CREATE TRIGGER reflexion_ai AFTER INSERT ON reflexion_records BEGIN
                    INSERT INTO reflexion_fts(rowid, id, root_cause, fix_applied, abstract_pattern, project)
                    VALUES (new.rowid, new.id, new.root_cause, new.fix_applied, new.abstract_pattern, new.project);
                END;

                CREATE TRIGGER reflexion_ad AFTER DELETE ON reflexion_records BEGIN
                    INSERT INTO reflexion_fts(reflexion_fts, rowid, id, root_cause, fix_applied, abstract_pattern, project)
                    VALUES('delete', old.rowid, old.id, old.root_cause, old.fix_applied, old.abstract_pattern, old.project);
                END;

                CREATE TRIGGER reflexion_au AFTER UPDATE ON reflexion_records BEGIN
                    INSERT INTO reflexion_fts(reflexion_fts, rowid, id, root_cause, fix_applied, abstract_pattern, project)
                    VALUES('delete', old.rowid, old.id, old.root_cause, old.fix_applied, old.abstract_pattern, old.project);
                    INSERT INTO reflexion_fts(rowid, id, root_cause, fix_applied, abstract_pattern, project)
                    VALUES (new.rowid, new.id, new.root_cause, new.fix_applied, new.abstract_pattern, new.project);
                END;
                ",
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        }
    }

    // Check if chunks table exists (it may not in old databases)
    let chunks_table_exists: bool = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='chunks'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

    if chunks_table_exists {
        let chunks_fts_exists: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='chunks_fts'",
                [],
                |row| row.get(0),
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        if !chunks_fts_exists {
            conn.execute_batch(
                "
                CREATE VIRTUAL TABLE chunks_fts USING fts5(
                    id UNINDEXED,
                    content,
                    source_path UNINDEXED,
                    heading
                );
                ",
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        }
    }

    Ok(())
}

/// Ensure all newer indexes exist
fn ensure_newer_indexes(conn: &Connection) -> Result<(), HyphaeError> {
    conn.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_memories_invalidated_at ON memories(invalidated_at);
         CREATE INDEX IF NOT EXISTS idx_memories_superseded_by ON memories(superseded_by);
         CREATE INDEX IF NOT EXISTS idx_memories_branch ON memories(branch);
         CREATE INDEX IF NOT EXISTS idx_memories_worktree ON memories(worktree);
         CREATE INDEX IF NOT EXISTS idx_memories_agent_id ON memories(agent_id);
         CREATE INDEX IF NOT EXISTS idx_memories_tier ON memories(tier);",
    )
    .map_err(|e| HyphaeError::Database(e.to_string()))?;

    Ok(())
}

/// Bootstrap an existing database by adding missing columns and stamping user_version if it has tables but version=0.
fn bootstrap_existing_db(conn: &mut Connection) -> Result<(), HyphaeError> {
    let user_version: i64 = conn
        .query_row("PRAGMA user_version", [], |r| r.get(0))
        .map_err(|e| HyphaeError::Database(e.to_string()))?;
    if user_version != 0 {
        return Ok(());
    }
    let table_exists: bool = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='memories'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|c| c > 0)
        .map_err(|e| HyphaeError::Database(e.to_string()))?;
    if !table_exists {
        // Fresh database: leave user_version at 0 so to_latest() runs M0.
        return Ok(());
    }

    // Add missing columns to memories that were introduced after the initial schema.
    // ALTER TABLE fails with "duplicate column name" if the column already exists;
    // that is the expected case on a fully up-to-date database, so we intentionally discard.
    // Column patches must run BEFORE execute_batch(BASELINE_SQL) because BASELINE_SQL includes
    // indexes on these columns; those indexes would fail if the columns don't exist yet.
    let _ = conn.execute("ALTER TABLE memories ADD COLUMN updated_at TEXT", []);
    let _ = conn.execute("ALTER TABLE memories ADD COLUMN embedding BLOB", []);
    let _ = conn.execute("ALTER TABLE memories ADD COLUMN project TEXT", []);
    let _ = conn.execute("ALTER TABLE memories ADD COLUMN branch TEXT", []);
    let _ = conn.execute("ALTER TABLE memories ADD COLUMN worktree TEXT", []);
    let _ = conn.execute("ALTER TABLE memories ADD COLUMN agent_id TEXT", []);
    let _ = conn.execute("ALTER TABLE memories ADD COLUMN expires_at TEXT", []);
    let _ = conn.execute("ALTER TABLE memories ADD COLUMN invalidated_at TEXT", []);
    let _ = conn.execute(
        "ALTER TABLE memories ADD COLUMN invalidation_reason TEXT",
        [],
    );
    let _ = conn.execute("ALTER TABLE memories ADD COLUMN superseded_by TEXT", []);
    let _ = conn.execute("ALTER TABLE memories ADD COLUMN tier TEXT", []);

    // Add tiered-memoir-content columns to concepts table
    let _ = conn.execute("ALTER TABLE concepts ADD COLUMN abstract_text TEXT", []);
    let _ = conn.execute("ALTER TABLE concepts ADD COLUMN overview_text TEXT", []);

    // Add block_type column to concepts table
    let _ = conn.execute(
        "ALTER TABLE concepts ADD COLUMN block_type TEXT NULL DEFAULT NULL",
        [],
    );

    // Add temporal validity columns to concept_links table
    let _ = conn.execute(
        "ALTER TABLE concept_links ADD COLUMN valid_from TEXT NOT NULL DEFAULT ''",
        [],
    );
    let _ = conn.execute("ALTER TABLE concept_links ADD COLUMN valid_to TEXT", []);

    // Run the full baseline SQL to create any tables added after the initial install.
    // All statements use IF NOT EXISTS — safe to run on any database state.
    conn.execute_batch(BASELINE_SQL)
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

    // Stamp user_version so rusqlite_migration skips M0 entirely.
    conn.execute_batch("PRAGMA user_version = 1;")
        .map_err(|e| HyphaeError::Database(e.to_string()))?;
    Ok(())
}

/// Initialize the database schema. `embedding_dims` controls the sqlite-vec vector size.
pub fn init_db(conn: &mut Connection) -> Result<(), HyphaeError> {
    init_db_with_dims(conn, 384)
}

pub fn init_db_with_dims(conn: &mut Connection, embedding_dims: usize) -> Result<(), HyphaeError> {
    bootstrap_existing_db(conn)?;

    // Run migrations (which includes M0 baseline that creates all tables if they don't exist)
    migrations()
        .to_latest(conn)
        .map_err(|e| HyphaeError::Database(format!("schema migration failed: {e}")))?;

    // Ensure FTS5 tables and triggers exist
    ensure_fts_tables(conn)?;

    // Ensure all newer indexes exist
    ensure_newer_indexes(conn)?;

    // sqlite-vec virtual tables (dimension-aware)
    let vec_memories_exists: bool = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='vec_memories'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

    if vec_memories_exists {
        // Check if stored dims differ from requested dims — if so, recreate
        let stored_dims: Option<String> = conn
            .query_row(
                "SELECT value FROM hyphae_metadata WHERE key = 'embedding_dims'",
                [],
                |row| row.get(0),
            )
            .ok();
        let stored: usize = stored_dims.and_then(|s| s.parse().ok()).unwrap_or(384);
        if stored != embedding_dims {
            // Model changed — drop vec table and clear embeddings
            conn.execute_batch("DROP TABLE IF EXISTS vec_memories")
                .map_err(|e| HyphaeError::Database(e.to_string()))?;
            conn.execute("UPDATE memories SET embedding = NULL", [])
                .map_err(|e| HyphaeError::Database(e.to_string()))?;
            conn.execute_batch(&format!(
                "CREATE VIRTUAL TABLE vec_memories USING vec0(
                    memory_id TEXT PRIMARY KEY,
                    embedding float[{embedding_dims}] distance_metric=cosine
                )"
            ))
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
            conn.execute(
                "INSERT OR REPLACE INTO hyphae_metadata (key, value) VALUES ('embedding_dims', ?1)",
                [&embedding_dims.to_string()],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        }
    } else {
        conn.execute_batch(&format!(
            "CREATE VIRTUAL TABLE vec_memories USING vec0(
                memory_id TEXT PRIMARY KEY,
                embedding float[{embedding_dims}] distance_metric=cosine
            )"
        ))
        .map_err(|e| HyphaeError::Database(e.to_string()))?;
        conn.execute(
            "INSERT OR REPLACE INTO hyphae_metadata (key, value) VALUES ('embedding_dims', ?1)",
            [&embedding_dims.to_string()],
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;
    }

    let vec_chunks_exists: bool = conn
        .query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='vec_chunks'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

    if !vec_chunks_exists {
        conn.execute_batch(&format!(
            "CREATE VIRTUAL TABLE vec_chunks USING vec0(
                chunk_id TEXT,
                embedding float[{embedding_dims}] distance_metric=cosine
            )"
        ))
        .map_err(|e| HyphaeError::Database(e.to_string()))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_helpers::ensure_vec_init;
    use rusqlite::Connection;

    #[test]
    fn test_init_db() {
        ensure_vec_init();
        let mut conn = Connection::open_in_memory().unwrap();
        init_db(&mut conn).unwrap();
        // Second call should be idempotent
        init_db(&mut conn).unwrap();
    }

    #[test]
    fn test_memoir_tables_exist() {
        ensure_vec_init();
        let mut conn = Connection::open_in_memory().unwrap();
        init_db(&mut conn).unwrap();

        // Verify all new tables exist
        let tables: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
                .unwrap();
            stmt.query_map([], |row| row.get(0))
                .unwrap()
                .map(|r| r.unwrap())
                .collect()
        };

        assert!(tables.contains(&"memoirs".to_string()));
        assert!(tables.contains(&"concepts".to_string()));
        assert!(tables.contains(&"concept_links".to_string()));
        assert!(tables.contains(&"concepts_fts".to_string()));
        assert!(tables.contains(&"vec_memories".to_string()));
        assert!(tables.contains(&"documents".to_string()));
        assert!(tables.contains(&"chunks".to_string()));
        assert!(tables.contains(&"chunks_fts".to_string()));
        assert!(tables.contains(&"vec_chunks".to_string()));
        assert!(tables.contains(&"recall_effectiveness".to_string()));
        assert!(tables.contains(&"shared_context".to_string()));
    }

    #[test]
    fn test_project_columns_exist() {
        ensure_vec_init();
        let mut conn = Connection::open_in_memory().unwrap();
        init_db(&mut conn).unwrap();

        let memories_has_project = conn.prepare("SELECT project FROM memories").is_ok();
        assert!(
            memories_has_project,
            "memories table should have project column"
        );

        let documents_has_project = conn.prepare("SELECT project FROM documents").is_ok();
        assert!(
            documents_has_project,
            "documents table should have project column"
        );
        let documents_has_runtime_session_id = conn
            .prepare("SELECT runtime_session_id FROM documents")
            .is_ok();
        assert!(
            documents_has_runtime_session_id,
            "documents table should have runtime_session_id column"
        );

        for column in ["project_root", "worktree_id", "scope", "runtime_session_id"] {
            let sessions_has_column = conn
                .prepare(&format!("SELECT {column} FROM sessions"))
                .is_ok();
            assert!(
                sessions_has_column,
                "sessions table should have {column} column"
            );
        }
    }

    #[test]
    fn test_init_db_idempotent() {
        ensure_vec_init();
        let mut conn = Connection::open_in_memory().unwrap();

        conn.execute_batch(
            "
            CREATE TABLE memories (
                id TEXT PRIMARY KEY,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL DEFAULT '',
                last_accessed TEXT NOT NULL,
                access_count INTEGER DEFAULT 0,
                weight REAL DEFAULT 1.0,
                topic TEXT NOT NULL,
                summary TEXT NOT NULL,
                raw_excerpt TEXT,
                keywords TEXT,
                importance TEXT NOT NULL,
                source_type TEXT NOT NULL,
                source_data TEXT,
                related_ids TEXT,
                embedding BLOB,
                project TEXT,
                expires_at TEXT
            );

            CREATE INDEX idx_memories_topic ON memories(topic);
            CREATE INDEX idx_memories_weight ON memories(weight);
            CREATE INDEX idx_memories_created ON memories(created_at);
            CREATE INDEX idx_memories_importance_weight ON memories(importance, weight);
            CREATE INDEX idx_memories_project ON memories(project);
            CREATE INDEX idx_memories_expires_at ON memories(expires_at);
            ",
        )
        .unwrap();

        init_db(&mut conn).unwrap();

        for column in [
            "project",
            "branch",
            "worktree",
            "expires_at",
            "invalidated_at",
            "invalidation_reason",
            "superseded_by",
            "agent_id",
            "tier",
        ] {
            let has_column = conn
                .prepare(&format!("SELECT {column} FROM memories"))
                .is_ok();
            assert!(has_column, "memories table should have {column} column");
        }
    }

    #[test]
    fn test_feedback_tables_have_session_foreign_keys() {
        ensure_vec_init();
        let mut conn = Connection::open_in_memory().unwrap();
        init_db(&mut conn).unwrap();

        let recall_fk: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0
                 FROM pragma_foreign_key_list('recall_events')
                 WHERE \"table\" = 'sessions' AND \"from\" = 'session_id'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            recall_fk,
            "recall_events.session_id should reference sessions.id"
        );

        let outcome_fk: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0
                 FROM pragma_foreign_key_list('outcome_signals')
                 WHERE \"table\" = 'sessions' AND \"from\" = 'session_id'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            outcome_fk,
            "outcome_signals.session_id should reference sessions.id"
        );

        let outcome_recall_fk: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0
                 FROM pragma_foreign_key_list('outcome_signals')
                 WHERE \"table\" = 'recall_events' AND \"from\" = 'recall_event_id'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            outcome_recall_fk,
            "outcome_signals.recall_event_id should reference recall_events.id"
        );
    }

    #[test]
    fn test_feedback_foreign_keys_set_null_on_session_delete() {
        ensure_vec_init();
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        init_db(&mut conn).unwrap();

        conn.execute(
            "INSERT INTO sessions (id, project, started_at, status) VALUES (?1, ?2, ?3, 'active')",
            ("ses_valid", "demo", "2026-03-27T00:00:00Z"),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO recall_events (id, session_id, query, recalled_at, memory_ids, memory_count, project)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (
                "rec_valid",
                "ses_valid",
                "query",
                "2026-03-27T00:00:00Z",
                "[]",
                0,
                "demo",
            ),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO outcome_signals (id, session_id, signal_type, signal_value, occurred_at, source, project)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (
                "sig_valid",
                "ses_valid",
                "session_success",
                2,
                "2026-03-27T00:00:00Z",
                "test",
                "demo",
            ),
        )
        .unwrap();

        conn.execute("DELETE FROM sessions WHERE id = 'ses_valid'", [])
            .unwrap();

        let recall_session_id: Option<String> = conn
            .query_row(
                "SELECT session_id FROM recall_events WHERE id = 'rec_valid'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(recall_session_id.is_none());

        let outcome_session_id: Option<String> = conn
            .query_row(
                "SELECT session_id FROM outcome_signals WHERE id = 'sig_valid'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(outcome_session_id.is_none());
    }

    #[test]
    fn test_feedback_recall_event_foreign_key_sets_null_on_recall_delete() {
        ensure_vec_init();
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        init_db(&mut conn).unwrap();

        conn.execute(
            "INSERT INTO sessions (id, project, started_at, status) VALUES (?1, ?2, ?3, 'active')",
            ("ses_valid", "demo", "2026-03-27T00:00:00Z"),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO recall_events (id, session_id, query, recalled_at, memory_ids, memory_count, project)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (
                "rec_valid",
                "ses_valid",
                "query",
                "2026-03-27T00:00:00Z",
                "[]",
                0,
                "demo",
            ),
        )
        .unwrap();
        conn.execute(
            "INSERT INTO outcome_signals (id, session_id, recall_event_id, signal_type, signal_value, occurred_at, source, project)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            (
                "sig_valid",
                "ses_valid",
                "rec_valid",
                "session_success",
                2,
                "2026-03-27T00:00:00Z",
                "test",
                "demo",
            ),
        )
        .unwrap();

        conn.execute("DELETE FROM recall_events WHERE id = 'rec_valid'", [])
            .unwrap();

        let recall_event_id: Option<String> = conn
            .query_row(
                "SELECT recall_event_id FROM outcome_signals WHERE id = 'sig_valid'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(recall_event_id.is_none());
    }

    #[test]
    fn test_feedback_foreign_keys_reject_new_invalid_session_ids() {
        ensure_vec_init();
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        init_db(&mut conn).unwrap();

        let recall_result = conn.execute(
            "INSERT INTO recall_events (id, session_id, query, recalled_at, memory_ids, memory_count, project)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (
                "rec_invalid",
                "ses_missing",
                "query",
                "2026-03-27T00:00:00Z",
                "[]",
                0,
                "demo",
            ),
        );
        assert!(recall_result.is_err());

        let outcome_result = conn.execute(
            "INSERT INTO outcome_signals (id, session_id, signal_type, signal_value, occurred_at, source, project)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            (
                "sig_invalid",
                "ses_missing",
                "session_failure",
                -2,
                "2026-03-27T00:00:00Z",
                "test",
                "demo",
            ),
        );
        assert!(outcome_result.is_err());

        conn.execute(
            "INSERT INTO sessions (id, project, started_at, status) VALUES (?1, ?2, ?3, 'active')",
            ("ses_valid", "demo", "2026-03-27T00:00:00Z"),
        )
        .unwrap();
        let outcome_recall_result = conn.execute(
            "INSERT INTO outcome_signals (id, session_id, recall_event_id, signal_type, signal_value, occurred_at, source, project)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            (
                "sig_invalid_recall",
                "ses_valid",
                "rec_missing",
                "session_success",
                2,
                "2026-03-27T00:00:00Z",
                "test",
                "demo",
            ),
        );
        assert!(outcome_recall_result.is_err());
    }
}
