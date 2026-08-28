# IPC 与 AudioHost 协议

## 1. 版本与公共 envelope

Tauri IPC 与 AudioHost 控制协议都使用明确版本。新增可选字段可以保持小版本兼容；删除/改名字段、改变语义或二进制布局必须升主版本。

Rust → React 事件 envelope：

```json
{
  "protocol_version": "1.0",
  "event_id": "0194...",
  "event_seq": 42,
  "snapshot_revision": 18,
  "occurred_at": "2026-08-28T12:00:00.000Z",
  "type": "turn.text_delta",
  "turn_id": "0194...",
  "generation": 7,
  "payload": {}
}
```

- `event_seq` 在进程生命周期内递增，用于发现 UI 漏事件。
- `snapshot_revision` 只在可观察状态变化时递增。
- UI 检测到序号缺口、窗口重新加载或从睡眠恢复时调用 `app_get_snapshot`，不能尝试猜测缺失状态。
- 所有 command 接收 `request_id`；重复 request 返回同一结果或幂等 no-op。

## 2. React → Rust commands

| command | 核心参数 | 结果/约束 |
|---|---|---|
| `app_get_snapshot` | — | 返回完整状态、活动 turn 摘要、权限和错误 |
| `chat_submit_text` | `request_id, conversation_id?, text` | trim 后非空；新输入会打断旧 turn |
| `turn_cancel` | `request_id, turn_id, reason` | 已结束 turn 为幂等 no-op |
| `playback_stop` | `request_id, turn_id?` | 只停语音，不取消已完成文本 |
| `voice_set_listening_mode` | `request_id, mode` | `disabled/armed/quiet_hours` |
| `voice_push_to_talk_start` | `request_id` | 可在 Disabled/QuietHours 主动使用 |
| `voice_push_to_talk_stop` | `request_id, capture_id` | capture 不匹配时拒绝 |
| `memory_candidate_confirm` | `request_id, candidate_id, edited_content?` | 事务写入长期记忆 |
| `memory_candidate_reject` | `request_id, candidate_id` | 终态、可幂等 |
| `memory_search` | `query, scope?, limit?, cursor?` | 仅返回用户可见记录 |
| `memory_update` | `request_id, memory_id, expected_version, content` | 乐观锁 |
| `memory_delete` | `request_id, memory_id, expected_version` | 物理删除正文和 FTS |
| `pet_set_click_through` | `request_id, enabled` | 保留可恢复快捷键/托盘入口 |
| `pet_set_visible` | `request_id, visible` | 不销毁后台核心 |
| `settings_get` | — | 返回非秘密设置；Keychain 只返回配置状态 |
| `settings_update` | `request_id, patch, expected_revision` | 校验后原子更新 |

## 3. Rust → React events

| event | 用途 |
|---|---|
| `state.changed` | 小体积状态差量；含新 snapshot revision |
| `turn.started` | 新 turn 与输入来源 |
| `turn.text_delta` | 助手 canonical 文本增量；30ms 或 24 字符批量发送 |
| `turn.message_committed` | 消息终态与可持久化 ID |
| `turn.ended` | completed/cancelled/failed 与原因 |
| `voice.asr_partial` | 临时识别文本和 revision |
| `voice.asr_final` | 最终用户文本；随后会出现 turn.started |
| `playback.chunk_started` | 口型和 speaking 状态起点 |
| `playback.level` | 归一化振幅；最大 20Hz，允许丢帧 |
| `playback.chunk_finished` | chunk 顺序推进 |
| `memory.candidate_created` | 等待用户确认的候选 |
| `memory.changed` | 确认、修改或删除后刷新 |
| `permission.changed` | 麦克风/语音识别权限 |
| `error.raised` / `error.cleared` | 带作用域和可恢复动作的错误 |

文本 delta 与振幅事件属于可丢弃流；终态事件不可丢弃。UI 始终能够通过快照恢复。

## 4. AudioHost 连接

AudioHost 是签名的 macOS `.app`，由 Rust 用 `/usr/bin/open -n -W -a <bundle> --args` 启动。运行目录使用仅当前用户可访问的短路径，例如：

```text
/tmp/daic-<uid>/<launch_nonce>/control.sock
/tmp/daic-<uid>/<launch_nonce>/media.sock
/tmp/daic-<uid>/<launch_nonce>/token
```

- 目录权限 `0700`，token 文件权限 `0600`。
- Rust 先 bind/listen，再启动 AudioHost，避免连接竞态。
- 256-bit 随机 token 通过 token 文件传递；AudioHost 读后立即删除，握手后双方从内存清除。
- 每次启动使用新 nonce 和 token；旧 socket/目录只在验证 owner 与路径前缀后清理。
- control 使用 UTF-8 JSON Lines；media 使用独立二进制流，禁止把 PCM base64 塞进 JSON。

## 5. 握手与心跳

AudioHost 连接后必须首先发送：

```json
{"type":"host.hello","protocol":{"major":1,"minor":0},"launch_nonce":"...","token":"...","host_version":"0.1.0","pid":1234,"capabilities":["apple_asr","apple_tts","pcm_capture"]}
```

Rust 验证后回复：

```json
{"type":"core.welcome","protocol":{"major":1,"minor":0},"heartbeat_ms":2000,"max_frame_bytes":1048576,"session_id":"..."}
```

- major 不同：立即拒绝并提示升级。
- minor 不同：使用双方 capability 交集。
- Rust 每 2 秒发送 `core.ping(seq)`；连续 3 次未收到 `host.pong` 即判定断线。
- 启动连接超时 15 秒；优雅停止等待 3 秒，之后终止进程。

## 6. AudioHost control 消息

所有请求都包含 `request_id`，所有会影响对话的消息都包含 `turn_id/generation` 或 `capture_id`。

Core → Host：

- `audio.configure`：输入/输出设备、sample rate、channels、frame duration。
- `capture.start` / `capture.stop`：低成本 PCM 捕获和 KWS 路由。
- `asr.start` / `asr.finalize` / `asr.stop`：Apple Speech 会话。
- `tts.enqueue`：`turn_id, generation, chunk_index, voice, text, rate, pitch`。
- `tts.stop`：停止指定 generation；无指定时停止全部。
- `permission.refresh`：重新读取 TCC 状态。
- `host.shutdown`：优雅退出。

Host → Core：

- `host.ready`：当前设备、格式、权限和 capability。
- `permission.changed` / `device.changed`。
- `capture.started` / `capture.stopped`。
- `asr.partial`：`capture_id, recognition_epoch, revision, text`。
- `asr.final`：仅在 endpoint/finalize 时发送；滚动重启不得伪造用户最终输入。
- `tts.chunk_started` / `tts.level` / `tts.chunk_finished`。
- `host.error`：结构化错误，不发送 API key、完整音频或无裁剪用户文本日志。
- `host.pong`。

标准响应：

```json
{"type":"response","request_id":"...","ok":false,"error":{"code":"permission_denied","message":"...","retryable":false}}
```

## 7. PCM 二进制帧

media socket 每帧使用 32-byte network-byte-order header：

| 字段 | 大小 | 说明 |
|---|---:|---|
| magic | 4 | ASCII `DAIC` |
| version | 1 | 当前为 1 |
| kind | 1 | 1=PCM input，2=TTS level samples，3=reserved |
| flags | 2 | bit0=end，bit1=discontinuity |
| payload_len | 4 | 最大 1 MiB |
| sequence | 8 | 每 stream 递增 |
| timestamp_ns | 8 | 单调时钟 |
| stream_id | 4 | control 消息协商的流 ID |

PCM payload 在 `audio.configure` 协商，MVP 固定为 little-endian signed PCM16、16 kHz、mono、20 ms frame。header 使用大端，音频 sample 使用小端。接收方必须校验 magic、长度、sequence 和已知 stream_id；异常帧关闭 media 连接并触发重新握手。

## 8. 背压与资源上限

- control 单消息最大 256 KiB；media 单帧最大 1 MiB。
- UI 文本增量最多 30Hz；音量最多 20Hz。
- PCM 使用有界 ring buffer，默认 1 秒；满时丢最旧帧并设置 discontinuity。
- 每个 turn 最多缓存 32 个 TTS chunk、总文本 64 KiB；超限暂停继续合成，不暂停 LLM 文本显示。
- control 写阻塞超过 2 秒视为连接异常，不能阻塞音频回调线程。

## 9. 错误码

统一最小集合：

```text
permission_denied | device_unavailable | provider_unavailable
authentication_failed | rate_limited | timeout | cancelled
protocol_mismatch | invalid_request | resource_exhausted
host_disconnected | internal
```

每个错误同时带 `scope`、`retryable`、`user_action` 和不含秘密的 `diagnostic_id`。UI 根据结构化字段展示操作，不解析 message 文本。

## 10. 兼容测试

`contracts/fixtures` 保存 hello/welcome、ASR、TTS、错误和二进制 header 的黄金样本。Rust 与 Swift 测试必须互相读取相同 fixture；CI 至少覆盖：

- 未知可选字段被忽略；
- major 不兼容被拒绝；
- 超长/畸形 JSON 和 payload 被拒绝；
- generation 过期事件不改变状态；
- sequence 缺口产生 discontinuity；
- 断线后所有活动 session 得到一次终态。
