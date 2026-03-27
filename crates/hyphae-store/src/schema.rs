use rusqlite::Connection;

use hyphae_core::HyphaeError;

/// Initialize the database schema. `embedding_dims` controls the sqlite-vec vector size.
/// Pass `None` to skip vector table creation (no embeddings feature).
pub fn init_db(conn: &Connection) -> Result<(), HyphaeError> {
    init_db_with_dims(conn, 384)
}

pub fn init_db_with_dims(conn: &Connection, embedding_dims: usize) -> Result<(), HyphaeError> {
    // SAFETY: No nested transactions — this is initialization code that does not call
    // other methods that open transactions. Called only once during database setup.
    let tx = conn.unchecked_transaction().map_err(|e| {
        HyphaeError::Database(format!("failed to start migration transaction: {e}"))
    })?;

    tx.execute_batch(
        "
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
            superseded_by TEXT
        );

        CREATE INDEX IF NOT EXISTS idx_memories_topic ON memories(topic);
        CREATE INDEX IF NOT EXISTS idx_memories_weight ON memories(weight);
        CREATE INDEX IF NOT EXISTS idx_memories_created ON memories(created_at);
        CREATE INDEX IF NOT EXISTS idx_memories_importance_weight ON memories(importance, weight);

        -- Memoir tables
        CREATE TABLE IF NOT EXISTS memoirs (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL UNIQUE,
            description TEXT NOT NULL DEFAULT '',
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            consolidation_threshold INTEGER NOT NULL DEFAULT 50
        );

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
            created_at TEXT NOT NULL,
            UNIQUE(source_id, target_id, relation),
            CHECK(source_id != target_id)
        );

        CREATE INDEX IF NOT EXISTS idx_concept_links_source ON concept_links(source_id);
        CREATE INDEX IF NOT EXISTS idx_concept_links_target ON concept_links(target_id);

        -- Session lifecycle tracking
        CREATE TABLE IF NOT EXISTS sessions (
            id TEXT PRIMARY KEY,
            project TEXT NOT NULL,
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

        -- RAG tables
        CREATE TABLE IF NOT EXISTS documents (
            id TEXT PRIMARY KEY,
            source_path TEXT NOT NULL UNIQUE,
            source_type TEXT NOT NULL,
            chunk_count INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
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
            created_at TEXT NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_chunks_document_id ON chunks(document_id);
        CREATE INDEX IF NOT EXISTS idx_chunks_source_path ON chunks(source_path);
        CREATE INDEX IF NOT EXISTS idx_documents_source_path ON documents(source_path);
        ",
    )
    .map_err(|e| HyphaeError::Database(e.to_string()))?;

    // ─────────────────────────────────────────────────────────────────────────
    // Check and migrate memories_fts table
    // ─────────────────────────────────────────────────────────────────────────
    let fts_exists: bool = tx
        .query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='memories_fts'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

    if !fts_exists {
        tx.execute_batch(
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
        // ─────────────────────────────────────────────────────────────────────
        // Migration: check if memories_fts has project column, if not rebuild
        // ─────────────────────────────────────────────────────────────────────
        let has_fts_project: bool = tx
            .prepare("SELECT COUNT(*) FROM pragma_table_info('memories_fts') WHERE name='project'")
            .and_then(|mut s| s.query_row([], |row| row.get(0)))
            .map_err(|e| HyphaeError::Database(e.to_string()))?;

        if !has_fts_project {
            // FTS5 tables cannot be ALTERed, so we must drop and recreate
            tx.execute_batch(
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

    // Check if concepts FTS table already exists
    let concepts_fts_exists: bool = tx
        .query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='concepts_fts'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

    if !concepts_fts_exists {
        tx.execute_batch(
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

    // Check if chunks FTS table already exists
    let chunks_fts_exists: bool = tx
        .query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='chunks_fts'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

    if !chunks_fts_exists {
        tx.execute_batch(
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

    // sqlite-vec virtual table for chunk embeddings (dimension-aware)
    let vec_chunks_exists: bool = tx
        .query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='vec_chunks'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

    if !vec_chunks_exists {
        tx.execute_batch(&format!(
            "CREATE VIRTUAL TABLE vec_chunks USING vec0(
                chunk_id TEXT,
                embedding float[{embedding_dims}] distance_metric=cosine
            )"
        ))
        .map_err(|e| HyphaeError::Database(e.to_string()))?;
    }

    // Metadata key-value table for internal state (e.g. last_decay_at)
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS hyphae_metadata (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL
        );",
    )
    .map_err(|e| HyphaeError::Database(e.to_string()))?;

    // Project links table for cross-project relationships
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS project_links (
            source_project TEXT NOT NULL,
            target_project TEXT NOT NULL,
            created_at TEXT NOT NULL,
            PRIMARY KEY (source_project, target_project),
            CHECK(source_project != target_project)
        );",
    )
    .map_err(|e| HyphaeError::Database(e.to_string()))?;

    // Migration: add updated_at column if missing (existing DBs pre-0.3.1)
    let has_updated_at: bool = tx
        .prepare("SELECT COUNT(*) FROM pragma_table_info('memories') WHERE name='updated_at'")
        .and_then(|mut s| s.query_row([], |row| row.get(0)))
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

    if !has_updated_at {
        tx.execute_batch(
            "ALTER TABLE memories ADD COLUMN updated_at TEXT;
             UPDATE memories SET updated_at = created_at WHERE updated_at IS NULL;",
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;
    }

    // Migration: add embedding column if missing (existing DBs)
    let has_embedding: bool = tx
        .prepare("SELECT COUNT(*) FROM pragma_table_info('memories') WHERE name='embedding'")
        .and_then(|mut s| s.query_row([], |row| row.get(0)))
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

    if !has_embedding {
        tx.execute_batch("ALTER TABLE memories ADD COLUMN embedding BLOB")
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
    }

    // Migration: add project column to memories
    let has_project_memories: bool = tx
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('memories') WHERE name='project'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0)
        > 0;
    if !has_project_memories {
        tx.execute_batch("ALTER TABLE memories ADD COLUMN project TEXT;")
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
    }
    tx.execute_batch("CREATE INDEX IF NOT EXISTS idx_memories_project ON memories(project);")
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

    let has_branch_memories: bool = tx
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('memories') WHERE name='branch'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0)
        > 0;
    if !has_branch_memories {
        tx.execute_batch("ALTER TABLE memories ADD COLUMN branch TEXT;")
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
    }
    tx.execute_batch("CREATE INDEX IF NOT EXISTS idx_memories_branch ON memories(branch);")
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

    let has_worktree_memories: bool = tx
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('memories') WHERE name='worktree'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0)
        > 0;
    if !has_worktree_memories {
        tx.execute_batch("ALTER TABLE memories ADD COLUMN worktree TEXT;")
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
    }
    tx.execute_batch("CREATE INDEX IF NOT EXISTS idx_memories_worktree ON memories(worktree);")
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

    // Migration: add project column to documents
    let has_project_documents: bool = tx
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('documents') WHERE name='project'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0)
        > 0;
    if !has_project_documents {
        tx.execute_batch(
            "ALTER TABLE documents ADD COLUMN project TEXT;
             CREATE INDEX IF NOT EXISTS idx_documents_project ON documents(project);",
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;
    }

    // Migration: add expires_at column to memories
    let has_expires_at: bool = tx
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('memories') WHERE name='expires_at'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0)
        > 0;
    if !has_expires_at {
        tx.execute_batch("ALTER TABLE memories ADD COLUMN expires_at TEXT;")
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
    }
    tx.execute_batch("CREATE INDEX IF NOT EXISTS idx_memories_expires_at ON memories(expires_at);")
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

    let has_invalidated_at: bool = tx
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('memories') WHERE name='invalidated_at'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0)
        > 0;
    if !has_invalidated_at {
        tx.execute_batch("ALTER TABLE memories ADD COLUMN invalidated_at TEXT;")
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
    }
    tx.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_memories_invalidated_at ON memories(invalidated_at);",
    )
    .map_err(|e| HyphaeError::Database(e.to_string()))?;

    let has_invalidation_reason: bool = tx
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('memories') WHERE name='invalidation_reason'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0)
        > 0;
    if !has_invalidation_reason {
        tx.execute_batch("ALTER TABLE memories ADD COLUMN invalidation_reason TEXT")
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
    }

    let has_superseded_by: bool = tx
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('memories') WHERE name='superseded_by'",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0)
        > 0;
    if !has_superseded_by {
        tx.execute_batch("ALTER TABLE memories ADD COLUMN superseded_by TEXT;")
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
    }
    tx.execute_batch(
        "CREATE INDEX IF NOT EXISTS idx_memories_superseded_by ON memories(superseded_by);",
    )
    .map_err(|e| HyphaeError::Database(e.to_string()))?;

    // sqlite-vec virtual table for vector search (dimension-aware)
    let vec_exists: bool = tx
        .query_row(
            "SELECT COUNT(*) > 0 FROM sqlite_master WHERE type='table' AND name='vec_memories'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

    if vec_exists {
        // Check if stored dims differ from requested dims — if so, recreate
        let stored_dims: Option<String> = tx
            .query_row(
                "SELECT value FROM hyphae_metadata WHERE key = 'embedding_dims'",
                [],
                |row| row.get(0),
            )
            .ok();
        let stored: usize = stored_dims.and_then(|s| s.parse().ok()).unwrap_or(384);
        if stored != embedding_dims {
            // Model changed — drop vec table and clear embeddings
            tx.execute_batch("DROP TABLE IF EXISTS vec_memories")
                .map_err(|e| HyphaeError::Database(e.to_string()))?;
            tx.execute("UPDATE memories SET embedding = NULL", [])
                .map_err(|e| HyphaeError::Database(e.to_string()))?;
            tx.execute_batch(&format!(
                "CREATE VIRTUAL TABLE vec_memories USING vec0(
                    memory_id TEXT PRIMARY KEY,
                    embedding float[{embedding_dims}] distance_metric=cosine
                )"
            ))
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
            tx.execute(
                "INSERT OR REPLACE INTO hyphae_metadata (key, value) VALUES ('embedding_dims', ?1)",
                [&embedding_dims.to_string()],
            )
            .map_err(|e| HyphaeError::Database(e.to_string()))?;
        }
    } else {
        tx.execute_batch(&format!(
            "CREATE VIRTUAL TABLE vec_memories USING vec0(
                memory_id TEXT PRIMARY KEY,
                embedding float[{embedding_dims}] distance_metric=cosine
            )"
        ))
        .map_err(|e| HyphaeError::Database(e.to_string()))?;
        tx.execute(
            "INSERT OR REPLACE INTO hyphae_metadata (key, value) VALUES ('embedding_dims', ?1)",
            [&embedding_dims.to_string()],
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;
    }

    let recall_events_has_session_fk: bool = tx
        .query_row(
            "SELECT COUNT(*) > 0
             FROM pragma_foreign_key_list('recall_events')
             WHERE \"table\" = 'sessions' AND \"from\" = 'session_id'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

    if !recall_events_has_session_fk {
        tx.execute_batch(
            "
            ALTER TABLE recall_events RENAME TO recall_events_old;

            CREATE TABLE recall_events (
                id TEXT PRIMARY KEY,
                session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL,
                query TEXT NOT NULL,
                recalled_at TEXT NOT NULL,
                memory_ids TEXT NOT NULL,
                memory_count INTEGER NOT NULL,
                project TEXT
            );

            INSERT INTO recall_events (id, session_id, query, recalled_at, memory_ids, memory_count, project)
            SELECT
                re.id,
                CASE
                    WHEN re.session_id IS NULL THEN NULL
                    WHEN EXISTS (SELECT 1 FROM sessions s WHERE s.id = re.session_id) THEN re.session_id
                    ELSE NULL
                END,
                re.query,
                re.recalled_at,
                re.memory_ids,
                re.memory_count,
                re.project
            FROM recall_events_old re;

            DROP TABLE recall_events_old;

            CREATE INDEX IF NOT EXISTS idx_recall_events_session
                ON recall_events(session_id);
            CREATE INDEX IF NOT EXISTS idx_recall_events_recalled_at
                ON recall_events(recalled_at);
            ",
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;
    }

    let outcome_signals_has_session_fk: bool = tx
        .query_row(
            "SELECT COUNT(*) > 0
             FROM pragma_foreign_key_list('outcome_signals')
             WHERE \"table\" = 'sessions' AND \"from\" = 'session_id'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;

    if !outcome_signals_has_session_fk {
        tx.execute_batch(
            "
            ALTER TABLE outcome_signals RENAME TO outcome_signals_old;

            CREATE TABLE outcome_signals (
                id TEXT PRIMARY KEY,
                session_id TEXT REFERENCES sessions(id) ON DELETE SET NULL,
                signal_type TEXT NOT NULL,
                signal_value INTEGER NOT NULL,
                occurred_at TEXT NOT NULL,
                source TEXT,
                project TEXT
            );

            INSERT INTO outcome_signals (id, session_id, signal_type, signal_value, occurred_at, source, project)
            SELECT
                os.id,
                CASE
                    WHEN os.session_id IS NULL THEN NULL
                    WHEN EXISTS (SELECT 1 FROM sessions s WHERE s.id = os.session_id) THEN os.session_id
                    ELSE NULL
                END,
                os.signal_type,
                os.signal_value,
                os.occurred_at,
                os.source,
                os.project
            FROM outcome_signals_old os;

            DROP TABLE outcome_signals_old;

            CREATE INDEX IF NOT EXISTS idx_outcome_signals_session
                ON outcome_signals(session_id);
            CREATE INDEX IF NOT EXISTS idx_outcome_signals_occurred_at
                ON outcome_signals(occurred_at);
            ",
        )
        .map_err(|e| HyphaeError::Database(e.to_string()))?;
    }

    tx.commit().map_err(|e| {
        HyphaeError::Database(format!("failed to commit migration transaction: {e}"))
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_helpers::ensure_vec_init;

    #[test]
    fn test_init_db() {
        ensure_vec_init();
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();
        // Second call should be idempotent
        init_db(&conn).unwrap();
    }

    #[test]
    fn test_memoir_tables_exist() {
        ensure_vec_init();
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();

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
    }

    #[test]
    fn test_project_columns_exist() {
        ensure_vec_init();
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();

        let memories_has_project: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('memories') WHERE name='project'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0)
            > 0;
        assert!(
            memories_has_project,
            "memories table should have project column"
        );

        let documents_has_project: bool = conn
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('documents') WHERE name='project'",
                [],
                |row| row.get(0),
            )
            .unwrap_or(0)
            > 0;
        assert!(
            documents_has_project,
            "documents table should have project column"
        );
    }

    #[test]
    fn test_init_db_migrates_older_memories_schema_before_creating_new_indexes() {
        ensure_vec_init();
        let conn = Connection::open_in_memory().unwrap();

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

        init_db(&conn).unwrap();

        for column in [
            "project",
            "branch",
            "worktree",
            "expires_at",
            "invalidated_at",
            "invalidation_reason",
            "superseded_by",
        ] {
            let has_column: bool = conn
                .query_row(
                    "SELECT COUNT(*) FROM pragma_table_info('memories') WHERE name = ?1",
                    [column],
                    |row| row.get(0),
                )
                .unwrap_or(0)
                > 0;
            assert!(has_column, "memories table should have {column} column");
        }
    }

    #[test]
    fn test_feedback_tables_have_session_foreign_keys() {
        ensure_vec_init();
        let conn = Connection::open_in_memory().unwrap();
        init_db(&conn).unwrap();

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
    }

    #[test]
    fn test_init_db_migrates_feedback_tables_to_add_session_foreign_keys() {
        ensure_vec_init();
        let conn = Connection::open_in_memory().unwrap();

        conn.execute_batch(
            "
            PRAGMA foreign_keys=ON;

            CREATE TABLE sessions (
                id TEXT PRIMARY KEY,
                project TEXT NOT NULL,
                task TEXT,
                started_at TEXT NOT NULL,
                ended_at TEXT,
                summary TEXT,
                files_modified TEXT,
                errors TEXT,
                status TEXT NOT NULL DEFAULT 'active'
            );

            CREATE TABLE recall_events (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                query TEXT NOT NULL,
                recalled_at TEXT NOT NULL,
                memory_ids TEXT NOT NULL,
                memory_count INTEGER NOT NULL,
                project TEXT
            );

            CREATE TABLE outcome_signals (
                id TEXT PRIMARY KEY,
                session_id TEXT,
                signal_type TEXT NOT NULL,
                signal_value INTEGER NOT NULL,
                occurred_at TEXT NOT NULL,
                source TEXT,
                project TEXT
            );

            INSERT INTO sessions (id, project, started_at, status)
            VALUES ('ses_valid', 'demo', '2026-03-27T00:00:00Z', 'active');

            INSERT INTO recall_events (id, session_id, query, recalled_at, memory_ids, memory_count, project)
            VALUES
                ('rec_valid', 'ses_valid', 'query', '2026-03-27T00:00:00Z', '[]', 0, 'demo'),
                ('rec_invalid', 'ses_missing', 'query', '2026-03-27T00:00:00Z', '[]', 0, 'demo');

            INSERT INTO outcome_signals (id, session_id, signal_type, signal_value, occurred_at, source, project)
            VALUES
                ('sig_valid', 'ses_valid', 'session_success', 2, '2026-03-27T00:00:00Z', 'test', 'demo'),
                ('sig_invalid', 'ses_missing', 'session_failure', -2, '2026-03-27T00:00:00Z', 'test', 'demo');
            ",
        )
        .unwrap();

        init_db(&conn).unwrap();

        let recall_invalid_session: Option<String> = conn
            .query_row(
                "SELECT session_id FROM recall_events WHERE id = 'rec_invalid'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(recall_invalid_session.is_none());

        let outcome_invalid_session: Option<String> = conn
            .query_row(
                "SELECT session_id FROM outcome_signals WHERE id = 'sig_invalid'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(outcome_invalid_session.is_none());

        let valid_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM recall_events WHERE session_id = 'ses_valid'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(valid_count, 1);
    }

    #[test]
    fn test_feedback_foreign_keys_set_null_on_session_delete() {
        ensure_vec_init();
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        init_db(&conn).unwrap();

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
    fn test_feedback_foreign_keys_reject_new_invalid_session_ids() {
        ensure_vec_init();
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        init_db(&conn).unwrap();

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
    }
}
