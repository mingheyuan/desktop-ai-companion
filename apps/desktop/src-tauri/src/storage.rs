use std::{path::Path, sync::Mutex};

use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::contracts::{ChatMessage, MessageRole, MessageStatus, PetSettings, ProviderSettings};

pub struct Storage {
    connection: Mutex<Connection>,
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
        if current_version > 1 {
            return Err(rusqlite::Error::InvalidQuery);
        }
        if current_version == 1 {
            return Ok(());
        }

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
        Ok(())
    }

    pub fn begin_text_turn(
        &self,
        conversation_id: &str,
        turn_id: &str,
        generation: u64,
        text: &str,
        now_ms: i64,
    ) -> Result<ChatMessage, rusqlite::Error> {
        let mut connection = self.connection.lock().expect("storage mutex poisoned");
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO turns(id, conversation_id, generation, input_source, status, started_at_ms)
             VALUES(?1, ?2, ?3, 'text', 'preparing', ?4)",
            params![turn_id, conversation_id, generation as i64, now_ms],
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversation_and_turn_round_trip() {
        let storage = Storage::open_in_memory().unwrap();
        let conversation = storage.ensure_conversation(1).unwrap();
        storage
            .begin_text_turn(&conversation, "turn-1", 1, "你好", 2)
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
}
