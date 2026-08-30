# 关键端到端时序

以下时序是模块间合同。实现可以替换内部算法，但不能绕过 generation guard、终态提交或取消顺序。

## 1. 应用启动与 AudioHost 就绪

```mermaid
sequenceDiagram
    participant UI as React/VRM
    participant Core as Rust Core
    participant DB as SQLite
    participant Host as macOS AudioHost

    UI->>Core: app_get_snapshot
    Core->>DB: migrate + recover interrupted turns
    DB-->>Core: conversations/settings/memories ready
    Core-->>UI: initial snapshot
    alt listening enabled or TTS requested
        Core->>Core: bind control/media sockets + create token
        Core->>Host: launch signed app bundle
        Host->>Core: host.hello(version, token, capabilities)
        Core-->>Host: core.welcome(heartbeat, limits)
        Host-->>Core: host.ready(permissions, devices)
    Core-->>UI: state:changed(audio=ready/degraded)
    end
```

数据库失败时不启动需要持久化的 turn；AudioHost 失败时仍允许文本对话。两类故障必须分别呈现。

## 2. 文本对话、流式 TTS 与完成

```mermaid
sequenceDiagram
    participant UI as React/VRM
    participant Core as Turn Coordinator
    participant Mem as Memory Reader
    participant LLM as LLM Provider
    participant TTS as TTS Queue
    participant Host as AudioHost
    participant DB as SQLite

    UI->>Core: chat_submit_text(request_id, text)
    Core->>Core: cancel previous generation; create turn
    Core->>DB: insert turn + completed user message
    Core->>Mem: retrieve confirmed memories within budget
    Mem-->>Core: user + relationship context
    Core->>LLM: start(turn_id, generation, context)
    loop streaming
        LLM-->>Core: TextDelta
        Core-->>UI: turn:text-delta (batched)
        opt sentence boundary
            Core->>TTS: synthesize(chunk_index, canonical, spoken)
            TTS-->>Host: tts.enqueue(next ordered chunk)
            Host-->>Core: tts.chunk_started / level / finished
            Core-->>UI: playback events
        end
    end
    LLM-->>Core: Completed
    Core->>TTS: seal remaining chunk
    TTS-->>Core: playback.drained or playback.not_required
    Core->>DB: commit assistant message + turn completed
    Core-->>UI: turn:message-committed + turn:ended
    Core->>Mem: enqueue memory extraction
```

TTS 失败只使语音降级；已有 LLM 文本仍可完成。LLM 失败则 turn failed，不生成记忆候选。

## 3. 模型 ToolCall、确认与恢复

当前实现每次模型响应只允许一个已注册的 `system.open_url`、`timer.create`、`timer.cancel` 或 `memory.save` 调用，每轮最多连续 3 步；显式中文意图仍可直接进入同一确认/执行边界。

```mermaid
sequenceDiagram
    participant UI as React
    participant Core as Rust Turn Coordinator
    participant LLM as OpenAI-compatible Provider
    participant DB as SQLite/tool_runs
    participant OS as macOS Tool

    Core->>LLM: messages + tool schemas
    LLM-->>Core: streamed ToolCall fragments
    Core->>Core: aggregate, validate and classify
    Core->>DB: insert model_suggested tool_run(pending)
    Core-->>UI: pending_tool_approval
    alt user rejects
        UI->>Core: tool_decide(false)
        Core->>DB: tool_run=rejected
        Core->>LLM: assistant tool_call + tool result(rejected)
    else user approves
        UI->>Core: tool_decide(true)
        Core->>DB: tool_run=approved
        Core->>OS: execute validated tool
        OS-->>Core: result or error
        Core->>DB: tool_run=succeeded/failed
        Core->>LLM: assistant tool_call + tool result
    end
    LLM-->>Core: final TextDelta / Completed
    Core->>DB: commit assistant message + turn completed
    Core-->>UI: final snapshot and turn:ended
```

工具结果作为不可信的 `tool` 消息回传；工具不能直接修改对话或记忆。用户切换输入或取消时，当前 generation 失效，待确认工具运行记为 cancelled。
确认卡片最多等待 2 分钟；超时后由 Rust 原子移除当前 pending 工具，写入 `cancelled` 审计，记忆候选标记为 rejected，且不执行任何外部副作用。模型发起的调用会收到“未确认，未执行”的工具结果后继续收尾；用户显式发起的调用直接结束当前回合。旧的过期任务必须用 `tool_call_id + turn_id + generation` 做身份校验，不能影响新确认卡片。

## 4. 唤醒词语音对话

以下是目标 KWS/VAD 时序。当前切片由 AudioHost 内 Apple Speech 设备端识别同时完成唤醒短语检测与 partial/final，未经过 PCM media socket；当前协议见 [实现状态矩阵](status.md#当前-audiohost-协议v10-子集)。

```mermaid
sequenceDiagram
    participant Host as AudioHost
    participant Audio as Rust Audio Pipeline
    participant Core as Turn Coordinator
    participant UI as React/VRM
    participant ASR as Apple ASR Session

    Host-->>Audio: PCM frames
    Audio->>Audio: pre-roll + KWS
    Audio->>Core: wake.detected(capture_id)
    Core->>Core: cancel active turn and playback
    Core-->>UI: state=listening
    Core->>ASR: capture.start
    ASR-->>Core: capture.started
    Core->>ASR: asr.start(capture_id, generation, pre_roll_ms=800)
    ASR->>ASR: prepend host-local pre-roll; start VAD route
    loop speech
        Host-->>ASR: microphone audio
        ASR-->>Core: asr.partial(revision, full_text)
    Core-->>UI: voice:asr-partial (replace)
    end
    Audio->>Core: speech.ended
    Core->>ASR: asr.finalize
    ASR-->>Core: asr.final(text)
    Core-->>UI: voice:asr-final
    Core->>Core: create text-equivalent turn(source=wake_word)
```

KWS 命中不是用户消息；VAD 结束也不是用户消息。只有非空 `asr.final` 创建 turn。

## 5. 按键说话

```mermaid
sequenceDiagram
    participant Hotkey as Global Shortcut/UI
    participant Core as Rust Core
    participant Host as AudioHost
    participant ASR as ASR Session

    Hotkey->>Core: voice_push_to_talk_start
    Core->>Core: cancel active turn; allocate capture_id
    Core->>Host: capture.start
    Host-->>Core: capture.started
    Core->>Host: asr.start(pre_roll_ms=0)
    Host-->>Core: asr.partial(revision, text)
    Hotkey->>Core: voice_push_to_talk_stop(capture_id)
    Core->>Host: asr.finalize
    Host-->>Core: asr.final(text)
    Core->>Core: submit turn(source=push_to_talk)
```

PTT 可在 ListeningMode=Disabled/QuietHours 使用。快捷键 key-up 丢失时，30 秒硬上限会 finalize；窗口失焦不能自行停止系统级快捷键 capture。

## 6. 新输入打断旧回复

```mermaid
sequenceDiagram
    participant Input as Text/PTT/Wake
    participant Core as Turn Coordinator
    participant LLM as LLM Provider
    participant TTS as TTS Queue
    participant Host as AudioHost
    participant DB as SQLite
    participant UI as React/VRM

    Input->>Core: new_user_input
    Core->>Core: increment generation
    par cancel generation
        Core-->>LLM: cancel old stream
        Core-->>TTS: cancel generators + clear queue
        Core-->>Host: tts.stop / asr.stop as applicable
    end
    Core->>DB: mark old turn/messages cancelled
    Core-->>UI: clear transient subtitle + turn:ended(cancelled)
    Core->>Core: create new turn
    Note over LLM,Host: late old-generation events are discarded
```

逻辑取消不等待 Host ack 才生效。Host 超时由 AudioHost supervisor 单独处理。

## 7. 记忆候选确认

```mermaid
sequenceDiagram
    participant Core as Turn Coordinator
    participant Worker as MemoryWorker
    participant UI as Memory UI
    participant DB as SQLite/FTS

    Core->>Worker: extract(completed turn canonical text)
    Worker->>DB: dedupe against confirmed memories
    Worker->>DB: insert pending candidate(s)
    Worker-->>UI: memory:candidate-created
    alt user confirms or edits
        UI->>Worker: confirm(candidate_id, edited_content?)
        Worker->>DB: transaction: upsert memory + FTS + audit + candidate confirmed
        Worker-->>UI: memory:changed
    else user rejects
        UI->>Worker: reject(candidate_id)
        Worker->>DB: candidate rejected
        Worker-->>UI: memory:changed
    end
```

未确认候选不能被 Context Builder 检索。确认命令按 candidate ID 幂等；重复确认返回同一 memory ID。

## 8. AudioHost 崩溃恢复

```mermaid
sequenceDiagram
    participant Host as AudioHost
    participant Sup as AudioHost Supervisor
    participant Core as State Machines
    participant UI as React/VRM

    Host--xSup: socket EOF / heartbeat timeout
    Sup->>Core: audiohost.disconnected
    Core->>Core: invalidate audio generation; finish sessions once
    Core-->>UI: recovering + text fallback
    loop max 3 attempts
        Sup->>Host: launch with new sockets/token
        Host-->>Sup: hello + ready
    end
    alt recovered
        Sup->>Core: audiohost.ready
        Core-->>UI: restore allowed listening mode
    else exhausted
        Sup->>Core: retry.exhausted
        Core-->>UI: audio unavailable; show retry/settings action
    end
```

恢复不自动重放崩溃前的 ASR partial 或 TTS chunk，避免重复输入和幽灵语音。
