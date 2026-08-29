# SQLite 数据模型与记忆写入规则

## 1. 存储边界

- 使用 `rusqlite` 的 bundled SQLite + FTS5，开启 `foreign_keys`、WAL 和 `busy_timeout`。
- SQLite 只保存会话、消息、确认流程、非秘密设置与角色关系状态。
- API key 和 provider token 只保存到 macOS Keychain；数据库仅保存 Keychain item 的逻辑引用。
- 原始麦克风 PCM、屏幕截图和 TTS 临时音频默认不落盘。
- MemoryWorker 是 `memory_candidates`、`memories`、`memory_fts` 与 `relationship_state` 的唯一写入者。

所有 ID 使用 UUIDv7 文本；时间使用 UTC Unix milliseconds。用户输入和助手回复分开保存 canonical/display/spoken 文本，记忆提取只能读取 canonical 文本。

## 2. 启动 PRAGMA

每条新连接必须执行：

```sql
PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA busy_timeout = 5000;
PRAGMA trusted_schema = OFF;
```

写入统一经过单写队列；读取可以使用独立只读连接。正常退出执行 checkpoint，但不能以 checkpoint 完成为退出前提。

## 3. v1 核心 schema

```sql
CREATE TABLE schema_migrations (
  version       INTEGER PRIMARY KEY,
  name          TEXT NOT NULL,
  applied_at_ms INTEGER NOT NULL
);

CREATE TABLE conversations (
  id            TEXT PRIMARY KEY,
  title         TEXT,
  character_id  TEXT NOT NULL,
  created_at_ms INTEGER NOT NULL,
  updated_at_ms INTEGER NOT NULL,
  archived_at_ms INTEGER
);

CREATE TABLE turns (
  id             TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
  generation     INTEGER NOT NULL,
  input_source   TEXT NOT NULL CHECK(input_source IN ('text','wake_word','push_to_talk')),
  status         TEXT NOT NULL CHECK(status IN ('preparing','active','finalizing','completed','cancelled','failed')),
  failure_code   TEXT,
  started_at_ms  INTEGER NOT NULL,
  ended_at_ms    INTEGER,
  UNIQUE(conversation_id, generation)
);

CREATE TABLE messages (
  id              TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
  turn_id         TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
  ordinal         INTEGER NOT NULL,
  role            TEXT NOT NULL CHECK(role IN ('user','assistant','system','tool')),
  canonical_text  TEXT NOT NULL,
  display_text    TEXT,
  spoken_text     TEXT,
  status          TEXT NOT NULL CHECK(status IN ('pending','streaming','completed','cancelled','failed')),
  created_at_ms   INTEGER NOT NULL,
  completed_at_ms INTEGER,
  UNIQUE(turn_id, ordinal)
);

CREATE INDEX idx_messages_conversation_time
  ON messages(conversation_id, created_at_ms);

CREATE TABLE memory_candidates (
  id                TEXT PRIMARY KEY,
  source_turn_id    TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
  source_message_id TEXT REFERENCES messages(id) ON DELETE SET NULL,
  scope             TEXT NOT NULL CHECK(scope IN ('user','relationship')),
  kind              TEXT NOT NULL,
  content           TEXT NOT NULL,
  normalized_key    TEXT NOT NULL,
  confidence        REAL NOT NULL CHECK(confidence >= 0 AND confidence <= 1),
  sensitivity       TEXT NOT NULL CHECK(sensitivity IN ('normal','sensitive','restricted')),
  status            TEXT NOT NULL CHECK(status IN ('pending','confirmed','rejected','expired')),
  created_at_ms     INTEGER NOT NULL,
  reviewed_at_ms    INTEGER
);

CREATE INDEX idx_memory_candidates_status
  ON memory_candidates(status, created_at_ms);

CREATE TABLE memories (
  id                  TEXT PRIMARY KEY,
  scope               TEXT NOT NULL CHECK(scope IN ('user','relationship')),
  character_id        TEXT,
  kind                TEXT NOT NULL,
  content             TEXT NOT NULL,
  normalized_key      TEXT NOT NULL,
  importance          REAL NOT NULL DEFAULT 0.5 CHECK(importance >= 0 AND importance <= 1),
  confidence          REAL NOT NULL CHECK(confidence >= 0 AND confidence <= 1),
  sensitivity         TEXT NOT NULL CHECK(sensitivity IN ('normal','sensitive','restricted')),
  source_message_id   TEXT REFERENCES messages(id) ON DELETE SET NULL,
  source_candidate_id TEXT UNIQUE REFERENCES memory_candidates(id) ON DELETE SET NULL,
  supersedes_id       TEXT REFERENCES memories(id) ON DELETE SET NULL,
  version             INTEGER NOT NULL DEFAULT 1,
  created_at_ms       INTEGER NOT NULL,
  updated_at_ms       INTEGER NOT NULL,
  last_used_at_ms     INTEGER,
  CHECK(
    (scope = 'user' AND character_id IS NULL) OR
    (scope = 'relationship' AND character_id IS NOT NULL)
  )
);

CREATE INDEX idx_memories_scope_key
  ON memories(scope, normalized_key);

CREATE TABLE relationship_state (
  character_id       TEXT PRIMARY KEY,
  affinity           REAL NOT NULL DEFAULT 0 CHECK(affinity >= -1 AND affinity <= 1),
  familiarity        REAL NOT NULL DEFAULT 0 CHECK(familiarity >= 0 AND familiarity <= 1),
  interaction_count  INTEGER NOT NULL DEFAULT 0,
  summary             TEXT NOT NULL DEFAULT '',
  version             INTEGER NOT NULL DEFAULT 1,
  updated_at_ms       INTEGER NOT NULL
);

CREATE TABLE memory_audit (
  id             TEXT PRIMARY KEY,
  memory_id      TEXT NOT NULL,
  action         TEXT NOT NULL CHECK(action IN ('confirmed','updated','deleted')),
  occurred_at_ms INTEGER NOT NULL
);

CREATE TABLE tool_runs (
  id               TEXT PRIMARY KEY,
  conversation_id  TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
  turn_id          TEXT NOT NULL REFERENCES turns(id) ON DELETE CASCADE,
  tool_name        TEXT NOT NULL,
  arguments_json   TEXT NOT NULL,
  origin           TEXT NOT NULL CHECK(origin IN ('user_explicit','model_suggested','scheduled')),
  status           TEXT NOT NULL CHECK(status IN ('pending','approved','rejected','succeeded','failed','cancelled')),
  result_summary   TEXT,
  created_at_ms    INTEGER NOT NULL,
  completed_at_ms  INTEGER
);

CREATE INDEX idx_tool_runs_conversation_time
  ON tool_runs(conversation_id, created_at_ms);

CREATE TABLE timers (
  id                TEXT PRIMARY KEY,
  conversation_id   TEXT NOT NULL REFERENCES conversations(id) ON DELETE CASCADE,
  tool_run_id       TEXT UNIQUE REFERENCES tool_runs(id) ON DELETE SET NULL,
  title             TEXT NOT NULL,
  due_at_ms         INTEGER NOT NULL,
  timezone          TEXT NOT NULL,
  original_request  TEXT NOT NULL,
  status            TEXT NOT NULL CHECK(status IN ('pending','fired','cancelled')),
  created_at_ms     INTEGER NOT NULL,
  updated_at_ms     INTEGER NOT NULL,
  fired_at_ms       INTEGER,
  cancelled_at_ms   INTEGER
);

CREATE INDEX idx_timers_pending_due
  ON timers(status, due_at_ms);

CREATE TABLE app_settings (
  key           TEXT PRIMARY KEY,
  value_json    TEXT NOT NULL,
  revision      INTEGER NOT NULL,
  updated_at_ms INTEGER NOT NULL
);
```

`scope='user'` 与具体角色无关，`character_id` 必须为空；`scope='relationship'` 必须带角色 ID。该约束由 repository 校验并由测试覆盖。角色核心设定和资源许可证属于版本控制内的角色包，不写入记忆表。

## 4. FTS5

确认后的记忆建立外部内容 FTS：

```sql
CREATE VIRTUAL TABLE memory_fts USING fts5(
  content,
  normalized_key,
  content='memories',
  content_rowid='rowid',
  tokenize='trigram'
);

```

启用 `trusted_schema=OFF` 时，SQLite 会拒绝 schema trigger 调用 FTS5 虚拟表。因而本项目不使用 FTS trigger，而由唯一的 MemoryWorker repository 在同一事务中显式执行：

```sql
-- insert 后取得 memories.rowid
INSERT INTO memory_fts(rowid, content, normalized_key)
VALUES (:rowid, :content, :normalized_key);

-- update/delete 前先读取旧值，并先删除旧索引项
INSERT INTO memory_fts(memory_fts, rowid, content, normalized_key)
VALUES ('delete', :old_rowid, :old_content, :old_normalized_key);
```

update 随后更新 `memories` 并插入新索引项；delete 随后删除 `memories` 行。任一步失败都回滚整个事务。实现还需遵守：

- 输入先做 Unicode NFC、空白折叠和大小写归一化；原文仍保存在 `content`。
- CJK 和普通文本用 trigram 检索；查询最多 12 个 OR term，结果默认 8 条、硬上限 20 条。
- 少于 3 个字符的查询不走 trigram，改用 `normalized_key` 精确/前缀匹配，并限定最近 200 条候选。
- 用户删除记忆时物理删除正文，MemoryWorker 在同一事务同步删除 FTS；audit 只保留 ID、动作和时间，不保留正文。
- 启动完整性检查发现 FTS 漂移时可从 `memories` 重建，不得从聊天记录重新推断已删除记忆。

`tool_runs` 保存工具调用的规范化参数、来源、授权/执行状态和脱敏结果摘要，不保存 API Key、原始音频或屏幕图像。应用启动时，未完成的 `pending/approved` 工具请求会标记为失败，避免确认卡片或执行记录在重启后悬挂。

`timers` 是一次性本地提醒的持久事实来源。创建提醒前先完成 `tool_runs` 的用户确认；确认后插入 `pending` 记录。应用调度器只能通过带状态条件的 claim 将到期记录转换为 `fired`，取消同样只能从 `pending` 转换为 `cancelled`，从而保证重启、并发唤醒或重复扫描不会重复触发。macOS 用户级 LaunchAgent 与应用调度器还共享一个本地原子 claim 目录，用于避免应用打开和应用退出两条投递路径重复通知。当前 `timezone='local'` 只表达本地相对计时，绝对日期、DST 复核、重复规则和退出登录期间的系统投递仍是后续增强。

## 5. 记忆候选流程

只有 `turn.status=completed` 才进入提取：

1. 读取该 turn 的 canonical user/assistant 文本。
2. 提取最多 3 条候选，分为 user fact/preference 与 relationship event/summary。
3. 归一化 key，并与同 scope 现有记录做 exact + FTS 去重。
4. 写入 `memory_candidates(status='pending')`，不进入上下文检索。
5. 用户确认时，可先编辑正文；在单事务内写入/更新 `memories`、更新候选状态并写 audit。
6. 用户拒绝时只把候选置为 rejected；正文不再参与任何检索。

`sensitive` 与 `restricted` 候选默认不自动展开给 Provider；MVP 均需显式确认。后续若加入自动记忆，也只能对低风险 normal 类型启用并提供总开关。

## 6. 检索与上下文预算

Context Builder 先在 user 与当前角色 relationship 两个 scope 分别检索，再合并排序：

```text
score = 0.45 * lexical_relevance
      + 0.25 * importance
      + 0.20 * recency
      + 0.10 * confidence
```

- user memory 最多 6 条，relationship memory 最多 4 条。
- 活动记录必须排除任何被另一条记录通过 `supersedes_id` 指向的旧记忆；历史记录只在管理界面查看，不注入模型。
- 总记忆文本默认不超过上下文预算的 15%，并设独立字符/Token 硬上限。
- restricted 记录只在用户当前问题明确涉及且设置允许时注入。
- 每次使用只异步更新 `last_used_at_ms`，不能因为一次检索改变记忆正文。
- 记忆以结构化、带来源标签的上下文块注入，不能伪装为 system 指令。
- OpenAI-compatible Provider 默认不接收长期记忆。只有用户在 Provider 设置中明确开启“发送匹配记忆”后，Context Builder 才检索并注入最多 6 条、总计最多 2400 字符的记忆；Mock Provider 仅在本机用于检索验收。

## 7. 消息提交与崩溃恢复

- 用户最终输入到达后，先创建 turn 与 completed user message，再启动 LLM。
- LLM delta 只写内存 buffer；可每 2 秒做一次 streaming checkpoint，但不得被当作 completed 消息。
- LLM 完成且播放结束/明确跳过后，在事务中写 canonical/display/spoken 文本并把 turn 置 completed。
- 取消或失败时保存已有 assistant buffer，并标记 cancelled/failed，便于 UI 解释；不得进入记忆提取。
- 启动时把残留的 preparing/active/finalizing turn 统一修复为 failed(`process_interrupted`)。

## 8. 迁移、备份与测试

- 迁移文件按递增版本只追加，不修改已发布 migration。
- 每次迁移在单事务运行；迁移前使用 SQLite backup API 生成带版本号的本地备份，成功启动后保留最近 3 份。
- downgrade 不自动执行；检测到数据库版本高于应用支持版本时以只读错误启动。
- 测试必须覆盖外键、候选幂等确认、乐观锁冲突、FTS 事务同步、删除无残留、崩溃恢复和迁移回滚。
