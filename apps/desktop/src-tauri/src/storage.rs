use std::{collections::HashSet, path::Path, sync::Mutex};

use rusqlite::{params, Connection, OptionalExtension};
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

use crate::contracts::{
    CharacterSettings, ChatMessage, DataSummary, MemoryItem, MessageRole, MessageStatus,
    PetSettings, ProviderSettings, TimerItem, VoiceSettings,
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
        if current_version > 4 {
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

    pub fn complete_turn(
        &self,
        conversation_id: &str,
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
                value.has_api_key = std::env::var("DESKTOP_AI_LLM_API_KEY").is_ok();
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

    pub fn create_user_memory(
        &self,
        content: &str,
        now_ms: i64,
    ) -> Result<MemoryItem, rusqlite::Error> {
        let mut connection = self.connection.lock().expect("storage mutex poisoned");
        let transaction = connection.transaction()?;
        let item = MemoryItem {
            id: Uuid::now_v7().to_string(),
            scope: "user".into(),
            character_id: None,
            kind: "note".into(),
            content: content.to_owned(),
            importance: 0.7,
            version: 1,
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
        };
        let normalized_key = normalize_memory_text(content);
        transaction.execute(
            "INSERT INTO memories(id, scope, character_id, kind, content, normalized_key, importance, confidence, sensitivity, version, created_at_ms, updated_at_ms)
             VALUES(?1, 'user', NULL, 'note', ?2, ?3, ?4, 1, 'normal', 1, ?5, ?5)",
            params![item.id, item.content, normalized_key, item.importance, now_ms],
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

    pub fn update_memory(
        &self,
        id: &str,
        content: &str,
        expected_version: u64,
        now_ms: i64,
    ) -> Result<Option<MemoryItem>, rusqlite::Error> {
        let mut connection = self.connection.lock().expect("storage mutex poisoned");
        let transaction = connection.transaction()?;
        let previous = transaction
            .query_row(
                "SELECT rowid, scope, character_id, kind, content, normalized_key, importance, version, created_at_ms
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
            "UPDATE memories SET content=?1, normalized_key=?2, version=version+1, updated_at_ms=?3 WHERE id=?4 AND version=?5",
            params![content, normalized_key, now_ms, id, expected_version],
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
            version: version + 1,
            created_at_ms,
            updated_at_ms: now_ms,
        }))
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
        let connection = self.connection.lock().expect("storage mutex poisoned");
        let limit = limit.clamp(1, 20) as i64;
        let normalized = normalize_memory_text(query);
        let fts = memory_fts_query(&normalized);
        let mut items = if normalized.is_empty() {
            let mut statement = connection.prepare(
                "SELECT id, scope, character_id, kind, content, importance, version, created_at_ms, updated_at_ms
                 FROM memories m
                 WHERE (m.scope='user' OR (m.scope='relationship' AND m.character_id=?1))
                   AND NOT EXISTS (SELECT 1 FROM memories newer WHERE newer.supersedes_id=m.id)
                 ORDER BY m.updated_at_ms DESC LIMIT ?2",
            )?;
            let rows = statement.query_map(params![character_id, limit], map_memory_row)?;
            rows.collect::<Result<Vec<_>, _>>()?
        } else if let Some(fts) = fts {
            let mut statement = connection.prepare(
                "SELECT m.id, m.scope, m.character_id, m.kind, m.content, m.importance, m.version, m.created_at_ms, m.updated_at_ms
                 FROM memory_fts JOIN memories m ON m.rowid=memory_fts.rowid
                 WHERE memory_fts MATCH ?1
                   AND (m.scope='user' OR (m.scope='relationship' AND m.character_id=?2))
                   AND NOT EXISTS (SELECT 1 FROM memories newer WHERE newer.supersedes_id=m.id)
                 ORDER BY bm25(memory_fts), m.importance DESC, m.updated_at_ms DESC LIMIT ?3",
            )?;
            let rows = statement.query_map(params![fts, character_id, limit], map_memory_row)?;
            rows.collect::<Result<Vec<_>, _>>()?
        } else {
            let prefix = format!("{normalized}%");
            let contains = format!("%{normalized}%");
            let mut statement = connection.prepare(
                "SELECT id, scope, character_id, kind, content, importance, version, created_at_ms, updated_at_ms
                 FROM memories m
                 WHERE (m.scope='user' OR (m.scope='relationship' AND m.character_id=?1))
                   AND (m.normalized_key LIKE ?2 ESCAPE '\\' OR lower(m.content) LIKE ?3 ESCAPE '\\')
                   AND NOT EXISTS (SELECT 1 FROM memories newer WHERE newer.supersedes_id=m.id)
                 ORDER BY m.importance DESC, m.updated_at_ms DESC LIMIT ?4",
            )?;
            let rows = statement.query_map(
                params![character_id, prefix, contains, limit],
                map_memory_row,
            )?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        if !normalized.is_empty() && items.is_empty() {
            items = lexical_memory_fallback(&connection, &normalized, character_id, limit)?;
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
                "SELECT COUNT(*) FROM memory_candidates",
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
        version: row.get(6)?,
        created_at_ms: row.get(7)?,
        updated_at_ms: row.get(8)?,
    })
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

fn normalize_memory_text(value: &str) -> String {
    value
        .nfc()
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
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
) -> Result<Vec<MemoryItem>, rusqlite::Error> {
    let query_terms = memory_lexical_terms(query);
    if query_terms.is_empty() {
        return Ok(Vec::new());
    }
    let minimum_overlap = if query_terms.len() >= 4 { 2 } else { 1 };
    let mut statement = connection.prepare(
        "SELECT id, scope, character_id, kind, content, importance, version, created_at_ms, updated_at_ms
         FROM memories m
         WHERE (m.scope='user' OR (m.scope='relationship' AND m.character_id=?1))
           AND NOT EXISTS (SELECT 1 FROM memories newer WHERE newer.supersedes_id=m.id)
         ORDER BY m.importance DESC, m.updated_at_ms DESC
         LIMIT 200",
    )?;
    let rows = statement.query_map([character_id], map_memory_row)?;
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
        storage
            .begin_turn(&conversation, "turn-1", 1, "text", "你好", 2)
            .unwrap();
        storage.mark_turn_active("turn-1").unwrap();
        storage
            .complete_turn(&conversation, "turn-1", "老师，爱丽丝已就绪。", 3)
            .unwrap();
        let messages = storage.list_messages(&conversation, 20).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].canonical_text, "你好");
        assert_eq!(messages[1].role, MessageRole::Assistant);
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
    fn data_cleanup_keeps_scopes_independent_and_rebuilds_fts() {
        let storage = Storage::open_in_memory().unwrap();
        let conversation = storage.ensure_conversation(1).unwrap();
        storage
            .begin_turn(&conversation, "turn-cleanup", 1, "text", "记住蓝色", 2)
            .unwrap();
        storage
            .complete_turn(&conversation, "turn-cleanup", "已记录。", 3)
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
    }
}
