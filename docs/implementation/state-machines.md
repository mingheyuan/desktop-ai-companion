# 状态机与取消语义

## 1. 总体原则

Rust Core 是状态机唯一所有者。UI、Provider 与 AudioHost 只能发送事件，不能直接修改全局状态。

系统同时维护四个正交全局状态，并在活动轮次中维护一个生成子状态：

```text
ListeningMode   Disabled | Armed | QuietHours
InputState      Stopped | WakeListening | Capturing | Recognizing | Recovering
TurnState       Idle | Preparing | Active | Finalizing | Completed | Cancelled | Failed
PlaybackState   Idle | Buffering | Playing | Stopping | Failed
GenerationState NotStarted | Streaming | Finished | Failed   # 仅属于当前 turn
```

`TurnState=Active` 时 LLM 可以仍在流式生成，同时 TTS 已经开始播放。原方案使用单一的 `Generating/Delivering` 状态无法准确表示这种并行关系，因此 v0.4 将生成状态拆成活动轮次的子状态。

## 2. 标识与不变量

每次用户最终输入创建：

```text
turn_id: UUIDv7
generation: u64              # 进程内严格递增
conversation_id: UUIDv7
source: text | wake_word | push_to_talk
```

必须满足：

1. 同一时刻最多一个前台活动 turn。
2. Provider、AudioHost 和 UI 的异步事件必须携带 `turn_id` 与 `generation`；晚到事件若不匹配当前活动代次，静默丢弃并记录 debug 日志。
3. ASR partial 只更新临时字幕；只有 `asr.final` 可以创建 turn。
4. `Cancelled`、`Failed` 和未完成 turn 不得自动生成长期记忆候选。
5. 消息的 canonical 文本不可被翻译、字幕清理或 TTS 发音替换所修改。
6. `Completed` 必须发生在 LLM 结束、播放队列清空或明确停止、消息事务提交之后。

## 3. ListeningMode

| 当前 | 事件 | 守卫 | 动作 | 下一状态 |
|---|---|---|---|---|
| Disabled | `listening.enable` | 麦克风权限允许 | 启动 AudioHost、KWS/VAD | Armed |
| Disabled | `listening.enable` | 权限未决定/拒绝 | 请求权限或报可恢复错误 | Disabled |
| Armed | `quiet_hours.enter` | — | 停止 KWS 捕获，保留 PTT | QuietHours |
| QuietHours | `quiet_hours.leave` | 用户仍启用监听 | 恢复 KWS 捕获 | Armed |
| Armed/QuietHours | `listening.disable` | — | 停止捕获并清空预滚缓冲 | Disabled |
| 任意 | `audiohost.failed` | — | 停止监听，发布降级原因 | Disabled |

`QuietHours` 禁止唤醒词，但不禁止用户主动按下 PTT。系统托盘和桌宠状态必须能区分 Disabled 与 QuietHours。

## 4. InputState

| 当前 | 事件 | 动作 | 下一状态 |
|---|---|---|---|
| Stopped | `listening.armed` | 打开低成本 PCM 捕获，运行 KWS/VAD | WakeListening |
| WakeListening | `wake.detected` | 取消当前活动 turn，要求 AudioHost 固定 pre-roll，创建 capture session | Capturing |
| Stopped/WakeListening | `ptt.started` | 取消当前活动 turn，创建 capture session | Capturing |
| Capturing | `capture.started` | 启动 ASR；唤醒来源携带 pre_roll_ms，PTT 为 0 | Recognizing |
| Capturing | `capture.timeout` | 丢弃本次捕获 | WakeListening 或 Stopped |
| Recognizing | `asr.partial` | 以 revision 替换临时字幕 | Recognizing |
| Recognizing | `speech.ended` / `ptt.stopped` | 请求 ASR finalize | Recognizing |
| Recognizing | `asr.final(nonempty)` | 清空临时字幕，提交用户输入 | WakeListening 或 Stopped |
| Recognizing | `asr.final(empty)` | 清空临时字幕 | WakeListening 或 Stopped |
| Capturing/Recognizing | `capture.cancel` | 停止 ASR 与音频路由 | WakeListening 或 Stopped |
| 任意活动态 | `audiohost.disconnected` | 清空 capture，开始受限重连 | Recovering |
| Recovering | `audiohost.ready` | 根据 ListeningMode 恢复 | WakeListening 或 Stopped |
| Recovering | `retry.exhausted` | 发布降级错误 | Stopped |

恢复目标由 `ListeningMode` 派生：Armed → WakeListening，其余 → Stopped。KWS 只接收 PCM，不接触 ASR 文本。

## 5. TurnState 与 GenerationState

| 当前 | 事件 | 动作 | 下一状态 |
|---|---|---|---|
| Idle/Completed/Cancelled/Failed | `user_input.final` | 取消旧代次，分配新 ID，持久化 pending 用户消息 | Preparing |
| Preparing | `context.ready` | 启动 LLM stream，Generation=Streaming | Active |
| Preparing | `context.failed` | 标记用户消息保留、助手消息失败 | Failed |
| Active | `llm.delta` | 追加 canonical assistant buffer；按批次发 UI；分句送 TTS | Active |
| Active | `llm.completed` | Generation=Finished；封口剩余 TTS chunk | Finalizing |
| Active | `llm.failed` | Generation=Failed；停止待播队列 | Failed |
| Finalizing | `playback.drained` | 事务提交助手消息，安排记忆提取 | Completed |
| Finalizing | `playback.not_required` | 事务提交助手消息，安排记忆提取 | Completed |
| Finalizing | `playback.stopped_by_user` | 保留完整文本，事务提交消息，安排记忆提取 | Completed |
| Preparing/Active/Finalizing | `turn.cancel` | 广播取消，停止流/队列/播放，保存明确的 cancelled 状态 | Cancelled |
| 任意活动态 | `new_user_input` | 执行 `turn.cancel(reason=superseded)` 后创建新 turn | Preparing（新代次） |

如果用户关闭语音或 TTS 失败，`playback.not_required` 允许文本轮次正常完成；错误仍显示为非阻塞降级。只有至少形成有效 canonical assistant 文本的轮次才可进入 Completed。

## 6. PlaybackState

TTS 分句器为每个 chunk 分配从 0 开始的连续 `chunk_index`。可并发合成，但播放顺序不可改变。

| 当前 | 事件 | 动作 | 下一状态 |
|---|---|---|---|
| Idle | `tts.chunk.enqueued` | 建立 turn 队列并开始合成 | Buffering |
| Buffering | `chunk.ready(next)` | 下发 AudioHost 播放 | Playing |
| Buffering | `all_chunks.cancelled/failed` | 清空队列 | Idle 或 Failed |
| Playing | `chunk.finished` 且下一块就绪 | 播放下一块 | Playing |
| Playing | `chunk.finished` 且下一块未就绪、生成未结束 | 等待 | Buffering |
| Playing | `chunk.finished` 且生成结束、队列空 | 发布 `playback.drained` | Idle |
| Buffering/Playing | `playback.stop` / `turn.cancel` | 停止当前音频并清空该代次 | Stopping |
| Stopping | `audiohost.stopped` 或超时 | 发布停止完成 | Idle |
| 任意活动态 | `audiohost.playback_error` | 清空队列，保留文本 | Failed |

`chunk.ready` 若 index 大于当前期望值，只进入有界 buffer；若小于期望值或 generation 已失效则丢弃。

用户只执行 `playback.stop` 时，不取消仍在生成的文本：协调器把当前 turn 的 `playback_enabled` 置为 false，丢弃后续 TTS chunk；LLM 完成后发送 `playback.not_required`。如果此时 GenerationState 已经 Finished，则停止确认直接发送 `playback.stopped_by_user`，避免 TurnState 卡在 Finalizing。PTT、唤醒或新文本属于新输入意图，会取消整个旧 turn，而不走这条仅停语音路径。

## 7. 打断规则

按优先级从高到低：

1. 应用退出、权限撤销、AudioHost 崩溃；
2. 用户显式停止/取消；
3. PTT 开始或唤醒词触发的新语音；
4. 文本框提交的新输入；
5. Provider 超时和普通错误。

每次取消按固定顺序执行：

1. 递增 generation，使晚到事件立即失效；
2. 取消 LLM stream 与 TTS 合成任务；
3. 清空待播 chunk；
4. 通知 AudioHost 停止当前播放/识别；
5. 清空临时字幕和 VRM speaking 状态；
6. 在数据库中把 pending/streaming 消息标记为 cancelled；
7. 发布一次最终快照。

AudioHost 在超时内未确认时，Rust Core 仍完成逻辑取消，随后单独重启 AudioHost，不能阻塞新文本输入。

## 8. 表现状态派生

UI/VRM 不维护另一套业务状态，而从快照派生：

```text
recovering > error/needs_input > speaking > listening > thinking > ready > sleeping > idle
```

- `speaking`：PlaybackState=Playing。
- `thinking`：TurnState=Preparing，或 Active 且尚未开始播放。
- `listening`：InputState=Capturing/Recognizing。
- `error`：存在需要用户处理的非过期错误。
- `recovering`：InputState=Recovering 或 AudioHost 正在重启。
- `needs_input`：权限、记忆确认或明确选择正在等待用户。
- `ready`：有完成结果尚未查看，且没有更高优先级活动。
- `sleeping`：ListeningMode=QuietHours，且没有活动 turn、输入或播放。

这保证字幕、按钮、托盘图标和 VRM 动作观察同一事实来源。
