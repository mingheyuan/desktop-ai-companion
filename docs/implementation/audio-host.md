# macOS AudioHost 实现规格

> 本文同时记录目标规格和当前落地状态。标为“目标”的 PCM/KWS/VAD/口型能力不能被 README 或 UI 宣称为已完成。

## 0. 当前可运行切片

截至当前实现，`native/audio-host/AudioHost.swift` 已具备：

- 固定 bundle identifier 的 `.app`、Info.plist 权限用途说明和本地签名构建脚本；
- Rust 先 bind、AudioHost 后连接的 UDS JSONL 通道，以及 protocol major、256-bit token、launch nonce 握手；
- 麦克风与 Speech Recognition 权限状态查询和仅由用户点击触发的授权请求；
- PTT、Apple Speech partial/final、45 秒 recognition task 滚动和 capture/epoch/generation guard；
- 使用 Apple Speech 设备端识别检测“爱丽丝”及少量中文同音变体（“艾丽丝”“爱丽思”“爱丽斯”“阿丽丝”）的持续监听 fallback；不支持设备端识别时直接关闭持续监听，不把环境音降级到在线识别；
- `AVSpeechSynthesizer` 本地朗读、停止、PTT/文字新输入打断、旧 utterance 回调过滤，以及基于 `willSpeakRange` 的近似口型 level；
- Rust 侧 turn/input/playback 状态投影、Tauri 命令、UI 控件和 `Cmd/Ctrl + Shift + M` PTT 快捷键；
- 不请求语音权限的真实 `.app` 启动、握手和退出测试。

尚未落地的目标项包括：二进制 PCM media socket、Sherpa KWS、独立 VAD、pre-roll、真实 RMS 口型、设备切换、安静时段，以及 30 分钟/睡眠恢复实机验收。当前已对 control socket 接入 heartbeat：Rust 每 2 秒发送 ping，6 秒未收到当前连接 pong 即走已有三次退避重启；仍需实机验证 AudioHost 内部音频线程失效、睡眠和外设切换场景。当前 fallback 在检测到命令文本后使用 1.1 秒静音计时，只说唤醒词而没有后续命令时 5 秒返回待机；唤醒匹配器接受固定的“爱丽丝”“天童爱丽丝”以及少量中文同音变体，匹配和命令截断都在 AudioHost 本地完成；口型 level 只表示发音区间活动而非真实响度；PTT 在设备端识别不可用时可能使用 Apple Speech 系统在线服务，UI 会根据 `capture.started.on_device` 显示“本机识别”或“系统语音服务”。

## 1. 职责

AudioHost 是随桌面应用分发的签名 macOS `.app`，提供稳定 TCC 身份和原生音频能力：

- 麦克风权限与 Speech Recognition 权限；
- AVAudioEngine 输入采集与设备变化；
- Apple Speech 流式识别；
- `AVSpeechSynthesizer.write` 缓冲合成、AVAudioEngine 播放与振幅回传；
- 与 Rust Core 的 UDS 控制/媒体连接。

目标架构中它不负责唤醒词判定、VAD 策略、LLM、记忆、角色设定或 UI，KWS/VAD 在 Rust 中运行，AudioHost 只供应 PCM。当前可运行 fallback 暂由 AudioHost 内 Apple Speech 完成设备端唤醒短语检测和 endpoint 计时，后续接入 media socket 后再迁回 Rust KWS/VAD。

## 2. 进程生命周期

```text
Stopped -> Launching -> Handshaking -> Ready -> Degraded
    ^          |             |          |          |
    +----------+-------------+----------+----------+
                         Stopping / Failed
```

| 状态 | 进入条件 | 退出条件 |
|---|---|---|
| Stopped | 初始或正常退出 | 需要音频能力 |
| Launching | Rust 已 bind socket 并启动 bundle | 进程连接或 15s 超时 |
| Handshaking | 收到 UDS 连接 | token/version/capability 验证完成 |
| Ready | 权限和至少一个所需能力可用 | 权限/设备变化、断线或停止 |
| Degraded | 部分能力不可用 | 恢复、用户禁用或故障升级 |
| Stopping | 收到 shutdown/应用退出 | 3s 内确认，否则终止 |
| Failed | 握手失败或重试耗尽 | 用户重试、设置变化或下次启动 |

AudioHost 进程在用户启用持续监听时常驻；监听关闭但仍可能使用 TTS 时可保持 60 秒空闲后退出。频繁对话不反复启动 bundle。

断线恢复最多自动尝试 3 次，退避 1s/2s/5s。每次使用新 nonce、socket 目录与 token。重试期间文本对话仍可用，超过次数后进入可见降级，不进行无限拉起。

## 3. Swift 内部并发

- `ControlQueue`：串行执行 configure/start/finalize/stop、generation 变化与资源释放。
- AVAudioEngine render callback：只复制到预分配 ring buffer，不分配大对象、不做 JSON/网络/锁等待。
- `MediaWriter`：从 ring buffer 读取并写 media socket；阻塞时按策略丢最旧帧。
- Apple Speech callback：先检查 `capture_id + recognition_epoch + generation`，再投递到 ControlQueue。
- TTS delegate callback：转换为 chunk/level 事件，不直接修改全局状态。

关闭顺序固定为：停止接收新命令 → invalidate generation → 移除 input tap → stop recognition task → stop synthesizer → 关闭 socket → 退出。

## 4. 麦克风与持续监听

MVP PCM 格式固定为 16kHz mono PCM16，每帧 20ms。若设备原始格式不同，AudioHost 在非实时工作队列使用 AVAudioConverter，media socket 只输出协商格式。

```text
Microphone
  -> AVAudioEngine input tap
  -> bounded pre-roll ring buffer (800ms)
      -> MediaWriter -> Rust KWS / VAD
      -> Apple Speech（收到 asr.start 后先消费宿主本地 pre-roll）
```

- ListeningMode=Armed 时保持输入 capture，Rust 仅运行低成本 KWS。
- 唤醒命中后，Rust 发送带 `pre_roll_ms` 的 `asr.start`；AudioHost 把最多 800ms 宿主本地预滚与后续音频送入本次 Apple Speech capture，避免 PCM 往返传输和吃掉开头。
- 捕获等待语音 5 秒超时；单次话语默认 30 秒硬上限；静音 endpoint 默认 800ms，均可在安全范围内配置。
- QuietHours 停止后台 capture；PTT 可临时启动 capture。
- 屏幕锁定、系统睡眠或默认输入设备失效时立即停止捕获并清空预滚；唤醒后重新枚举设备和权限，不能沿用旧 stream ID。

### TTS 自唤醒

在回声消除 Spike 通过前，MVP 播放 TTS 时暂停 wake-word 判定，但持续显示麦克风状态；用户仍可用全局 PTT/停止快捷键立即打断。若后续启用 voice barge-in，必须先证明输出回采不会造成稳定自唤醒，并作为显式 capability 开启。

## 5. Apple Speech 会话

借鉴本地 `realtime-ai-translation` 的成熟做法：权限由 app bundle 请求、控制操作串行化、回调带 generation guard，并在长会话中滚动 recognition task 而不重启 AVAudioEngine。

本项目增加以下约束：

- `capture_id` 标识用户一次讲话；`recognition_epoch` 只标识 Apple 内部 task。
- partial 带单调 revision，语义是“替换当前临时文本”。
- task 接近平台时限时可在约 45 秒滚动；滚动结果合并到 capture accumulator，不发送用户级 Final。
- 只有 Rust 发来的 `asr.finalize` 或明确 endpoint 才允许发一次 `asr.final`。
- `asr.stop` 是取消，不把最后一个 partial 冒充 Final。
- 回调来自旧 generation/epoch 时直接丢弃。
- 可恢复错误最多重建 task 2 次；权限、locale 不支持等错误立即结束 session。

Swift 侧保持 `committed_prefix + current_partial`，每次 task 滚动时把稳定文本并入 prefix。Rust 仍按 revision 接收完整临时文本，避免跨 epoch 重复 append。

## 6. Apple TTS 与播放

MVP 使用单一、可观测的本地播放路径：

```text
AVSpeechSynthesizer.write(_:toBufferCallback:)
  -> bounded PCM queue
  -> AVAudioPlayerNode / AVAudioEngine
  -> selected output device
  -> RMS envelope events
```

- 每个 utterance 携带 `turn_id/generation/chunk_index`。
- Rust 可以提前排队多个 chunk；AudioHost 只播放当前代次的连续 index。
- `tts.stop` 立即停止 synthesizer 与 player node，清空合成/PCM/待播三层队列，并只发一次停止终态。
- 口型优先使用实际 PCM 计算的低频 RMS/包络值，UI 最大消费 20Hz；range callback 只用于可选字幕进度，不作为真实响度。
- voice 不可用时返回 `invalid_request` 并列出安全的 fallback voice ID；不得静默换成完全不同语言。
- route/device 改变时当前 chunk 可失败，但文本 turn 继续完成。

云端 TTS 后续产生的 PCM 也通过统一播放命令进入 AudioHost，确保设备、音量、停止和口型只有一条实现路径。

## 7. 权限状态

分别上报：

```text
microphone: not_determined | denied | restricted | authorized
speech_recognition: not_determined | denied | restricted | authorized
```

- 只在用户主动启用语音/监听时请求权限，不在首次启动自动弹窗。
- denied/restricted 时给出打开系统设置的 UI action；AudioHost 自己不循环请求。
- 权限变化会终止相关 session 并通知 Rust；文本和不依赖该权限的能力继续工作。
- 应用必须有明确的麦克风活动指示、暂停入口和托盘状态。

## 8. 设备变化与故障

- 监听默认设备、sample rate、route 和 engine configuration change。
- 变化时停止旧 engine，递增 audio generation，重建 converter/tap，再发布新 stream ID。
- 重建期间 Rust InputState=Recovering；旧 PCM 和 ASR 回调因 generation 失效。
- media socket 背压只允许丢 PCM 并标 discontinuity，不能阻塞 AVAudioEngine callback。
- control socket 断开视为 Core 已退出：立即停止麦克风和播放，清理 socket，不继续后台监听。

## 9. 日志与可观测性

统一字段：`launch_nonce`、`session_id`、`capture_id`、`turn_id`、`generation`、`recognition_epoch`、`chunk_index`、`diagnostic_id`。

允许记录状态、耗时、字节/帧数、错误码和设备类型；默认不记录识别文本、合成文本、原始音频、token 或完整路径中的用户名。开发者显式开启内容日志时也应进行截断，并在 UI 显示当前处于调试模式。

## 10. 验收测试

- 首次授权、拒绝、系统设置重新授权和权限撤销。
- Armed 连续运行 30 分钟、睡眠/唤醒、插拔耳机、切换输入设备。
- Apple Speech task 滚动不重复/丢失用户级 Final。
- 20 次快速 start/stop/PTT、播放中 PTT 打断、应用退出时正在识别。
- 人为 kill AudioHost 后自动恢复且无幽灵语音/字幕。
- Core 崩溃/关闭 socket 后 AudioHost 在限定时间内停止麦克风。
- media/control 畸形帧、背压、sequence discontinuity 和协议版本不匹配。
