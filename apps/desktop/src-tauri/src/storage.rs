use std::{collections::HashSet, path::Path, sync::Mutex};

use rusqlite::{params, Connection, OptionalExtension};
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

use crate::contracts::{
    CharacterSettings, ChatMessage, DataSummary, MemoryCandidate, MemoryItem, MemorySettings,
    MessageRole, MessageStatus, PetSettings, ProviderSettings, RelationshipState, TimerHistoryItem,
    TimerItem, ToolRunSummary, VoiceSettings,
};

pub struct Storage {
    connection: Mutex<Connection>,
}

pub struct ToolRunInput<'a> {
    pub id: &'a str,
    pub conversation_id: &'a str,
    pub turn_id: &'a str,
    pub tool_name: &'a str,
    pub arguments_json: &'a str,
    pub origin: &'a str,
}

pub struct TimerInput<'a> {
    pub id: &'a str,
    pub conversation_id: &'a str,
    pub tool_run_id: &'a str,
    pub title: &'a str,
    pub due_at_ms: i64,
    pub timezone: &'a str,
    pub original_request: &'a str,
}

const RELATIONSHIP_FAMILIARITY_STEP: f64 = 0.02;

#[allow(dead_code)]
pub struct MemoryCandidateInput<'a> {
    pub id: &'a str,
    pub source_turn_id: &'a str,
    pub source_message_id: Option<&'a str>,
    pub scope: &'a str,
    pub kind: &'a str,
    pub content: &'a str,
    pub confidence: f64,
    pub sensitivity: &'a str,
    pub created_at_ms: i64,
}

pub struct MemoryCandidateV2Input<'a> {
    pub id: &'a str,
    pub source_turn_id: &'a str,
    pub source_message_id: Option<&'a str>,
    pub scope: &'a str,
    pub kind: &'a str,
    pub content: &'a str,
    pub confidence: f64,
    pub sensitivity: &'a str,
    pub origin: &'a str,
    pub reason: &'a str,
    pub fact_key: Option<&'a str>,
    pub proposed_action: &'a str,
    pub expires_at_ms: Option<i64>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryConflictResolution {
    Replace,
    KeepBoth,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MemoryConfirmResult {
    Saved(MemoryItem),
    Duplicate(MemoryItem),
    Conflict(MemoryItem),
}

const MEMORY_CANDIDATE_RETENTION_MS: i64 = 30 * 24 * 60 * 60 * 1_000;
const MEMORY_SELECT_COLUMNS: &str =
    "m.id, m.scope, m.character_id, m.kind, m.content, m.importance,
    m.confidence, m.sensitivity, m.origin, m.fact_key, m.source_turn_id,
    m.source_message_id, m.valid_from_ms, m.valid_until_ms, m.last_confirmed_at_ms,
    m.version, m.created_at_ms, m.updated_at_ms, m.last_used_at_ms";

const MEMORY_CANDIDATE_SELECT: &str = r#"
    SELECT c.id, c.scope, c.kind, c.content, c.confidence, c.sensitivity, c.status,
           c.created_at_ms, c.reviewed_at_ms,
           c.origin, c.reason, c.fact_key, c.proposed_action, c.expires_at_ms,
           (SELECT m.id
              FROM memories m
             WHERE m.scope=c.scope
               AND ((c.scope='user' AND m.character_id IS NULL)
                 OR (c.scope='relationship' AND m.character_id=source_conversation.character_id))
               AND m.normalized_key=c.normalized_key
               AND NOT EXISTS (SELECT 1 FROM memories newer WHERE newer.supersedes_id=m.id)
             ORDER BY m.updated_at_ms DESC, m.id DESC
             LIMIT 1) AS memory_duplicate_id,
           (SELECT m.content
              FROM memories m
             WHERE m.scope=c.scope
               AND ((c.scope='user' AND m.character_id IS NULL)
                 OR (c.scope='relationship' AND m.character_id=source_conversation.character_id))
               AND m.normalized_key=c.normalized_key
               AND NOT EXISTS (SELECT 1 FROM memories newer WHERE newer.supersedes_id=m.id)
             ORDER BY m.updated_at_ms DESC, m.id DESC
             LIMIT 1) AS memory_duplicate_content,
           (SELECT other.id
              FROM memory_candidates other
              JOIN turns other_turn ON other_turn.id=other.source_turn_id
              JOIN conversations other_conversation ON other_conversation.id=other_turn.conversation_id
             WHERE other.id <> c.id
               AND other.status='pending'
               AND other.scope=c.scope
               AND (c.scope <> 'relationship' OR other_conversation.character_id=source_conversation.character_id)
               AND other.normalized_key=c.normalized_key
               AND (other.created_at_ms < c.created_at_ms
                    OR (other.created_at_ms = c.created_at_ms AND other.id < c.id))
             ORDER BY other.created_at_ms ASC, other.id ASC
             LIMIT 1) AS candidate_duplicate_id,
           (SELECT other.content
              FROM memory_candidates other
              JOIN turns other_turn ON other_turn.id=other.source_turn_id
              JOIN conversations other_conversation ON other_conversation.id=other_turn.conversation_id
             WHERE other.id <> c.id
               AND other.status='pending'
               AND other.scope=c.scope
               AND (c.scope <> 'relationship' OR other_conversation.character_id=source_conversation.character_id)
               AND other.normalized_key=c.normalized_key
               AND (other.created_at_ms < c.created_at_ms
                    OR (other.created_at_ms = c.created_at_ms AND other.id < c.id))
             ORDER BY other.created_at_ms ASC, other.id ASC
             LIMIT 1) AS candidate_duplicate_content
           ,(SELECT m.id
              FROM memories m
             WHERE c.fact_key IS NOT NULL
               AND c.fact_key <> ''
               AND m.scope=c.scope
               AND ((c.scope='user' AND m.character_id IS NULL)
                 OR (c.scope='relationship' AND m.character_id=source_conversation.character_id))
               AND m.fact_key=c.fact_key
               AND m.normalized_key<>c.normalized_key
               AND (m.valid_until_ms IS NULL OR m.valid_until_ms > CAST(strftime('%s','now') AS INTEGER) * 1000)
               AND NOT EXISTS (SELECT 1 FROM memories newer WHERE newer.supersedes_id=m.id)
             ORDER BY m.updated_at_ms DESC, m.id DESC
             LIMIT 1) AS conflict_memory_id
           ,(SELECT m.content
              FROM memories m
             WHERE c.fact_key IS NOT NULL
               AND c.fact_key <> ''
               AND m.scope=c.scope
               AND ((c.scope='user' AND m.character_id IS NULL)
                 OR (c.scope='relationship' AND m.character_id=source_conversation.character_id))
               AND m.fact_key=c.fact_key
               AND m.normalized_key<>c.normalized_key
               AND (m.valid_until_ms IS NULL OR m.valid_until_ms > CAST(strftime('%s','now') AS INTEGER) * 1000)
               AND NOT EXISTS (SELECT 1 FROM memories newer WHERE newer.supersedes_id=m.id)
             ORDER BY m.updated_at_ms DESC, m.id DESC
             LIMIT 1) AS conflict_memory_content
      FROM memory_candidates c
      JOIN turns source_turn ON source_turn.id=c.source_turn_id
      JOIN conversations source_conversation ON source_conversation.id=source_turn.conversation_id"#;

impl Storage {
    pub fn open(path: &Path) -> Result<Self, rusqlite::Error> {
        let mut connection = Connection::open(path)?;
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA busy_timeout = 5000;
             PRAGMA trusted_schema = OFF;",
        )?;
        Self::migrate(&mut connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self, rusqlite::Error> {
        let mut connection = Connection::open_in_memory()?;
        connection.execute_batch("PRAGMA foreign_keys = ON; PRAGMA trusted_schema = OFF;")?;
        Self::migrate(&mut connection)?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    fn migrate(connection: &mut Connection) -> Result<(), rusqlite::Error> {
        connection.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
               version INTEGER PRIMARY KEY,
               name TEXT NOT NULL,
               applied_at_ms INTEGER NOT NULL
             );",
        )?;
        let current_version: i64 = connection.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )?;
        if current_version > 5 {
            return Err(rusqlite::Error::InvalidQuery);
        }
        if current_version == 0 {
            let transaction = connection.transaction()?;
            transaction.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
               version INTEGER PRIMARY KEY,
               name TEXT NOT NULL,
               applied_at_ms INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS conversations (
               id TEXT PRIMARY KEY,
               title TEXT,
               character_id TEXT NOT NULL,
               created_at_ms INTEGER NOT NULL,
               updated_at_ms INTEGER NOT NULL,
               archived_at_ms INTEGER
             );
             CREATE TABLE IF NOT EXISTS turns (
               id TEXT PRIMARY KEY,
               conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
               generation INTEGER NOT NULL,
               input_source TEXT NOT NULL CHECK(input_source IN ('text','wake_word','push_to_talk')),
               status TEXT NOT NULL CHECK(status IN ('preparing','active','finalizing','completed','cancelled','failed')),
               failure_code TEXT,
               started_at_ms INTEGER NOT NULL,
               ended_at_ms INTEGER,
               UNIQUE(conversation_id, generation)
             );
             CREATE TABLE IF NOT EXISTS messages (
               id TEXT PRIMARY KEY,
               conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
               turn_id TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
               ordinal INTEGER NOT NULL,
               role TEXT NOT NULL CHECK(role IN ('user','assistant','system','tool')),
               canonical_text TEXT NOT NULL,
               display_text TEXT,
               spoken_text TEXT,
               status TEXT NOT NULL CHECK(status IN ('pending','streaming','completed','cancelled','failed')),
               created_at_ms INTEGER NOT NULL,
               completed_at_ms INTEGER,
               UNIQUE(turn_id, ordinal)
             );
             CREATE INDEX IF NOT EXISTS idx_messages_conversation_time
               ON messages(conversation_id, created_at_ms);
             CREATE TABLE IF NOT EXISTS memory_candidates (
               id TEXT PRIMARY KEY,
               source_turn_id TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
               source_message_id TEXT REFERENCES messages(id) ON DELETE SET NULL,
               scope TEXT NOT NULL CHECK(scope IN ('user','relationship')),
               kind TEXT NOT NULL,
               content TEXT NOT NULL,
               normalized_key TEXT NOT NULL,
               confidence REAL NOT NULL CHECK(confidence >= 0 AND confidence <= 1),
               sensitivity TEXT NOT NULL CHECK(sensitivity IN ('normal','sensitive','restricted')),
               status TEXT NOT NULL CHECK(status IN ('pending','confirmed','rejected','expired')),
               created_at_ms INTEGER NOT NULL,
               reviewed_at_ms INTEGER
             );
             CREATE TABLE IF NOT EXISTS memories (
               id TEXT PRIMARY KEY,
               scope TEXT NOT NULL CHECK(scope IN ('user','relationship')),
               character_id TEXT,
               kind TEXT NOT NULL,
               content TEXT NOT NULL,
               normalized_key TEXT NOT NULL,
               importance REAL NOT NULL DEFAULT 0.5 CHECK(importance >= 0 AND importance <= 1),
               confidence REAL NOT NULL CHECK(confidence >= 0 AND confidence <= 1),
               sensitivity TEXT NOT NULL CHECK(sensitivity IN ('normal','sensitive','restricted')),
               source_message_id TEXT REFERENCES messages(id) ON DELETE SET NULL,
               source_candidate_id TEXT UNIQUE REFERENCES memory_candidates(id) ON DELETE SET NULL,
               supersedes_id TEXT REFERENCES memories(id) ON DELETE SET NULL,
               version INTEGER NOT NULL DEFAULT 1,
               created_at_ms INTEGER NOT NULL,
               updated_at_ms INTEGER NOT NULL,
               last_used_at_ms INTEGER,
               CHECK((scope = 'user' AND character_id IS NULL) OR (scope = 'relationship' AND character_id IS NOT NULL))
             );
             CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(
               content,
               normalized_key,
               content='memories',
               content_rowid='rowid',
               tokenize='trigram'
             );
             CREATE TABLE IF NOT EXISTS relationship_state (
               character_id TEXT PRIMARY KEY,
               affinity REAL NOT NULL DEFAULT 0 CHECK(affinity >= -1 AND affinity <= 1),
               familiarity REAL NOT NULL DEFAULT 0 CHECK(familiarity >= 0 AND familiarity <= 1),
               interaction_count INTEGER NOT NULL DEFAULT 0,
               summary TEXT NOT NULL DEFAULT '',
               version INTEGER NOT NULL DEFAULT 1,
               updated_at_ms INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS app_settings (
               key TEXT PRIMARY KEY,
               value_json TEXT NOT NULL,
               revision INTEGER NOT NULL,
               updated_at_ms INTEGER NOT NULL
             );",
        )?;
            transaction.execute(
                "INSERT INTO schema_migrations(version, name, applied_at_ms) VALUES(1, 'initial_schema', CAST(strftime('%s','now') AS INTEGER) * 1000)",
                [],
            )?;
            transaction.commit()?;
        }
        if current_version < 2 {
            let transaction = connection.transaction()?;
            transaction.execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_memory_candidates_status
                   ON memory_candidates(status, created_at_ms);
                 CREATE TABLE IF NOT EXISTS memory_audit (
                   id TEXT PRIMARY KEY,
                   memory_id TEXT NOT NULL,
                   action TEXT NOT NULL CHECK(action IN ('confirmed','updated','deleted')),
                   occurred_at_ms INTEGER NOT NULL
                 );
                 INSERT INTO memory_fts(memory_fts) VALUES('rebuild');",
            )?;
            transaction.execute(
                "INSERT INTO schema_migrations(version, name, applied_at_ms) VALUES(2, 'memory_repository', CAST(strftime('%s','now') AS INTEGER) * 1000)",
                [],
            )?;
            transaction.commit()?;
        }
        if current_version < 3 {
            let transaction = connection.transaction()?;
            transaction.execute_batch(
                "CREATE TABLE IF NOT EXISTS tool_runs (
                   id TEXT PRIMARY KEY,
                   conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                   turn_id TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
                   tool_name TEXT NOT NULL,
                   arguments_json TEXT NOT NULL,
                   origin TEXT NOT NULL CHECK(origin IN ('user_explicit','model_suggested','scheduled')),
                   status TEXT NOT NULL CHECK(status IN ('pending','approved','rejected','succeeded','failed','cancelled')),
                   result_summary TEXT,
                   created_at_ms INTEGER NOT NULL,
                   completed_at_ms INTEGER
                 );
                 CREATE INDEX IF NOT EXISTS idx_tool_runs_conversation_time
                   ON tool_runs(conversation_id, created_at_ms);",
            )?;
            transaction.execute(
                "INSERT INTO schema_migrations(version, name, applied_at_ms) VALUES(3, 'tool_runtime_audit', CAST(strftime('%s','now') AS INTEGER) * 1000)",
                [],
            )?;
            transaction.commit()?;
        }
        if current_version < 4 {
            let transaction = connection.transaction()?;
            transaction.execute_batch(
                "CREATE TABLE IF NOT EXISTS timers (
                   id TEXT PRIMARY KEY,
                   conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
                   tool_run_id TEXT UNIQUE REFERENCES tool_runs(id) ON DELETE SET NULL,
                   title TEXT NOT NULL,
                   due_at_ms INTEGER NOT NULL,
                   timezone TEXT NOT NULL,
                   original_request TEXT NOT NULL,
                   status TEXT NOT NULL CHECK(status IN ('pending','fired','cancelled')),
                   created_at_ms INTEGER NOT NULL,
                   updated_at_ms INTEGER NOT NULL,
                   fired_at_ms INTEGER,
                   cancelled_at_ms INTEGER
                 );
                 CREATE INDEX IF NOT EXISTS idx_timers_pending_due
                   ON timers(status, due_at_ms);",
            )?;
            transaction.execute(
                "INSERT INTO schema_migrations(version, name, applied_at_ms) VALUES(4, 'timers', CAST(strftime('%s','now') AS INTEGER) * 1000)",
                [],
            )?;
            transaction.commit()?;
        }
        if current_version < 5 {
            let transaction = connection.transaction()?;
            transaction.execute_batch(
                "ALTER TABLE memory_candidates ADD COLUMN origin TEXT NOT NULL DEFAULT 'automatic_candidate';
                 ALTER TABLE memory_candidates ADD COLUMN reason TEXT NOT NULL DEFAULT '历史候选，等待老师确认。';
                 ALTER TABLE memory_candidates ADD COLUMN fact_key TEXT;
                 ALTER TABLE memory_candidates ADD COLUMN proposed_action TEXT NOT NULL DEFAULT 'create';
                 ALTER TABLE memory_candidates ADD COLUMN expires_at_ms INTEGER;
                 ALTER TABLE memories ADD COLUMN origin TEXT NOT NULL DEFAULT 'user_approved_candidate';
                 ALTER TABLE memories ADD COLUMN fact_key TEXT;
                 ALTER TABLE memories ADD COLUMN source_turn_id TEXT;
                 ALTER TABLE memories ADD COLUMN valid_from_ms INTEGER;
                 ALTER TABLE memories ADD COLUMN valid_until_ms INTEGER;
                 ALTER TABLE memories ADD COLUMN last_confirmed_at_ms INTEGER;
                 ALTER TABLE memory_audit ADD COLUMN actor TEXT NOT NULL DEFAULT 'system';
                 ALTER TABLE memory_audit ADD COLUMN candidate_id TEXT;
                 ALTER TABLE memory_audit ADD COLUMN previous_version INTEGER;
                 ALTER TABLE memory_audit ADD COLUMN replacement_id TEXT;
                 CREATE INDEX IF NOT EXISTS idx_memories_fact_key
                   ON memories(scope, character_id, fact_key, updated_at_ms);
                 CREATE INDEX IF NOT EXISTS idx_memories_expiry
                   ON memories(valid_until_ms, updated_at_ms);",
            )?;
            transaction.execute(
                "UPDATE memories
                    SET source_turn_id=(SELECT turn_id FROM messages WHERE messages.id=memories.source_message_id)
                  WHERE source_turn_id IS NULL AND source_message_id IS NOT NULL",
                [],
            )?;
            transaction.execute(
                "UPDATE memory_candidates
                    SET expires_at_ms=created_at_ms + ?1
                  WHERE expires_at_ms IS NULL AND status='pending'",
                [MEMORY_CANDIDATE_RETENTION_MS],
            )?;
            transaction.execute(
                "INSERT INTO schema_migrations(version, name, applied_at_ms) VALUES(5, 'memory_v2_metadata', CAST(strftime('%s','now') AS INTEGER) * 1000)",
                [],
            )?;
            transaction.commit()?;
        }
        Ok(())
    }

    pub fn ensure_conversation(&self, now_ms: i64) -> Result<String, rusqlite::Error> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        if let Some(id) = connection
            .query_row(
                "SELECT id FROM conversations WHERE archived_at_ms IS NULL ORDER BY updated_at_ms DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?
        {
            return Ok(id);
        }
        let id = Uuid::now_v7().to_string();
        connection.execute(
            "INSERT INTO conversations(id, title, character_id, created_at_ms, updated_at_ms) VALUES(?1, ?2, 'aris', ?3, ?3)",
            params![id, "爱丽丝的冒险记录", now_ms],
        )?;
        Ok(id)
    }

    pub fn load_relationship_state(
        &self,
        character_id: &str,
    ) -> Result<RelationshipState, rusqlite::Error> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        connection
            .query_row(
                "SELECT character_id, affinity, familiarity, interaction_count, summary, version, updated_at_ms
                   FROM relationship_state
                  WHERE character_id=?1",
                [character_id],
                |row| {
                    Ok(RelationshipState {
                        character_id: row.get(0)?,
                        affinity: row.get(1)?,
                        familiarity: row.get(2)?,
                        interaction_count: row.get(3)?,
                        summary: row.get(4)?,
                        version: row.get(5)?,
                        updated_at_ms: row.get(6)?,
                    })
                },
            )
            .optional()
            .map(|state| {
                state.unwrap_or_else(|| RelationshipState {
                    character_id: character_id.to_owned(),
                    affinity: 0.0,
                    familiarity: 0.0,
                    interaction_count: 0,
                    summary: String::new(),
                    version: 1,
                    updated_at_ms: 0,
                })
            })
    }

    pub fn apply_relationship_signal(
        &self,
        character_id: &str,
        affinity_delta: f64,
        summary: &str,
        now_ms: i64,
    ) -> Result<(), rusqlite::Error> {
        if !affinity_delta.is_finite() {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let affinity_delta = affinity_delta.clamp(-0.1, 0.1);
        let connection = self.connection.lock().expect("storage mutex poisoned");
        connection.execute(
            "INSERT INTO relationship_state(character_id, affinity, familiarity, interaction_count, summary, version, updated_at_ms)
             VALUES(?1, MAX(-1.0, MIN(1.0, ?2)), 0, 0, ?3, 1, ?4)
             ON CONFLICT(character_id) DO UPDATE SET
               affinity=MAX(-1.0, MIN(1.0, relationship_state.affinity + ?2)),
               summary=CASE WHEN ?3='' THEN relationship_state.summary ELSE ?3 END,
               version=relationship_state.version + 1,
               updated_at_ms=excluded.updated_at_ms",
            params![character_id, affinity_delta, summary.trim(), now_ms],
        )?;
        Ok(())
    }

    pub fn recover_interrupted_turns(&self, now_ms: i64) -> Result<(), rusqlite::Error> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        connection.execute(
            "UPDATE turns SET status='failed', failure_code='process_interrupted', ended_at_ms=?1
             WHERE status IN ('preparing','active','finalizing')",
            [now_ms],
        )?;
        connection.execute(
            "UPDATE messages SET status='failed', completed_at_ms=?1 WHERE status IN ('pending','streaming')",
            [now_ms],
        )?;
        connection.execute(
            "UPDATE tool_runs
             SET status='failed', result_summary='应用在工具确认或执行前退出。', completed_at_ms=?1
             WHERE status IN ('pending','approved')",
            [now_ms],
        )?;
        Ok(())
    }

    pub fn latest_turn_generation(&self, conversation_id: &str) -> Result<u64, rusqlite::Error> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        connection
            .query_row(
                "SELECT COALESCE(MAX(generation), 0) FROM turns WHERE conversation_id=?1",
                [conversation_id],
                |row| row.get::<_, i64>(0),
            )
            .map(|generation| generation.max(0) as u64)
    }

    pub fn begin_turn(
        &self,
        conversation_id: &str,
        turn_id: &str,
        generation: u64,
        input_source: &str,
        text: &str,
        now_ms: i64,
    ) -> Result<ChatMessage, rusqlite::Error> {
        if !matches!(input_source, "text" | "wake_word" | "push_to_talk") {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let mut connection = self.connection.lock().expect("storage mutex poisoned");
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO turns(id, conversation_id, generation, input_source, status, started_at_ms)
             VALUES(?1, ?2, ?3, ?4, 'preparing', ?5)",
            params![turn_id, conversation_id, generation as i64, input_source, now_ms],
        )?;
        let message_id = Uuid::now_v7().to_string();
        transaction.execute(
            "INSERT INTO messages(id, conversation_id, turn_id, ordinal, role, canonical_text, display_text, status, created_at_ms, completed_at_ms)
             VALUES(?1, ?2, ?3, 0, 'user', ?4, ?4, 'completed', ?5, ?5)",
            params![message_id, conversation_id, turn_id, text, now_ms],
        )?;
        transaction.execute(
            "UPDATE conversations SET updated_at_ms=?1 WHERE id=?2",
            params![now_ms, conversation_id],
        )?;
        transaction.commit()?;
        Ok(ChatMessage {
            id: message_id,
            turn_id: turn_id.to_owned(),
            role: MessageRole::User,
            canonical_text: text.to_owned(),
            status: MessageStatus::Completed,
            created_at_ms: now_ms,
        })
    }

    pub fn mark_turn_active(&self, turn_id: &str) -> Result<(), rusqlite::Error> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        connection.execute("UPDATE turns SET status='active' WHERE id=?1", [turn_id])?;
        Ok(())
    }

    pub fn user_message_for_turn(
        &self,
        conversation_id: &str,
        turn_id: &str,
    ) -> Result<Option<ChatMessage>, rusqlite::Error> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        connection
            .query_row(
                "SELECT id, turn_id, canonical_text, created_at_ms
                   FROM messages
                  WHERE conversation_id=?1 AND turn_id=?2 AND role='user'
                  ORDER BY ordinal ASC
                  LIMIT 1",
                params![conversation_id, turn_id],
                |row| {
                    Ok(ChatMessage {
                        id: row.get(0)?,
                        turn_id: row.get(1)?,
                        role: MessageRole::User,
                        canonical_text: row.get(2)?,
                        status: MessageStatus::Completed,
                        created_at_ms: row.get(3)?,
                    })
                },
            )
            .optional()
    }

    pub fn complete_turn(
        &self,
        conversation_id: &str,
        character_id: &str,
        turn_id: &str,
        text: &str,
        now_ms: i64,
    ) -> Result<ChatMessage, rusqlite::Error> {
        let mut connection = self.connection.lock().expect("storage mutex poisoned");
        let transaction = connection.transaction()?;
        let message_id = Uuid::now_v7().to_string();
        transaction.execute(
            "INSERT INTO messages(id, conversation_id, turn_id, ordinal, role, canonical_text, display_text, spoken_text, status, created_at_ms, completed_at_ms)
             VALUES(?1, ?2, ?3, 1, 'assistant', ?4, ?4, '', 'completed', ?5, ?5)",
            params![message_id, conversation_id, turn_id, text, now_ms],
        )?;
        transaction.execute(
            "UPDATE turns SET status='completed', ended_at_ms=?1 WHERE id=?2",
            params![now_ms, turn_id],
        )?;
        transaction.execute(
            "UPDATE conversations SET updated_at_ms=?1 WHERE id=?2",
            params![now_ms, conversation_id],
        )?;
        transaction.execute(
            "INSERT INTO relationship_state(character_id, affinity, familiarity, interaction_count, summary, version, updated_at_ms)
             VALUES(?1, 0, MIN(1.0, ?2), 1, '', 1, ?3)
             ON CONFLICT(character_id) DO UPDATE SET
               familiarity=MIN(1.0, relationship_state.familiarity + ?2),
               interaction_count=relationship_state.interaction_count + 1,
               version=relationship_state.version + 1,
               updated_at_ms=excluded.updated_at_ms",
            params![character_id, RELATIONSHIP_FAMILIARITY_STEP, now_ms],
        )?;
        transaction.commit()?;
        Ok(ChatMessage {
            id: message_id,
            turn_id: turn_id.to_owned(),
            role: MessageRole::Assistant,
            canonical_text: text.to_owned(),
            status: MessageStatus::Completed,
            created_at_ms: now_ms,
        })
    }

    pub fn finish_turn_with_status(
        &self,
        turn_id: &str,
        status: &str,
        failure_code: Option<&str>,
        partial_text: &str,
        conversation_id: &str,
        now_ms: i64,
    ) -> Result<(), rusqlite::Error> {
        let mut connection = self.connection.lock().expect("storage mutex poisoned");
        let transaction = connection.transaction()?;
        transaction.execute(
            "UPDATE turns SET status=?1, failure_code=?2, ended_at_ms=?3 WHERE id=?4",
            params![status, failure_code, now_ms, turn_id],
        )?;
        if !partial_text.is_empty() {
            transaction.execute(
                "INSERT OR IGNORE INTO messages(id, conversation_id, turn_id, ordinal, role, canonical_text, display_text, spoken_text, status, created_at_ms, completed_at_ms)
                 VALUES(?1, ?2, ?3, 1, 'assistant', ?4, ?4, '', ?5, ?6, ?6)",
                params![Uuid::now_v7().to_string(), conversation_id, turn_id, partial_text, status, now_ms],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn list_messages(
        &self,
        conversation_id: &str,
        limit: usize,
    ) -> Result<Vec<ChatMessage>, rusqlite::Error> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let mut statement = connection.prepare(
            "SELECT id, turn_id, role, canonical_text, status, created_at_ms
             FROM messages WHERE conversation_id=?1
             ORDER BY created_at_ms DESC, ordinal DESC LIMIT ?2",
        )?;
        let rows = statement.query_map(params![conversation_id, limit as i64], |row| {
            let role: String = row.get(2)?;
            let status: String = row.get(4)?;
            Ok(ChatMessage {
                id: row.get(0)?,
                turn_id: row.get(1)?,
                role: if role == "user" {
                    MessageRole::User
                } else {
                    MessageRole::Assistant
                },
                canonical_text: row.get(3)?,
                status: match status.as_str() {
                    "pending" => MessageStatus::Pending,
                    "streaming" => MessageStatus::Streaming,
                    "cancelled" => MessageStatus::Cancelled,
                    "failed" => MessageStatus::Failed,
                    _ => MessageStatus::Completed,
                },
                created_at_ms: row.get(5)?,
            })
        })?;
        let mut messages = rows.collect::<Result<Vec<_>, _>>()?;
        messages.reverse();
        Ok(messages)
    }

    pub fn load_provider_settings(&self) -> Result<ProviderSettings, rusqlite::Error> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let stored = connection
            .query_row(
                "SELECT value_json, revision FROM app_settings WHERE key='provider.llm'",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?)),
            )
            .optional()?;
        match stored {
            Some((json, revision)) => {
                let mut value: ProviderSettings = serde_json::from_str(&json).unwrap_or_default();
                value.revision = revision;
                value.apply_environment_overrides();
                Ok(value)
            }
            None => Ok(ProviderSettings::default()),
        }
    }

    pub fn save_provider_settings(
        &self,
        value: &ProviderSettings,
        now_ms: i64,
    ) -> Result<(), rusqlite::Error> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let json = serde_json::to_string(value).expect("provider settings serialize");
        connection.execute(
            "INSERT INTO app_settings(key, value_json, revision, updated_at_ms) VALUES('provider.llm', ?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json, revision=excluded.revision, updated_at_ms=excluded.updated_at_ms",
            params![json, value.revision, now_ms],
        )?;
        Ok(())
    }

    pub fn load_pet_settings(&self) -> Result<PetSettings, rusqlite::Error> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let stored = connection
            .query_row(
                "SELECT value_json, revision FROM app_settings WHERE key='desktop.pet'",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?)),
            )
            .optional()?;
        match stored {
            Some((json, revision)) => {
                let mut value: PetSettings = serde_json::from_str(&json).unwrap_or_default();
                value.revision = revision;
                Ok(value)
            }
            None => Ok(PetSettings::default()),
        }
    }

    pub fn save_pet_settings(
        &self,
        value: &PetSettings,
        now_ms: i64,
    ) -> Result<(), rusqlite::Error> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let json = serde_json::to_string(value).expect("pet settings serialize");
        connection.execute(
            "INSERT INTO app_settings(key, value_json, revision, updated_at_ms) VALUES('desktop.pet', ?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json, revision=excluded.revision, updated_at_ms=excluded.updated_at_ms",
            params![json, value.revision, now_ms],
        )?;
        Ok(())
    }

    pub fn load_character_settings(&self) -> Result<CharacterSettings, rusqlite::Error> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let stored = connection
            .query_row(
                "SELECT value_json, revision FROM app_settings WHERE key='character.active'",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?)),
            )
            .optional()?;
        match stored {
            Some((json, revision)) => {
                let mut value: CharacterSettings = serde_json::from_str(&json).unwrap_or_default();
                value.revision = revision;
                Ok(value)
            }
            None => Ok(CharacterSettings::default()),
        }
    }

    pub fn save_character_settings(
        &self,
        value: &CharacterSettings,
        now_ms: i64,
    ) -> Result<(), rusqlite::Error> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let json = serde_json::to_string(value).expect("character settings serialize");
        connection.execute(
            "INSERT INTO app_settings(key, value_json, revision, updated_at_ms) VALUES('character.active', ?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json, revision=excluded.revision, updated_at_ms=excluded.updated_at_ms",
            params![json, value.revision, now_ms],
        )?;
        Ok(())
    }

    pub fn load_voice_settings(&self) -> Result<VoiceSettings, rusqlite::Error> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let stored = connection
            .query_row(
                "SELECT value_json, revision FROM app_settings WHERE key='voice.settings'",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?)),
            )
            .optional()?;
        match stored {
            Some((json, revision)) => {
                let mut value: VoiceSettings = serde_json::from_str(&json).unwrap_or_default();
                value.revision = revision;
                Ok(value)
            }
            None => Ok(VoiceSettings::default()),
        }
    }

    pub fn save_voice_settings(
        &self,
        value: &VoiceSettings,
        now_ms: i64,
    ) -> Result<(), rusqlite::Error> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let json = serde_json::to_string(value).expect("voice settings serialize");
        connection.execute(
            "INSERT INTO app_settings(key, value_json, revision, updated_at_ms) VALUES('voice.settings', ?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json, revision=excluded.revision, updated_at_ms=excluded.updated_at_ms",
            params![json, value.revision, now_ms],
        )?;
        Ok(())
    }

    pub fn load_memory_settings(&self) -> Result<MemorySettings, rusqlite::Error> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let stored = connection
            .query_row(
                "SELECT value_json, revision FROM app_settings WHERE key='memory.settings'",
                [],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?)),
            )
            .optional()?;
        match stored {
            Some((json, revision)) => {
                let mut value: MemorySettings = serde_json::from_str(&json).unwrap_or_default();
                value.revision = revision;
                Ok(value)
            }
            None => Ok(MemorySettings::default()),
        }
    }

    pub fn save_memory_settings(
        &self,
        value: &MemorySettings,
        now_ms: i64,
    ) -> Result<(), rusqlite::Error> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let json = serde_json::to_string(value).expect("memory settings serialize");
        connection.execute(
            "INSERT INTO app_settings(key, value_json, revision, updated_at_ms) VALUES('memory.settings', ?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET value_json=excluded.value_json, revision=excluded.revision, updated_at_ms=excluded.updated_at_ms",
            params![json, value.revision, now_ms],
        )?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn create_user_memory(
        &self,
        content: &str,
        now_ms: i64,
    ) -> Result<MemoryItem, rusqlite::Error> {
        self.create_user_memory_with_metadata(content, "note", "normal", None, now_ms)
    }

    pub fn create_user_memory_with_metadata(
        &self,
        content: &str,
        kind: &str,
        sensitivity: &str,
        fact_key: Option<&str>,
        now_ms: i64,
    ) -> Result<MemoryItem, rusqlite::Error> {
        let mut connection = self.connection.lock().expect("storage mutex poisoned");
        let transaction = connection.transaction()?;
        let normalized_key = normalize_memory_text(content);
        let valid_until_ms = memory_valid_until_for_kind(kind, now_ms);
        let duplicate_query = format!(
            "SELECT {MEMORY_SELECT_COLUMNS}
               FROM memories m
              WHERE m.scope='user'
                AND m.character_id IS NULL
                AND m.normalized_key=?1
                AND (m.valid_until_ms IS NULL OR m.valid_until_ms > ?2)
                AND NOT EXISTS (SELECT 1 FROM memories newer WHERE newer.supersedes_id=m.id)
              ORDER BY m.updated_at_ms DESC, m.id DESC
              LIMIT 1"
        );
        if let Some(existing) = transaction
            .query_row(
                &duplicate_query,
                params![&normalized_key, now_ms],
                map_memory_row,
            )
            .optional()?
        {
            transaction.commit()?;
            return Ok(existing);
        }
        let item = MemoryItem {
            id: Uuid::now_v7().to_string(),
            scope: "user".into(),
            character_id: None,
            kind: kind.to_owned(),
            content: content.to_owned(),
            importance: 0.7,
            confidence: 1.0,
            sensitivity: sensitivity.to_owned(),
            origin: "user_explicit".into(),
            fact_key: fact_key.map(str::to_owned),
            source_turn_id: None,
            source_message_id: None,
            valid_from_ms: None,
            valid_until_ms,
            last_confirmed_at_ms: Some(now_ms),
            version: 1,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        };
        transaction.execute(
            "INSERT INTO memories(
                id, scope, character_id, kind, content, normalized_key, importance,
                confidence, sensitivity, origin, source_turn_id, fact_key,
                valid_until_ms, last_confirmed_at_ms, version, created_at_ms, updated_at_ms
             ) VALUES(?1, 'user', NULL, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 1, ?13, ?13)",
            params![
                item.id,
                item.kind,
                item.content,
                normalized_key,
                item.importance,
                item.confidence,
                item.sensitivity,
                item.origin,
                item.source_turn_id,
                item.fact_key,
                item.valid_until_ms,
                item.last_confirmed_at_ms,
                now_ms,
            ],
        )?;
        let rowid = transaction.last_insert_rowid();
        transaction.execute(
            "INSERT INTO memory_fts(rowid, content, normalized_key) VALUES(?1, ?2, ?3)",
            params![rowid, item.content, normalized_key],
        )?;
        transaction.execute(
            "INSERT INTO memory_audit(id, memory_id, action, occurred_at_ms) VALUES(?1, ?2, 'confirmed', ?3)",
            params![Uuid::now_v7().to_string(), item.id, now_ms],
        )?;
        transaction.commit()?;
        Ok(item)
    }

    #[allow(dead_code)]
    pub fn update_memory(
        &self,
        id: &str,
        content: &str,
        expected_version: u64,
        now_ms: i64,
    ) -> Result<Option<MemoryItem>, rusqlite::Error> {
        self.update_memory_with_metadata(id, content, expected_version, None, None, now_ms)
    }

    pub fn update_memory_with_metadata(
        &self,
        id: &str,
        content: &str,
        expected_version: u64,
        next_fact_key: Option<&str>,
        next_sensitivity: Option<&str>,
        now_ms: i64,
    ) -> Result<Option<MemoryItem>, rusqlite::Error> {
        let mut connection = self.connection.lock().expect("storage mutex poisoned");
        let transaction = connection.transaction()?;
        let previous = transaction
            .query_row(
                "SELECT rowid, scope, character_id, kind, content, normalized_key, importance, version, created_at_ms,
                        confidence, sensitivity, origin, fact_key, source_turn_id, source_message_id,
                        valid_from_ms, valid_until_ms, last_confirmed_at_ms
                 FROM memories WHERE id=?1",
                [id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, f64>(6)?,
                        row.get::<_, u64>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, f64>(9)?,
                        row.get::<_, String>(10)?,
                        row.get::<_, String>(11)?,
                        row.get::<_, Option<String>>(12)?,
                        row.get::<_, Option<String>>(13)?,
                        row.get::<_, Option<String>>(14)?,
                        row.get::<_, Option<i64>>(15)?,
                        row.get::<_, Option<i64>>(16)?,
                        row.get::<_, Option<i64>>(17)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            rowid,
            scope,
            character_id,
            kind,
            old_content,
            old_key,
            importance,
            version,
            created_at_ms,
            confidence,
            previous_sensitivity,
            origin,
            _previous_fact_key,
            source_turn_id,
            source_message_id,
            valid_from_ms,
            valid_until_ms,
            _last_confirmed_at_ms,
        )) = previous
        else {
            return Ok(None);
        };
        if version != expected_version {
            return Ok(None);
        }
        let normalized_key = normalize_memory_text(content);
        transaction.execute(
            "INSERT INTO memory_fts(memory_fts, rowid, content, normalized_key) VALUES('delete', ?1, ?2, ?3)",
            params![rowid, old_content, old_key],
        )?;
        transaction.execute(
            "UPDATE memories
                SET content=?1, normalized_key=?2, fact_key=?3,
                    sensitivity=COALESCE(?4, sensitivity), version=version+1,
                    updated_at_ms=?5, last_confirmed_at_ms=?5
              WHERE id=?6 AND version=?7",
            params![
                content,
                normalized_key,
                next_fact_key,
                next_sensitivity,
                now_ms,
                id,
                expected_version
            ],
        )?;
        transaction.execute(
            "INSERT INTO memory_fts(rowid, content, normalized_key) VALUES(?1, ?2, ?3)",
            params![rowid, content, normalized_key],
        )?;
        transaction.execute(
            "INSERT INTO memory_audit(id, memory_id, action, occurred_at_ms) VALUES(?1, ?2, 'updated', ?3)",
            params![Uuid::now_v7().to_string(), id, now_ms],
        )?;
        transaction.commit()?;
        Ok(Some(MemoryItem {
            id: id.to_owned(),
            scope,
            character_id,
            kind,
            content: content.to_owned(),
            importance,
            confidence,
            sensitivity: next_sensitivity
                .map(str::to_owned)
                .unwrap_or(previous_sensitivity),
            origin,
            fact_key: next_fact_key.map(str::to_owned),
            source_turn_id,
            source_message_id,
            valid_from_ms,
            valid_until_ms,
            last_confirmed_at_ms: Some(now_ms),
            version: version + 1,
            created_at_ms,
            updated_at_ms: now_ms,
        }))
    }

    pub fn memory_by_id(&self, id: &str) -> Result<Option<MemoryItem>, rusqlite::Error> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let query = format!("SELECT {MEMORY_SELECT_COLUMNS} FROM memories m WHERE m.id=?1");
        connection
            .query_row(&query, [id], map_memory_row)
            .optional()
    }

    pub fn find_active_memory_conflict(
        &self,
        scope: &str,
        character_id: Option<&str>,
        fact_key: Option<&str>,
        content: &str,
        now_ms: i64,
    ) -> Result<Option<MemoryItem>, rusqlite::Error> {
        let Some(fact_key) = fact_key.filter(|value| !value.trim().is_empty()) else {
            return Ok(None);
        };
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let query = format!(
            "SELECT {MEMORY_SELECT_COLUMNS}
               FROM memories m
              WHERE m.scope=?1
                AND ((m.scope='user' AND m.character_id IS NULL)
                  OR (m.scope='relationship' AND m.character_id=?2))
                AND m.fact_key=?3
                AND m.normalized_key<>?4
                AND (m.valid_until_ms IS NULL OR m.valid_until_ms>?5)
                AND NOT EXISTS (SELECT 1 FROM memories newer WHERE newer.supersedes_id=m.id)
              ORDER BY m.updated_at_ms DESC, m.id DESC
              LIMIT 1"
        );
        connection
            .query_row(
                &query,
                params![
                    scope,
                    character_id,
                    fact_key,
                    normalize_memory_text(content),
                    now_ms
                ],
                map_memory_row,
            )
            .optional()
    }

    pub fn has_duplicate_user_memory(
        &self,
        content: &str,
        excluded_id: &str,
    ) -> Result<bool, rusqlite::Error> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let normalized_key = normalize_memory_text(content);
        let found: i64 = connection.query_row(
            "SELECT EXISTS(
                SELECT 1
                  FROM memories m
                 WHERE m.scope='user'
                   AND m.character_id IS NULL
                   AND m.normalized_key=?1
                   AND m.id<>?2
                   AND (m.valid_until_ms IS NULL OR m.valid_until_ms>?3)
                   AND NOT EXISTS (SELECT 1 FROM memories newer WHERE newer.supersedes_id=m.id)
            )",
            params![normalized_key, excluded_id, unix_now_ms()],
            |row| row.get(0),
        )?;
        Ok(found != 0)
    }

    pub fn delete_memory(
        &self,
        id: &str,
        expected_version: u64,
        now_ms: i64,
    ) -> Result<bool, rusqlite::Error> {
        let mut connection = self.connection.lock().expect("storage mutex poisoned");
        let transaction = connection.transaction()?;
        let previous = transaction
            .query_row(
                "SELECT rowid, content, normalized_key, version FROM memories WHERE id=?1",
                [id],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, u64>(3)?,
                    ))
                },
            )
            .optional()?;
        let Some((rowid, content, normalized_key, version)) = previous else {
            return Ok(false);
        };
        if version != expected_version {
            return Ok(false);
        }
        transaction.execute(
            "INSERT INTO memory_fts(memory_fts, rowid, content, normalized_key) VALUES('delete', ?1, ?2, ?3)",
            params![rowid, content, normalized_key],
        )?;
        transaction.execute(
            "DELETE FROM memories WHERE id=?1 AND version=?2",
            params![id, expected_version],
        )?;
        transaction.execute(
            "INSERT INTO memory_audit(id, memory_id, action, occurred_at_ms) VALUES(?1, ?2, 'deleted', ?3)",
            params![Uuid::now_v7().to_string(), id, now_ms],
        )?;
        transaction.commit()?;
        Ok(true)
    }

    pub fn search_memories(
        &self,
        query: &str,
        character_id: &str,
        limit: usize,
    ) -> Result<Vec<MemoryItem>, rusqlite::Error> {
        self.search_memories_internal(query, character_id, limit, false)
    }

    pub fn search_memories_for_context(
        &self,
        query: &str,
        character_id: &str,
        limit: usize,
    ) -> Result<Vec<MemoryItem>, rusqlite::Error> {
        self.search_memories_internal(query, character_id, limit, true)
    }

    fn search_memories_internal(
        &self,
        query: &str,
        character_id: &str,
        limit: usize,
        normal_only: bool,
    ) -> Result<Vec<MemoryItem>, rusqlite::Error> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let limit = limit.clamp(1, 20) as i64;
        let now_ms = unix_now_ms();
        let sensitivity_clause = if normal_only {
            "AND m.sensitivity='normal'"
        } else {
            ""
        };
        let normalized = normalize_memory_text(query);
        let fts = memory_fts_query(&normalized);
        let mut items = if normalized.is_empty() {
            let query = format!(
                "SELECT {MEMORY_SELECT_COLUMNS}
                   FROM memories m
                  WHERE (m.scope='user' OR (m.scope='relationship' AND m.character_id=?1))
                    AND (m.valid_until_ms IS NULL OR m.valid_until_ms > ?2)
                    {sensitivity_clause}
                    AND NOT EXISTS (SELECT 1 FROM memories newer WHERE newer.supersedes_id=m.id)
                  ORDER BY m.updated_at_ms DESC LIMIT ?3"
            );
            let mut statement = connection.prepare(&query)?;
            let rows = statement.query_map(params![character_id, now_ms, limit], map_memory_row)?;
            rows.collect::<Result<Vec<_>, _>>()?
        } else if let Some(fts) = fts {
            let query = format!(
                "SELECT {MEMORY_SELECT_COLUMNS}
                   FROM memory_fts JOIN memories m ON m.rowid=memory_fts.rowid
                  WHERE memory_fts MATCH ?1
                    AND (m.scope='user' OR (m.scope='relationship' AND m.character_id=?2))
                    AND (m.valid_until_ms IS NULL OR m.valid_until_ms > ?3)
                    {sensitivity_clause}
                    AND NOT EXISTS (SELECT 1 FROM memories newer WHERE newer.supersedes_id=m.id)
                  ORDER BY bm25(memory_fts), m.importance DESC, m.updated_at_ms DESC LIMIT ?4"
            );
            let mut statement = connection.prepare(&query)?;
            let rows =
                statement.query_map(params![fts, character_id, now_ms, limit], map_memory_row)?;
            rows.collect::<Result<Vec<_>, _>>()?
        } else {
            let prefix = format!("{normalized}%");
            let contains = format!("%{normalized}%");
            let query = format!(
                "SELECT {MEMORY_SELECT_COLUMNS}
                   FROM memories m
                  WHERE (m.scope='user' OR (m.scope='relationship' AND m.character_id=?1))
                    AND (m.normalized_key LIKE ?2 ESCAPE '\\' OR lower(m.content) LIKE ?3 ESCAPE '\\')
                    AND (m.valid_until_ms IS NULL OR m.valid_until_ms > ?4)
                    {sensitivity_clause}
                    AND NOT EXISTS (SELECT 1 FROM memories newer WHERE newer.supersedes_id=m.id)
                  ORDER BY m.importance DESC, m.updated_at_ms DESC LIMIT ?5"
            );
            let mut statement = connection.prepare(&query)?;
            let rows = statement.query_map(
                params![character_id, prefix, contains, now_ms, limit],
                map_memory_row,
            )?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        if !normalized.is_empty() && items.is_empty() {
            items = lexical_memory_fallback(
                &connection,
                &normalized,
                character_id,
                limit,
                normal_only,
                now_ms,
            )?;
        }
        items.truncate(limit as usize);
        Ok(items)
    }

    pub fn mark_memories_used(&self, ids: &[String], now_ms: i64) -> Result<(), rusqlite::Error> {
        if ids.is_empty() {
            return Ok(());
        }
        let mut connection = self.connection.lock().expect("storage mutex poisoned");
        let transaction = connection.transaction()?;
        for id in ids {
            transaction.execute(
                "UPDATE memories SET last_used_at_ms=?1 WHERE id=?2",
                params![now_ms, id],
            )?;
        }
        transaction.commit()
    }

    pub fn data_summary(&self, conversation_id: &str) -> Result<DataSummary, rusqlite::Error> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        Ok(DataSummary {
            message_count: connection.query_row(
                "SELECT COUNT(*) FROM messages WHERE conversation_id=?1",
                [conversation_id],
                |row| row.get(0),
            )?,
            memory_count: connection
                .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))?,
            memory_candidate_count: connection.query_row(
                "SELECT COUNT(*) FROM memory_candidates WHERE status='pending'",
                [],
                |row| row.get(0),
            )?,
        })
    }

    pub fn clear_conversation_history(
        &self,
        conversation_id: &str,
        now_ms: i64,
    ) -> Result<(), rusqlite::Error> {
        let mut connection = self.connection.lock().expect("storage mutex poisoned");
        let transaction = connection.transaction()?;
        transaction.execute(
            "DELETE FROM turns WHERE conversation_id=?1",
            [conversation_id],
        )?;
        transaction.execute(
            "DELETE FROM timers WHERE conversation_id=?1",
            [conversation_id],
        )?;
        transaction.execute(
            "UPDATE conversations SET updated_at_ms=?1 WHERE id=?2",
            params![now_ms, conversation_id],
        )?;
        transaction.commit()
    }

    pub fn clear_all_memories(&self) -> Result<(), rusqlite::Error> {
        let mut connection = self.connection.lock().expect("storage mutex poisoned");
        let transaction = connection.transaction()?;
        transaction.execute("DELETE FROM memories", [])?;
        transaction.execute("INSERT INTO memory_fts(memory_fts) VALUES('rebuild')", [])?;
        transaction.execute("DELETE FROM memory_candidates", [])?;
        transaction.execute("DELETE FROM relationship_state", [])?;
        transaction.execute("DELETE FROM memory_audit", [])?;
        transaction.commit()
    }

    #[allow(dead_code)]
    pub fn create_memory_candidate(
        &self,
        input: MemoryCandidateInput<'_>,
    ) -> Result<MemoryCandidate, rusqlite::Error> {
        self.create_memory_candidate_with_metadata(MemoryCandidateV2Input {
            id: input.id,
            source_turn_id: input.source_turn_id,
            source_message_id: input.source_message_id,
            scope: input.scope,
            kind: input.kind,
            content: input.content,
            confidence: input.confidence,
            sensitivity: input.sensitivity,
            origin: "automatic_candidate",
            reason: "从本轮用户消息中识别出的候选，仍需老师确认。",
            fact_key: None,
            proposed_action: "create",
            // The compatibility helper is also used by deterministic storage tests and
            // older callers. New production candidates set an explicit expiry through
            // `MemoryCandidateV2Input`.
            expires_at_ms: None,
            created_at_ms: input.created_at_ms,
        })
    }

    pub fn create_memory_candidate_with_metadata(
        &self,
        input: MemoryCandidateV2Input<'_>,
    ) -> Result<MemoryCandidate, rusqlite::Error> {
        if !matches!(
            input.origin,
            "user_explicit" | "model_suggested" | "automatic_candidate"
        ) || !matches!(input.proposed_action, "create" | "replace" | "append")
            || !matches!(input.sensitivity, "normal" | "sensitive" | "restricted")
        {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let candidate = MemoryCandidate {
            id: input.id.to_owned(),
            scope: input.scope.to_owned(),
            kind: input.kind.to_owned(),
            content: input.content.to_owned(),
            confidence: input.confidence,
            sensitivity: input.sensitivity.to_owned(),
            status: "pending".into(),
            created_at_ms: input.created_at_ms,
            reviewed_at_ms: None,
            duplicate_of: None,
            duplicate_content: None,
            origin: input.origin.to_owned(),
            reason: input.reason.to_owned(),
            fact_key: input.fact_key.map(str::to_owned),
            proposed_action: input.proposed_action.to_owned(),
            expires_at_ms: input.expires_at_ms,
            conflict_of: None,
            conflict_content: None,
        };
        let normalized_key = normalize_memory_text(&candidate.content);
        connection.execute(
            "INSERT INTO memory_candidates(
                id, source_turn_id, source_message_id, scope, kind, content,
                normalized_key, confidence, sensitivity, status, created_at_ms,
                origin, reason, fact_key, proposed_action, expires_at_ms
             ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'pending', ?10, ?11, ?12, ?13, ?14, ?15)",
            params![
                candidate.id,
                input.source_turn_id,
                input.source_message_id,
                candidate.scope,
                candidate.kind,
                candidate.content,
                normalized_key,
                candidate.confidence,
                candidate.sensitivity,
                candidate.created_at_ms,
                candidate.origin,
                candidate.reason,
                candidate.fact_key,
                candidate.proposed_action,
                candidate.expires_at_ms,
            ],
        )?;
        load_memory_candidate(&connection, &candidate.id)?
            .ok_or(rusqlite::Error::QueryReturnedNoRows)
    }

    pub fn expire_stale_memory_candidates(&self, now_ms: i64) -> Result<usize, rusqlite::Error> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        connection.execute(
            "UPDATE memory_candidates
                SET status='expired', reviewed_at_ms=?1
              WHERE status='pending' AND expires_at_ms IS NOT NULL AND expires_at_ms<=?1",
            [now_ms],
        )
    }

    pub fn has_duplicate_memory_or_candidate(
        &self,
        content: &str,
        scope: &str,
        character_id: &str,
    ) -> Result<bool, rusqlite::Error> {
        self.expire_stale_memory_candidates(unix_now_ms())?;
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let normalized_key = normalize_memory_text(content);
        let found: i64 = connection.query_row(
            "SELECT EXISTS(
                SELECT 1
                  FROM memory_candidates c
                  JOIN turns t ON t.id=c.source_turn_id
                  JOIN conversations conversation ON conversation.id=t.conversation_id
                 WHERE c.status='pending'
                   AND c.scope=?1
                   AND c.normalized_key=?2
                   AND (?1='user' OR conversation.character_id=?3)
                   AND (c.expires_at_ms IS NULL OR c.expires_at_ms > ?4)
                UNION ALL
                SELECT 1
                  FROM memories m
                 WHERE m.scope=?1
                   AND m.normalized_key=?2
                   AND ((?1='user' AND m.character_id IS NULL)
                     OR (?1='relationship' AND m.character_id=?3))
                   AND (m.valid_until_ms IS NULL OR m.valid_until_ms > ?4)
                   AND NOT EXISTS (SELECT 1 FROM memories newer WHERE newer.supersedes_id=m.id)
            )",
            params![scope, normalized_key, character_id, unix_now_ms()],
            |row| row.get(0),
        )?;
        Ok(found != 0)
    }

    pub fn list_pending_memory_candidates(
        &self,
        limit: usize,
    ) -> Result<Vec<MemoryCandidate>, rusqlite::Error> {
        self.expire_stale_memory_candidates(unix_now_ms())?;
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let query = format!(
            "{MEMORY_CANDIDATE_SELECT}
             WHERE c.status='pending'
             ORDER BY c.created_at_ms DESC
             LIMIT ?1"
        );
        let mut statement = connection.prepare(&query)?;
        let rows = statement.query_map([limit.clamp(1, 50) as i64], map_memory_candidate_row)?;
        rows.collect()
    }

    #[allow(dead_code)]
    pub fn update_memory_candidate(
        &self,
        candidate_id: &str,
        content: &str,
    ) -> Result<Option<MemoryCandidate>, rusqlite::Error> {
        self.update_memory_candidate_with_metadata(candidate_id, content, None, None)
    }

    pub fn update_memory_candidate_with_metadata(
        &self,
        candidate_id: &str,
        content: &str,
        fact_key: Option<&str>,
        sensitivity: Option<&str>,
    ) -> Result<Option<MemoryCandidate>, rusqlite::Error> {
        self.expire_stale_memory_candidates(unix_now_ms())?;
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let normalized_key = normalize_memory_text(content);
        let changed = connection.execute(
            "UPDATE memory_candidates
             SET content=?1, normalized_key=?2, fact_key=?3,
                 sensitivity=COALESCE(?4, sensitivity)
             WHERE id=?5 AND status='pending'",
            params![content, normalized_key, fact_key, sensitivity, candidate_id],
        )?;
        if changed != 1 {
            return Ok(None);
        }
        load_memory_candidate(&connection, candidate_id)
    }

    #[allow(dead_code)]
    pub fn confirm_memory_candidate(
        &self,
        candidate_id: &str,
        character_id: &str,
        now_ms: i64,
    ) -> Result<Option<MemoryItem>, rusqlite::Error> {
        let result = match self.confirm_memory_candidate_with_resolution(
            candidate_id,
            character_id,
            None,
            now_ms,
        ) {
            Ok(result) => result,
            Err(rusqlite::Error::QueryReturnedNoRows) => return Ok(None),
            Err(error) => return Err(error),
        };
        match result {
            MemoryConfirmResult::Saved(item) | MemoryConfirmResult::Duplicate(item) => {
                Ok(Some(item))
            }
            MemoryConfirmResult::Conflict(_) => Ok(None),
        }
    }

    pub fn confirm_memory_candidate_with_resolution(
        &self,
        candidate_id: &str,
        character_id: &str,
        resolution: Option<MemoryConflictResolution>,
        now_ms: i64,
    ) -> Result<MemoryConfirmResult, rusqlite::Error> {
        let mut connection = self.connection.lock().expect("storage mutex poisoned");
        let transaction = connection.transaction()?;
        let candidate = transaction
            .query_row(
                "SELECT scope, kind, content, normalized_key, confidence, sensitivity,
                        source_turn_id, source_message_id, created_at_ms, origin, fact_key
                 FROM memory_candidates
                 WHERE id=?1 AND status='pending'
                   AND (expires_at_ms IS NULL OR expires_at_ms>?2)",
                params![candidate_id, now_ms],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, f64>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, Option<String>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, String>(9)?,
                        row.get::<_, Option<String>>(10)?,
                    ))
                },
            )
            .optional()?;
        let Some((
            scope,
            kind,
            content,
            normalized_key,
            confidence,
            sensitivity,
            source_turn_id,
            source_message_id,
            created_at_ms,
            origin,
            fact_key,
        )) = candidate
        else {
            return Err(rusqlite::Error::QueryReturnedNoRows);
        };
        let memory_character_id = match scope.as_str() {
            "user" => None,
            "relationship" => Some(character_id),
            _ => return Err(rusqlite::Error::InvalidQuery),
        };
        let duplicate_query = format!(
            "SELECT {MEMORY_SELECT_COLUMNS}
               FROM memories m
              WHERE m.scope=?1
                AND ((m.scope='user' AND m.character_id IS NULL)
                  OR (m.scope='relationship' AND m.character_id=?2))
                AND m.normalized_key=?3
                AND (m.valid_until_ms IS NULL OR m.valid_until_ms>?4)
                AND NOT EXISTS (SELECT 1 FROM memories newer WHERE newer.supersedes_id=m.id)
              ORDER BY m.updated_at_ms DESC, m.id DESC
              LIMIT 1"
        );
        if let Some(existing) = transaction
            .query_row(
                &duplicate_query,
                params![scope, character_id, normalized_key, now_ms],
                map_memory_row,
            )
            .optional()?
        {
            transaction.execute(
                "UPDATE memory_candidates SET status='confirmed', reviewed_at_ms=?1
                 WHERE id=?2 AND status='pending'",
                params![now_ms, candidate_id],
            )?;
            transaction.commit()?;
            return Ok(MemoryConfirmResult::Duplicate(existing));
        }
        let conflict_query = format!(
            "SELECT {MEMORY_SELECT_COLUMNS}
               FROM memories m
              WHERE ?1 IS NOT NULL AND ?1<>''
                AND m.scope=?2
                AND ((m.scope='user' AND m.character_id IS NULL)
                  OR (m.scope='relationship' AND m.character_id=?3))
                AND m.fact_key=?1
                AND m.normalized_key<>?4
                AND (m.valid_until_ms IS NULL OR m.valid_until_ms>?5)
                AND NOT EXISTS (SELECT 1 FROM memories newer WHERE newer.supersedes_id=m.id)
              ORDER BY m.updated_at_ms DESC, m.id DESC
              LIMIT 1"
        );
        let conflict = transaction
            .query_row(
                &conflict_query,
                params![fact_key, scope, character_id, normalized_key, now_ms],
                map_memory_row,
            )
            .optional()?;
        let (supersedes_id, previous_version) = match (conflict, resolution) {
            (Some(existing), None) => {
                transaction.commit()?;
                return Ok(MemoryConfirmResult::Conflict(existing));
            }
            (Some(existing), Some(MemoryConflictResolution::Replace)) => {
                (Some(existing.id.clone()), Some(existing.version))
            }
            (Some(_), Some(MemoryConflictResolution::KeepBoth)) | (None, _) => (None, None),
        };
        let valid_until_ms = memory_valid_until_for_kind(&kind, created_at_ms);
        let item = MemoryItem {
            id: Uuid::now_v7().to_string(),
            scope,
            character_id: memory_character_id.map(str::to_owned),
            kind,
            content,
            importance: 0.7,
            confidence,
            sensitivity,
            origin,
            fact_key,
            source_turn_id,
            source_message_id,
            valid_from_ms: Some(created_at_ms),
            valid_until_ms,
            last_confirmed_at_ms: Some(now_ms),
            version: 1,
            created_at_ms,
            updated_at_ms: now_ms,
        };
        transaction.execute(
            "INSERT INTO memories(
                id, scope, character_id, kind, content, normalized_key, importance,
                confidence, sensitivity, origin, fact_key, source_turn_id,
                source_message_id, source_candidate_id, supersedes_id,
                valid_from_ms, valid_until_ms, last_confirmed_at_ms,
                version, created_at_ms, updated_at_ms
             ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, 1, ?19, ?20)",
            params![
                item.id,
                item.scope,
                item.character_id,
                item.kind,
                item.content,
                normalized_key,
                item.importance,
                item.confidence,
                item.sensitivity,
                item.origin,
                item.fact_key,
                item.source_turn_id,
                item.source_message_id,
                candidate_id,
                supersedes_id,
                item.valid_from_ms,
                item.valid_until_ms,
                item.last_confirmed_at_ms,
                item.created_at_ms,
                item.updated_at_ms,
            ],
        )?;
        let rowid = transaction.last_insert_rowid();
        transaction.execute(
            "INSERT INTO memory_fts(rowid, content, normalized_key) VALUES(?1, ?2, ?3)",
            params![rowid, item.content, normalized_key],
        )?;
        transaction.execute(
            "INSERT INTO memory_audit(
                id, memory_id, action, occurred_at_ms, actor, candidate_id,
                previous_version, replacement_id
             ) VALUES(?1, ?2, 'confirmed', ?3, 'user', ?4, ?5, ?6)",
            params![
                Uuid::now_v7().to_string(),
                item.id,
                now_ms,
                candidate_id,
                previous_version,
                supersedes_id,
            ],
        )?;
        transaction.execute(
            "UPDATE memory_candidates SET status='confirmed', reviewed_at_ms=?1
             WHERE id=?2 AND status='pending'",
            params![now_ms, candidate_id],
        )?;
        transaction.commit()?;
        Ok(MemoryConfirmResult::Saved(item))
    }

    pub fn reject_memory_candidate(
        &self,
        candidate_id: &str,
        now_ms: i64,
    ) -> Result<bool, rusqlite::Error> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let changed = connection.execute(
            "UPDATE memory_candidates SET status='rejected', reviewed_at_ms=?1
             WHERE id=?2 AND status='pending'",
            params![now_ms, candidate_id],
        )?;
        Ok(changed == 1)
    }

    pub fn begin_tool_run(
        &self,
        input: ToolRunInput<'_>,
        now_ms: i64,
    ) -> Result<(), rusqlite::Error> {
        if !matches!(
            input.origin,
            "user_explicit" | "model_suggested" | "scheduled"
        ) {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let connection = self.connection.lock().expect("storage mutex poisoned");
        connection.execute(
            "INSERT INTO tool_runs(id, conversation_id, turn_id, tool_name, arguments_json, origin, status, created_at_ms)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, 'pending', ?7)",
            params![
            input.id,
            input.conversation_id,
            input.turn_id,
            input.tool_name,
            input.arguments_json,
            input.origin,
            now_ms
        ],
        )?;
        Ok(())
    }

    pub fn finish_tool_run(
        &self,
        id: &str,
        status: &str,
        result_summary: &str,
        now_ms: i64,
    ) -> Result<(), rusqlite::Error> {
        if !matches!(
            status,
            "approved" | "rejected" | "succeeded" | "failed" | "cancelled"
        ) {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let connection = self.connection.lock().expect("storage mutex poisoned");
        connection.execute(
            "UPDATE tool_runs
             SET status=?1, result_summary=?2, completed_at_ms=?3
             WHERE id=?4 AND status IN ('pending','approved')",
            params![status, result_summary, now_ms, id],
        )?;
        Ok(())
    }

    pub fn list_tool_runs(
        &self,
        conversation_id: &str,
        limit: usize,
    ) -> Result<Vec<ToolRunSummary>, rusqlite::Error> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let mut statement = connection.prepare(
            "SELECT id, tool_name, origin, status, result_summary,
                    created_at_ms, completed_at_ms
             FROM tool_runs
             WHERE conversation_id=?1
             ORDER BY created_at_ms DESC
             LIMIT ?2",
        )?;
        let rows = statement.query_map(
            params![conversation_id, limit.clamp(1, 50) as i64],
            map_tool_run_row,
        )?;
        rows.collect()
    }

    pub fn create_timer(
        &self,
        input: TimerInput<'_>,
        now_ms: i64,
    ) -> Result<TimerItem, rusqlite::Error> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        connection.execute(
            "INSERT INTO timers(id, conversation_id, tool_run_id, title, due_at_ms, timezone, original_request, status, created_at_ms, updated_at_ms)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', ?8, ?8)",
            params![
                input.id,
                input.conversation_id,
                input.tool_run_id,
                input.title,
                input.due_at_ms,
                input.timezone,
                input.original_request,
                now_ms
            ],
        )?;
        Ok(TimerItem {
            id: input.id.to_owned(),
            title: input.title.to_owned(),
            due_at_ms: input.due_at_ms,
            timezone: input.timezone.to_owned(),
            original_request: input.original_request.to_owned(),
            status: "pending".into(),
            created_at_ms: now_ms,
        })
    }

    pub fn list_pending_timers(&self, limit: usize) -> Result<Vec<TimerItem>, rusqlite::Error> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let mut statement = connection.prepare(
            "SELECT id, title, due_at_ms, timezone, original_request, status, created_at_ms
             FROM timers WHERE status='pending' ORDER BY due_at_ms ASC LIMIT ?1",
        )?;
        let rows = statement.query_map([limit.clamp(1, 50) as i64], map_timer_row)?;
        rows.collect()
    }

    pub fn list_all_pending_timers(&self) -> Result<Vec<TimerItem>, rusqlite::Error> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let mut statement = connection.prepare(
            "SELECT id, title, due_at_ms, timezone, original_request, status, created_at_ms
             FROM timers WHERE status='pending' ORDER BY due_at_ms ASC",
        )?;
        let rows = statement.query_map([], map_timer_row)?;
        rows.collect()
    }

    pub fn list_timer_history(
        &self,
        limit: usize,
    ) -> Result<Vec<TimerHistoryItem>, rusqlite::Error> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let mut statement = connection.prepare(
            "SELECT id, title, due_at_ms, status,
                    CASE WHEN status='fired' THEN fired_at_ms ELSE cancelled_at_ms END,
                    created_at_ms
             FROM timers
             WHERE status IN ('fired', 'cancelled')
             ORDER BY COALESCE(fired_at_ms, cancelled_at_ms, updated_at_ms) DESC
             LIMIT ?1",
        )?;
        let rows = statement.query_map([limit.clamp(1, 50) as i64], map_timer_history_row)?;
        rows.collect()
    }

    pub fn next_pending_timer(&self) -> Result<Option<TimerItem>, rusqlite::Error> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        connection
            .query_row(
                "SELECT id, title, due_at_ms, timezone, original_request, status, created_at_ms
                 FROM timers WHERE status='pending' ORDER BY due_at_ms ASC LIMIT 1",
                [],
                map_timer_row,
            )
            .optional()
    }

    pub fn claim_due_timer(
        &self,
        timer_id: &str,
        now_ms: i64,
    ) -> Result<Option<TimerItem>, rusqlite::Error> {
        let mut connection = self.connection.lock().expect("storage mutex poisoned");
        let transaction = connection.transaction()?;
        let timer = transaction
            .query_row(
                "SELECT id, title, due_at_ms, timezone, original_request, status, created_at_ms
                 FROM timers WHERE id=?1 AND status='pending' AND due_at_ms<=?2",
                params![timer_id, now_ms],
                map_timer_row,
            )
            .optional()?;
        let Some(mut timer) = timer else {
            transaction.commit()?;
            return Ok(None);
        };
        transaction.execute(
            "UPDATE timers SET status='fired', updated_at_ms=?1, fired_at_ms=?1 WHERE id=?2 AND status='pending'",
            params![now_ms, timer_id],
        )?;
        timer.status = "fired".into();
        transaction.commit()?;
        Ok(Some(timer))
    }

    pub fn cancel_timer(&self, timer_id: &str, now_ms: i64) -> Result<bool, rusqlite::Error> {
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let changed = connection.execute(
            "UPDATE timers SET status='cancelled', updated_at_ms=?1, cancelled_at_ms=?1
             WHERE id=?2 AND status='pending'",
            params![now_ms, timer_id],
        )?;
        Ok(changed == 1)
    }
}

fn map_memory_row(row: &rusqlite::Row<'_>) -> Result<MemoryItem, rusqlite::Error> {
    Ok(MemoryItem {
        id: row.get(0)?,
        scope: row.get(1)?,
        character_id: row.get(2)?,
        kind: row.get(3)?,
        content: row.get(4)?,
        importance: row.get(5)?,
        confidence: row.get(6)?,
        sensitivity: row.get(7)?,
        origin: row.get(8)?,
        fact_key: row.get(9)?,
        source_turn_id: row.get(10)?,
        source_message_id: row.get(11)?,
        valid_from_ms: row.get(12)?,
        valid_until_ms: row.get(13)?,
        last_confirmed_at_ms: row.get(14)?,
        version: row.get(15)?,
        created_at_ms: row.get(16)?,
        updated_at_ms: row.get(17)?,
    })
}

fn map_memory_candidate_row(row: &rusqlite::Row<'_>) -> Result<MemoryCandidate, rusqlite::Error> {
    let memory_duplicate_id = row.get::<_, Option<String>>(14)?;
    let memory_duplicate_content = row.get::<_, Option<String>>(15)?;
    let candidate_duplicate_id = row.get::<_, Option<String>>(16)?;
    let candidate_duplicate_content = row.get::<_, Option<String>>(17)?;
    Ok(MemoryCandidate {
        id: row.get(0)?,
        scope: row.get(1)?,
        kind: row.get(2)?,
        content: row.get(3)?,
        confidence: row.get(4)?,
        sensitivity: row.get(5)?,
        status: row.get(6)?,
        created_at_ms: row.get(7)?,
        reviewed_at_ms: row.get(8)?,
        origin: row.get(9)?,
        reason: row.get(10)?,
        fact_key: row.get(11)?,
        proposed_action: row.get(12)?,
        expires_at_ms: row.get(13)?,
        duplicate_of: memory_duplicate_id.or(candidate_duplicate_id),
        duplicate_content: memory_duplicate_content.or(candidate_duplicate_content),
        conflict_of: row.get(18)?,
        conflict_content: row.get(19)?,
    })
}

fn load_memory_candidate(
    connection: &Connection,
    candidate_id: &str,
) -> Result<Option<MemoryCandidate>, rusqlite::Error> {
    let query = format!(
        "{MEMORY_CANDIDATE_SELECT}
         WHERE c.id=?1"
    );
    connection
        .query_row(&query, [candidate_id], map_memory_candidate_row)
        .optional()
}

fn map_timer_row(row: &rusqlite::Row<'_>) -> Result<TimerItem, rusqlite::Error> {
    Ok(TimerItem {
        id: row.get(0)?,
        title: row.get(1)?,
        due_at_ms: row.get(2)?,
        timezone: row.get(3)?,
        original_request: row.get(4)?,
        status: row.get(5)?,
        created_at_ms: row.get(6)?,
    })
}

fn map_timer_history_row(row: &rusqlite::Row<'_>) -> Result<TimerHistoryItem, rusqlite::Error> {
    Ok(TimerHistoryItem {
        id: row.get(0)?,
        title: row.get(1)?,
        due_at_ms: row.get(2)?,
        status: row.get(3)?,
        completed_at_ms: row.get(4)?,
        created_at_ms: row.get(5)?,
    })
}

fn map_tool_run_row(row: &rusqlite::Row<'_>) -> Result<ToolRunSummary, rusqlite::Error> {
    Ok(ToolRunSummary {
        id: row.get(0)?,
        tool_name: row.get(1)?,
        origin: row.get(2)?,
        status: row.get(3)?,
        result_summary: row.get(4)?,
        created_at_ms: row.get(5)?,
        completed_at_ms: row.get(6)?,
    })
}

fn normalize_memory_text(value: &str) -> String {
    value
        .nfc()
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn unix_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn memory_valid_until_for_kind(kind: &str, created_at_ms: i64) -> Option<i64> {
    let ttl_ms = match kind {
        "goal" => Some(90 * 24 * 60 * 60 * 1_000),
        "project" => Some(180 * 24 * 60 * 60 * 1_000),
        _ => None,
    }?;
    Some(created_at_ms.saturating_add(ttl_ms))
}

fn memory_fts_query(value: &str) -> Option<String> {
    let mut terms = Vec::<String>::new();
    for segment in value.split(|character: char| !character.is_alphanumeric()) {
        let characters = segment.chars().collect::<Vec<_>>();
        if characters.len() < 3 {
            continue;
        }
        if characters.iter().all(char::is_ascii) {
            terms.push(segment.to_owned());
        } else {
            for window in characters.windows(3) {
                terms.push(window.iter().collect());
                if terms.len() >= 12 {
                    break;
                }
            }
        }
        if terms.len() >= 12 {
            break;
        }
    }
    terms.sort();
    terms.dedup();
    (!terms.is_empty()).then(|| {
        terms
            .into_iter()
            .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
            .collect::<Vec<_>>()
            .join(" OR ")
    })
}

fn lexical_memory_fallback(
    connection: &Connection,
    query: &str,
    character_id: &str,
    limit: i64,
    normal_only: bool,
    now_ms: i64,
) -> Result<Vec<MemoryItem>, rusqlite::Error> {
    let query_terms = memory_lexical_terms(query);
    if query_terms.is_empty() {
        return Ok(Vec::new());
    }
    let minimum_overlap = if query_terms.len() >= 4 { 2 } else { 1 };
    let sensitivity_clause = if normal_only {
        "AND m.sensitivity='normal'"
    } else {
        ""
    };
    let query = format!(
        "SELECT {MEMORY_SELECT_COLUMNS}
           FROM memories m
          WHERE (m.scope='user' OR (m.scope='relationship' AND m.character_id=?1))
            AND (m.valid_until_ms IS NULL OR m.valid_until_ms > ?2)
            {sensitivity_clause}
            AND NOT EXISTS (SELECT 1 FROM memories newer WHERE newer.supersedes_id=m.id)
          ORDER BY m.importance DESC, m.updated_at_ms DESC
          LIMIT 200"
    );
    let mut statement = connection.prepare(&query)?;
    let rows = statement.query_map(params![character_id, now_ms], map_memory_row)?;
    let mut scored = rows
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter_map(|memory| {
            let content_terms = memory_lexical_terms(&memory.content);
            let overlap = query_terms.intersection(&content_terms).count();
            (overlap >= minimum_overlap).then_some((overlap, memory))
        })
        .collect::<Vec<_>>();
    scored.sort_by(|(left_score, left), (right_score, right)| {
        right_score
            .cmp(left_score)
            .then_with(|| right.importance.total_cmp(&left.importance))
            .then_with(|| right.updated_at_ms.cmp(&left.updated_at_ms))
    });
    Ok(scored
        .into_iter()
        .take(limit as usize)
        .map(|(_, memory)| memory)
        .collect())
}

fn memory_lexical_terms(value: &str) -> HashSet<String> {
    let normalized = normalize_memory_text(value);
    let mut terms = HashSet::new();
    for segment in normalized.split(|character: char| !character.is_alphanumeric()) {
        let characters = segment.chars().collect::<Vec<_>>();
        if characters.iter().all(char::is_ascii) {
            if characters.len() >= 2 {
                terms.insert(segment.to_owned());
            }
        } else {
            terms.extend(
                characters
                    .windows(2)
                    .map(|window| window.iter().collect::<String>()),
            );
        }
    }
    terms
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_and_turn_round_trip() {
        let storage = Storage::open_in_memory().unwrap();
        let conversation = storage.ensure_conversation(1).unwrap();
        assert_eq!(storage.latest_turn_generation(&conversation).unwrap(), 0);
        storage
            .begin_turn(&conversation, "turn-1", 1, "text", "你好", 2)
            .unwrap();
        assert_eq!(storage.latest_turn_generation(&conversation).unwrap(), 1);
        storage.mark_turn_active("turn-1").unwrap();
        storage
            .complete_turn(&conversation, "aris", "turn-1", "老师，爱丽丝已就绪。", 3)
            .unwrap();
        let messages = storage.list_messages(&conversation, 20).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].canonical_text, "你好");
        assert_eq!(messages[1].role, MessageRole::Assistant);
        let relationship = storage.load_relationship_state("aris").unwrap();
        assert_eq!(relationship.interaction_count, 1);
        assert!((relationship.familiarity - 0.02).abs() < f64::EPSILON);
    }

    #[test]
    fn relationship_state_counts_completed_turns_and_caps_familiarity() {
        let storage = Storage::open_in_memory().unwrap();
        let conversation = storage.ensure_conversation(1).unwrap();

        for index in 0..60 {
            let turn_id = format!("relationship-turn-{index}");
            storage
                .begin_turn(
                    &conversation,
                    &turn_id,
                    index + 1,
                    "text",
                    "你好，爱丽丝。",
                    index as i64 + 2,
                )
                .unwrap();
            storage
                .complete_turn(
                    &conversation,
                    "aris",
                    &turn_id,
                    "老师，爱丽丝收到。",
                    index as i64 + 3,
                )
                .unwrap();
        }

        let relationship = storage.load_relationship_state("aris").unwrap();
        assert_eq!(relationship.interaction_count, 60);
        assert!((relationship.familiarity - 1.0).abs() < f64::EPSILON);
        assert_eq!(relationship.version, 60);
        assert_eq!(relationship.affinity, 0.0);

        storage
            .begin_turn(
                &conversation,
                "relationship-cancelled",
                61,
                "text",
                "取消这次对话",
                100,
            )
            .unwrap();
        storage
            .finish_turn_with_status(
                "relationship-cancelled",
                "cancelled",
                Some("user_cancelled"),
                "",
                &conversation,
                101,
            )
            .unwrap();
        assert_eq!(
            storage
                .load_relationship_state("aris")
                .unwrap()
                .interaction_count,
            60
        );
    }

    #[test]
    fn relationship_signals_are_bounded_without_affecting_familiarity() {
        let storage = Storage::open_in_memory().unwrap();

        storage
            .apply_relationship_signal("aris", 0.08, "最近互动积极", 10)
            .unwrap();
        let first = storage.load_relationship_state("aris").unwrap();
        assert!((first.affinity - 0.08).abs() < f64::EPSILON);
        assert_eq!(first.familiarity, 0.0);
        assert_eq!(first.interaction_count, 0);
        assert_eq!(first.summary, "最近互动积极");

        storage
            .apply_relationship_signal("aris", 2.0, "", 11)
            .unwrap();
        let upper = storage.load_relationship_state("aris").unwrap();
        assert_eq!(upper.affinity, 0.18);
        assert_eq!(upper.summary, "最近互动积极");

        storage
            .apply_relationship_signal("aris", -2.0, "最近出现负面情绪", 12)
            .unwrap();
        let lower = storage.load_relationship_state("aris").unwrap();
        assert!((lower.affinity - 0.08).abs() < f64::EPSILON);
        assert_eq!(lower.summary, "最近出现负面情绪");
        assert_eq!(lower.familiarity, 0.0);
    }

    #[test]
    fn user_message_for_turn_round_trips_the_original_input() {
        let storage = Storage::open_in_memory().unwrap();
        let conversation = storage.ensure_conversation(1).unwrap();
        let created = storage
            .begin_turn(
                &conversation,
                "source-message-turn",
                1,
                "text",
                "老师，我喜欢蓝色。",
                10,
            )
            .unwrap();

        let loaded = storage
            .user_message_for_turn(&conversation, "source-message-turn")
            .unwrap()
            .unwrap();
        assert_eq!(loaded.id, created.id);
        assert_eq!(loaded.turn_id, "source-message-turn");
        assert_eq!(loaded.role, MessageRole::User);
        assert_eq!(loaded.status, MessageStatus::Completed);
        assert_eq!(loaded.canonical_text, "老师，我喜欢蓝色。");
    }

    #[test]
    fn scope_constraint_rejects_invalid_user_character_pair() {
        let storage = Storage::open_in_memory().unwrap();
        let connection = storage.connection.lock().unwrap();
        let result = connection.execute(
            "INSERT INTO memories(id, scope, character_id, kind, content, normalized_key, confidence, sensitivity, created_at_ms, updated_at_ms)
             VALUES('m1', 'user', 'aris', 'preference', 'x', 'x', 1, 'normal', 1, 1)",
            [],
        );
        assert!(result.is_err());
    }

    #[test]
    fn pet_settings_round_trip() {
        let storage = Storage::open_in_memory().unwrap();
        let settings = PetSettings {
            visible: false,
            click_through: true,
            scale: 1.2,
            revision: 3,
        };
        storage.save_pet_settings(&settings, 10).unwrap();
        assert_eq!(storage.load_pet_settings().unwrap(), settings);
    }

    #[test]
    fn memory_settings_round_trip() {
        let storage = Storage::open_in_memory().unwrap();
        let settings = MemorySettings {
            automatic_candidates_enabled: false,
            revision: 4,
        };
        storage.save_memory_settings(&settings, 10).unwrap();
        assert_eq!(storage.load_memory_settings().unwrap(), settings);
    }

    #[test]
    fn character_settings_round_trip() {
        let storage = Storage::open_in_memory().unwrap();
        let settings = CharacterSettings {
            model_path: Some("/managed/model.vrm".into()),
            model_file_name: Some("model.vrm".into()),
            model_size_bytes: Some(42),
            revision: 2,
            ..CharacterSettings::default()
        };
        storage.save_character_settings(&settings, 10).unwrap();
        assert_eq!(storage.load_character_settings().unwrap(), settings);
    }

    #[test]
    fn memory_fts_update_and_delete_are_transactional() {
        let storage = Storage::open_in_memory().unwrap();
        let created = storage.create_user_memory("我喜欢蓝色", 10).unwrap();
        let found = storage
            .search_memories("我喜欢什么颜色", "aris", 8)
            .unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].id, created.id);

        let updated = storage
            .update_memory(&created.id, "我喜欢绿色", created.version, 11)
            .unwrap()
            .unwrap();
        assert!(storage
            .search_memories("欢蓝色", "aris", 8)
            .unwrap()
            .is_empty());
        assert_eq!(
            storage.search_memories("欢绿色", "aris", 8).unwrap()[0].version,
            2
        );
        assert!(storage
            .delete_memory(&updated.id, updated.version, 12)
            .unwrap());
        assert!(storage
            .search_memories("欢绿色", "aris", 8)
            .unwrap()
            .is_empty());

        let connection = storage.connection.lock().unwrap();
        let audit_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM memory_audit", [], |row| row.get(0))
            .unwrap();
        assert_eq!(audit_count, 3);
    }

    #[test]
    fn direct_memory_creation_reuses_an_exact_existing_memory() {
        let storage = Storage::open_in_memory().unwrap();
        let first = storage.create_user_memory("我喜欢蓝色", 10).unwrap();
        let second = storage.create_user_memory("  我喜欢蓝色 ", 20).unwrap();

        assert_eq!(second.id, first.id);
        assert_eq!(second.version, first.version);
        assert!(storage
            .has_duplicate_user_memory("我喜欢蓝色", "another-memory")
            .unwrap());
        assert!(!storage
            .has_duplicate_user_memory("我喜欢蓝色", &first.id)
            .unwrap());
        assert_eq!(
            storage
                .search_memories("喜欢蓝色", "aris", 20)
                .unwrap()
                .len(),
            1
        );
        let connection = storage.connection.lock().unwrap();
        let audit_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM memory_audit", [], |row| row.get(0))
            .unwrap();
        assert_eq!(audit_count, 1);
    }

    #[test]
    fn pending_candidate_exposes_earlier_exact_duplicate() {
        let storage = Storage::open_in_memory().unwrap();
        let conversation = storage.ensure_conversation(1).unwrap();
        storage
            .begin_turn(&conversation, "duplicate-turn", 1, "text", "记住蓝色", 2)
            .unwrap();
        storage
            .create_memory_candidate(MemoryCandidateInput {
                id: "candidate-first",
                source_turn_id: "duplicate-turn",
                source_message_id: None,
                scope: "user",
                kind: "note",
                content: "我喜欢蓝色",
                confidence: 0.8,
                sensitivity: "normal",
                created_at_ms: 3,
            })
            .unwrap();
        let second = storage
            .create_memory_candidate(MemoryCandidateInput {
                id: "candidate-second",
                source_turn_id: "duplicate-turn",
                source_message_id: None,
                scope: "user",
                kind: "note",
                content: "  我喜欢蓝色 ",
                confidence: 0.7,
                sensitivity: "normal",
                created_at_ms: 4,
            })
            .unwrap();

        assert_eq!(second.duplicate_of.as_deref(), Some("candidate-first"));
        assert_eq!(second.duplicate_content.as_deref(), Some("我喜欢蓝色"));
        let candidates = storage.list_pending_memory_candidates(20).unwrap();
        assert!(candidates
            .iter()
            .find(|candidate| candidate.id == "candidate-first")
            .unwrap()
            .duplicate_of
            .is_none());
        assert_eq!(
            candidates
                .iter()
                .filter(|candidate| candidate.duplicate_of.is_some())
                .count(),
            1
        );
    }

    #[test]
    fn confirming_duplicate_candidate_does_not_create_a_second_memory() {
        let storage = Storage::open_in_memory().unwrap();
        let existing = storage.create_user_memory("我喜欢蓝色", 1).unwrap();
        let conversation = storage.ensure_conversation(2).unwrap();
        storage
            .begin_turn(
                &conversation,
                "duplicate-confirm-turn",
                1,
                "text",
                "记住蓝色",
                3,
            )
            .unwrap();
        storage
            .create_memory_candidate(MemoryCandidateInput {
                id: "candidate-confirm-duplicate",
                source_turn_id: "duplicate-confirm-turn",
                source_message_id: None,
                scope: "user",
                kind: "note",
                content: "我喜欢蓝色",
                confidence: 0.9,
                sensitivity: "normal",
                created_at_ms: 4,
            })
            .unwrap();

        let confirmed = storage
            .confirm_memory_candidate("candidate-confirm-duplicate", "aris", 5)
            .unwrap()
            .unwrap();
        assert_eq!(confirmed.id, existing.id);
        assert!(storage
            .list_pending_memory_candidates(20)
            .unwrap()
            .is_empty());
        let connection = storage.connection.lock().unwrap();
        let memory_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM memories", [], |row| row.get(0))
            .unwrap();
        assert_eq!(memory_count, 1);
    }

    #[test]
    fn memory_candidates_are_reviewed_into_formal_memories() {
        let storage = Storage::open_in_memory().unwrap();
        let conversation = storage.ensure_conversation(1).unwrap();
        storage
            .begin_turn(&conversation, "candidate-turn", 1, "text", "记住蓝色", 2)
            .unwrap();

        let candidate = storage
            .create_memory_candidate(MemoryCandidateInput {
                id: "candidate-1",
                source_turn_id: "candidate-turn",
                source_message_id: None,
                scope: "user",
                kind: "preference",
                content: "老师喜欢蓝色",
                confidence: 0.8,
                sensitivity: "normal",
                created_at_ms: 3,
            })
            .unwrap();
        assert_eq!(candidate.status, "pending");
        assert_eq!(
            storage.list_pending_memory_candidates(20).unwrap(),
            vec![candidate]
        );

        let edited = storage
            .update_memory_candidate("candidate-1", "老师喜欢青色")
            .unwrap()
            .unwrap();
        assert_eq!(edited.content, "老师喜欢青色");

        let memory = storage
            .confirm_memory_candidate("candidate-1", "aris", 4)
            .unwrap()
            .unwrap();
        assert_eq!(memory.content, "老师喜欢青色");
        assert!(storage
            .list_pending_memory_candidates(20)
            .unwrap()
            .is_empty());
        assert_eq!(
            storage.search_memories("喜欢青色", "aris", 8).unwrap()[0].id,
            memory.id
        );
        assert!(storage
            .update_memory_candidate("candidate-1", "不应再次编辑")
            .unwrap()
            .is_none());

        let connection = storage.connection.lock().unwrap();
        let status: String = connection
            .query_row(
                "SELECT status FROM memory_candidates WHERE id='candidate-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "confirmed");
    }

    #[test]
    fn memory_v2_candidate_conflict_requires_an_explicit_resolution() {
        let storage = Storage::open_in_memory().unwrap();
        let conversation = storage.ensure_conversation(unix_now_ms()).unwrap();
        storage
            .begin_turn(
                &conversation,
                "v2-memory-turn-1",
                1,
                "text",
                "我喜欢咖啡",
                unix_now_ms(),
            )
            .unwrap();
        storage
            .create_memory_candidate_with_metadata(MemoryCandidateV2Input {
                id: "v2-candidate-1",
                source_turn_id: "v2-memory-turn-1",
                source_message_id: None,
                scope: "user",
                kind: "preference",
                content: "我喜欢咖啡",
                confidence: 0.9,
                sensitivity: "normal",
                origin: "user_explicit",
                reason: "用户明确要求保存",
                fact_key: Some("user:preference:咖啡"),
                proposed_action: "create",
                expires_at_ms: Some(unix_now_ms() + 60_000),
                created_at_ms: unix_now_ms(),
            })
            .unwrap();
        let first = storage
            .confirm_memory_candidate_with_resolution("v2-candidate-1", "aris", None, unix_now_ms())
            .unwrap();
        assert!(matches!(first, MemoryConfirmResult::Saved(_)));

        storage
            .begin_turn(
                &conversation,
                "v2-memory-turn-2",
                2,
                "text",
                "我不喜欢咖啡",
                unix_now_ms(),
            )
            .unwrap();
        let second = storage
            .create_memory_candidate_with_metadata(MemoryCandidateV2Input {
                id: "v2-candidate-2",
                source_turn_id: "v2-memory-turn-2",
                source_message_id: None,
                scope: "user",
                kind: "preference",
                content: "我不喜欢咖啡",
                confidence: 0.9,
                sensitivity: "normal",
                origin: "user_explicit",
                reason: "用户明确要求保存",
                fact_key: Some("user:preference:咖啡"),
                proposed_action: "replace",
                expires_at_ms: Some(unix_now_ms() + 60_000),
                created_at_ms: unix_now_ms(),
            })
            .unwrap();
        assert!(second.conflict_of.is_some());
        let conflict = storage
            .confirm_memory_candidate_with_resolution("v2-candidate-2", "aris", None, unix_now_ms())
            .unwrap();
        assert!(matches!(conflict, MemoryConfirmResult::Conflict(_)));

        let replaced = storage
            .confirm_memory_candidate_with_resolution(
                "v2-candidate-2",
                "aris",
                Some(MemoryConflictResolution::Replace),
                unix_now_ms(),
            )
            .unwrap();
        let MemoryConfirmResult::Saved(replaced) = replaced else {
            panic!("expected replacement to save");
        };
        assert_eq!(replaced.content, "我不喜欢咖啡");
        let memories = storage.search_memories("咖啡", "aris", 20).unwrap();
        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0].content, "我不喜欢咖啡");
    }

    #[test]
    fn expired_candidates_and_sensitive_context_are_filtered() {
        let storage = Storage::open_in_memory().unwrap();
        let now = unix_now_ms();
        let conversation = storage.ensure_conversation(now).unwrap();
        storage
            .begin_turn(&conversation, "v2-expiry-turn", 1, "text", "过期候选", now)
            .unwrap();
        storage
            .create_memory_candidate_with_metadata(MemoryCandidateV2Input {
                id: "v2-expired-candidate",
                source_turn_id: "v2-expiry-turn",
                source_message_id: None,
                scope: "user",
                kind: "note",
                content: "过期候选",
                confidence: 0.5,
                sensitivity: "normal",
                origin: "automatic_candidate",
                reason: "过期测试",
                fact_key: None,
                proposed_action: "create",
                expires_at_ms: Some(now - 1),
                created_at_ms: now - 2,
            })
            .unwrap();
        assert!(storage
            .list_pending_memory_candidates(20)
            .unwrap()
            .is_empty());

        storage
            .begin_turn(
                &conversation,
                "v2-sensitive-turn",
                2,
                "text",
                "我的住址是某地",
                now,
            )
            .unwrap();
        storage
            .create_memory_candidate_with_metadata(MemoryCandidateV2Input {
                id: "v2-sensitive-candidate",
                source_turn_id: "v2-sensitive-turn",
                source_message_id: None,
                scope: "user",
                kind: "note",
                content: "我的住址是某地",
                confidence: 1.0,
                sensitivity: "sensitive",
                origin: "user_explicit",
                reason: "用户明确要求保存",
                fact_key: None,
                proposed_action: "create",
                expires_at_ms: Some(now + 60_000),
                created_at_ms: now,
            })
            .unwrap();
        let MemoryConfirmResult::Saved(sensitive) = storage
            .confirm_memory_candidate_with_resolution(
                "v2-sensitive-candidate",
                "aris",
                None,
                now + 1,
            )
            .unwrap()
        else {
            panic!("expected sensitive memory to save");
        };
        assert_eq!(
            storage.search_memories("住址", "aris", 20).unwrap().len(),
            1
        );
        assert!(storage
            .search_memories_for_context("住址", "aris", 20)
            .unwrap()
            .is_empty());
        assert_eq!(sensitive.sensitivity, "sensitive");
    }

    #[test]
    fn duplicate_memory_or_candidate_detection_is_scoped_and_normalized() {
        let storage = Storage::open_in_memory().unwrap();
        assert!(!storage
            .has_duplicate_memory_or_candidate("我喜欢蓝色", "user", "aris")
            .unwrap());

        storage.create_user_memory("我喜欢蓝色。", 1).unwrap();
        assert!(storage
            .has_duplicate_memory_or_candidate("  我喜欢蓝色。  ", "user", "aris")
            .unwrap());

        let conversation = storage.ensure_conversation(1).unwrap();
        storage
            .begin_turn(
                &conversation,
                "automatic-memory-turn",
                1,
                "text",
                "我喜欢绿色",
                2,
            )
            .unwrap();
        storage
            .create_memory_candidate(MemoryCandidateInput {
                id: "automatic-memory-candidate",
                source_turn_id: "automatic-memory-turn",
                source_message_id: None,
                scope: "user",
                kind: "preference",
                content: "我喜欢绿色",
                confidence: 0.8,
                sensitivity: "normal",
                created_at_ms: 3,
            })
            .unwrap();
        assert!(storage
            .has_duplicate_memory_or_candidate("我喜欢绿色", "user", "another-character")
            .unwrap());
    }

    #[test]
    fn relationship_memory_candidate_is_scoped_to_the_active_character() {
        let storage = Storage::open_in_memory().unwrap();
        let conversation = storage.ensure_conversation(1).unwrap();
        storage
            .begin_turn(
                &conversation,
                "relationship-memory-turn",
                1,
                "text",
                "记住我们第一次并肩完成任务",
                2,
            )
            .unwrap();
        storage
            .create_memory_candidate(MemoryCandidateInput {
                id: "relationship-candidate",
                source_turn_id: "relationship-memory-turn",
                source_message_id: None,
                scope: "relationship",
                kind: "event",
                content: "我们第一次并肩完成任务",
                confidence: 1.0,
                sensitivity: "normal",
                created_at_ms: 3,
            })
            .unwrap();

        let memory = storage
            .confirm_memory_candidate("relationship-candidate", "aris", 4)
            .unwrap()
            .unwrap();
        assert_eq!(memory.scope, "relationship");
        assert_eq!(memory.character_id.as_deref(), Some("aris"));
        assert_eq!(memory.kind, "event");
        assert_eq!(
            storage
                .search_memories("第一次并肩", "aris", 8)
                .unwrap()
                .first()
                .map(|item| item.id.as_str()),
            Some(memory.id.as_str())
        );
        assert!(storage
            .search_memories("第一次并肩", "another-character", 8)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn memory_candidate_rejection_is_idempotent() {
        let storage = Storage::open_in_memory().unwrap();
        let conversation = storage.ensure_conversation(1).unwrap();
        storage
            .begin_turn(&conversation, "reject-turn", 1, "text", "记住红色", 2)
            .unwrap();
        storage
            .create_memory_candidate(MemoryCandidateInput {
                id: "candidate-reject",
                source_turn_id: "reject-turn",
                source_message_id: None,
                scope: "user",
                kind: "note",
                content: "红色",
                confidence: 0.6,
                sensitivity: "normal",
                created_at_ms: 3,
            })
            .unwrap();

        assert!(storage
            .reject_memory_candidate("candidate-reject", 4)
            .unwrap());
        assert!(!storage
            .reject_memory_candidate("candidate-reject", 5)
            .unwrap());
        assert!(storage
            .list_pending_memory_candidates(20)
            .unwrap()
            .is_empty());
        assert!(storage
            .confirm_memory_candidate("candidate-reject", "aris", 6)
            .unwrap()
            .is_none());
    }

    #[test]
    fn data_cleanup_keeps_scopes_independent_and_rebuilds_fts() {
        let storage = Storage::open_in_memory().unwrap();
        let conversation = storage.ensure_conversation(1).unwrap();
        storage
            .begin_turn(&conversation, "turn-cleanup", 1, "text", "记住蓝色", 2)
            .unwrap();
        storage
            .complete_turn(&conversation, "aris", "turn-cleanup", "已记录。", 3)
            .unwrap();
        storage.create_user_memory("老师喜欢蓝色", 4).unwrap();
        {
            let connection = storage.connection.lock().unwrap();
            connection
                .execute(
                    "INSERT INTO memory_candidates(id, source_turn_id, scope, kind, content, normalized_key, confidence, sensitivity, status, created_at_ms)
                     VALUES('candidate-cleanup', 'turn-cleanup', 'user', 'preference', '蓝色', '蓝色', 1, 'normal', 'pending', 4)",
                    [],
                )
                .unwrap();
        }

        assert_eq!(
            storage.data_summary(&conversation).unwrap(),
            DataSummary {
                message_count: 2,
                memory_count: 1,
                memory_candidate_count: 1,
            }
        );
        storage
            .clear_conversation_history(&conversation, 5)
            .unwrap();
        assert_eq!(
            storage.data_summary(&conversation).unwrap().message_count,
            0
        );
        assert_eq!(storage.data_summary(&conversation).unwrap().memory_count, 1);
        assert_eq!(
            storage
                .data_summary(&conversation)
                .unwrap()
                .memory_candidate_count,
            0
        );
        assert_eq!(
            storage
                .search_memories("喜欢蓝色", "aris", 8)
                .unwrap()
                .len(),
            1
        );

        storage.clear_all_memories().unwrap();
        assert_eq!(storage.data_summary(&conversation).unwrap().memory_count, 0);
        assert!(storage
            .search_memories("喜欢蓝色", "aris", 8)
            .unwrap()
            .is_empty());
        let connection = storage.connection.lock().unwrap();
        let audit_count: u64 = connection
            .query_row("SELECT COUNT(*) FROM memory_audit", [], |row| row.get(0))
            .unwrap();
        assert_eq!(audit_count, 0);
    }

    #[test]
    fn tool_runs_record_approval_and_result() {
        let storage = Storage::open_in_memory().unwrap();
        let conversation = storage.ensure_conversation(1).unwrap();
        storage
            .begin_turn(
                &conversation,
                "tool-turn",
                1,
                "text",
                "打开 https://example.com",
                2,
            )
            .unwrap();
        storage
            .begin_tool_run(
                ToolRunInput {
                    id: "tool-1",
                    conversation_id: &conversation,
                    turn_id: "tool-turn",
                    tool_name: "system.open_url",
                    arguments_json: r#"{"url":"https://example.com/"}"#,
                    origin: "user_explicit",
                },
                3,
            )
            .unwrap();
        storage
            .finish_tool_run("tool-1", "approved", "用户允许", 4)
            .unwrap();
        storage
            .finish_tool_run("tool-1", "succeeded", "浏览器已打开", 5)
            .unwrap();

        let connection = storage.connection.lock().unwrap();
        let row: (String, String) = connection
            .query_row(
                "SELECT status, result_summary FROM tool_runs WHERE id='tool-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(row, ("succeeded".into(), "浏览器已打开".into()));
        drop(connection);

        let history = storage.list_tool_runs(&conversation, 20).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].tool_name, "system.open_url");
        assert_eq!(history[0].origin, "user_explicit");
        assert_eq!(history[0].status, "succeeded");
        assert_eq!(history[0].result_summary.as_deref(), Some("浏览器已打开"));
        assert_eq!(history[0].created_at_ms, 3);
        assert_eq!(history[0].completed_at_ms, Some(5));
    }

    #[test]
    fn timers_round_trip_and_transition_once() {
        let storage = Storage::open_in_memory().unwrap();
        let conversation = storage.ensure_conversation(1).unwrap();
        storage
            .begin_turn(&conversation, "timer-turn", 1, "text", "提醒我喝水", 2)
            .unwrap();
        storage
            .begin_tool_run(
                ToolRunInput {
                    id: "timer-tool",
                    conversation_id: &conversation,
                    turn_id: "timer-turn",
                    tool_name: "timer.create",
                    arguments_json: r#"{"title":"喝水","due_at_ms":100}"#,
                    origin: "user_explicit",
                },
                3,
            )
            .unwrap();
        let timer = storage
            .create_timer(
                TimerInput {
                    id: "timer-1",
                    conversation_id: &conversation,
                    tool_run_id: "timer-tool",
                    title: "喝水",
                    due_at_ms: 100,
                    timezone: "local",
                    original_request: "30 秒后提醒我喝水",
                },
                4,
            )
            .unwrap();
        assert_eq!(
            storage.list_pending_timers(20).unwrap(),
            vec![timer.clone()]
        );
        assert!(storage.claim_due_timer("timer-1", 99).unwrap().is_none());
        let fired = storage.claim_due_timer("timer-1", 100).unwrap().unwrap();
        assert_eq!(fired.status, "fired");
        assert!(storage.claim_due_timer("timer-1", 101).unwrap().is_none());

        storage
            .begin_turn(
                &conversation,
                "timer-turn-2",
                2,
                "text",
                "设置第二个提醒",
                5,
            )
            .unwrap();
        storage
            .begin_tool_run(
                ToolRunInput {
                    id: "timer-tool-2",
                    conversation_id: &conversation,
                    turn_id: "timer-turn-2",
                    tool_name: "timer.create",
                    arguments_json: r#"{"title":"取消我","due_at_ms":200}"#,
                    origin: "user_explicit",
                },
                5,
            )
            .unwrap();
        let cancelled = storage
            .create_timer(
                TimerInput {
                    id: "timer-2",
                    conversation_id: &conversation,
                    tool_run_id: "timer-tool-2",
                    title: "取消我",
                    due_at_ms: 200,
                    timezone: "local",
                    original_request: "取消测试",
                },
                5,
            )
            .unwrap();
        assert_eq!(cancelled.status, "pending");
        assert!(storage.cancel_timer("timer-2", 6).unwrap());
        assert!(!storage.cancel_timer("timer-2", 7).unwrap());
        assert!(storage.list_pending_timers(20).unwrap().is_empty());

        let history = storage.list_timer_history(20).unwrap();
        assert_eq!(history.len(), 2);
        let fired_history = history.iter().find(|item| item.id == "timer-1").unwrap();
        assert_eq!(fired_history.status, "fired");
        assert_eq!(fired_history.completed_at_ms, Some(100));
        let cancelled_history = history.iter().find(|item| item.id == "timer-2").unwrap();
        assert_eq!(cancelled_history.status, "cancelled");
        assert_eq!(cancelled_history.completed_at_ms, Some(6));
    }
}
