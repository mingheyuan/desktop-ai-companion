# Desktop AI Companion 内部技术架构

> 版本：v0.2 架构基线
>
> 状态：已确认方向，实施前仍需冻结协议字段、数据库迁移和关键状态转移
>
> 更新日期：2026-08-28

## 1. 文档目标

本文定义 MVP 内部各组件的职责、进程边界、通信方式、状态管理、语音链路、记忆流程和桌宠交互组织方式。

这份文档解决的是“各技术如何协作”，不是立即开始编码的信号。在第 13 节列出的实施前门槛完成前，不初始化业务代码骨架。

### MVP 架构目标

- 桌宠窗口、聊天界面、语音链路和记忆管理具有清晰的数据所有权。
- 一次对话可以被可靠地启动、打断、取消和恢复，迟到事件不会污染新对话。
- 持续监听只在本地完成唤醒检测，唤醒前音频不进入云端服务。
- 记忆写入可追溯、可确认、可撤销，角色核心设定不会被普通对话覆盖。
- 角色表现由统一状态派生，业务模块不能直接争抢动画和表情控制权。

### MVP 非目标

- 多角色同时运行。
- 跨设备同步和云端记忆。
- 向量数据库、复杂自治规划和长期后台任务。
- 钢琴、MIDI、屏幕理解和完整换装系统。

## 2. 总体结构

```mermaid
flowchart LR
    subgraph UI["Tauri WebView / React"]
        Pet["桌宠悬浮层\nThree.js + VRM"]
        Panel["聊天 / 设置 / 记忆面板"]
        ViewStore["Zustand 展示状态"]
    end

    subgraph Core["Tauri / Rust 核心"]
        Supervisor["InteractionSupervisor\n交互总协调器"]
        Turn["TurnActor\n单轮对话"]
        AudioWorker["AudioWorker\n音频控制队列"]
        MemoryWorker["MemoryWorker\n记忆串行写入"]
        Providers["ProviderRegistry\nLLM / ASR / TTS"]
        Activity["ActivityStore + AttentionResolver"]
        Behavior["BehaviorScheduler\n角色表现调度"]
        Window["PetOverlayController\n窗口与托盘"]
        DB[("SQLite + FTS5")]
    end

    subgraph Native["持久 Swift AudioHost.app"]
        Capture["AVAudioEngine\n采集 / 播放 / 环形缓冲"]
        Speech["Apple Speech"]
        Synth["AVSpeechSynthesizer\nBuffer TTS"]
        Permission["音频设备与权限"]
    end

    UI <-->|"类型化 Tauri Command / Event"| Core
    Supervisor --> Turn
    Supervisor --> AudioWorker
    Supervisor --> MemoryWorker
    Supervisor --> Activity
    Turn --> Providers
    MemoryWorker <--> DB
    Activity --> Behavior
    Behavior --> ViewStore
    Window --> Pet
    AudioWorker <-->|"UDS：JSONL 控制 + 二进制 PCM"| Native
```

核心原则是：**Rust 核心保存业务事实，Swift AudioHost 保存瞬时音频状态，React 只保存可重建的展示状态，SQLite 保存持久数据。**

## 3. 进程与模块边界

### 3.1 React / Tauri WebView

负责：

- VRM 渲染、动画混合、表情、视线和口型展示。
- 桌宠气泡、聊天、设置、记忆管理和活动列表界面。
- 用户点击、拖动、快捷操作等 UI 意图上报。
- 缓存后端状态的只读投影；重载后可从 Rust 快照恢复。

不负责：

- 对话编排、记忆决策、密钥保存、音频设备控制。
- 将 UI 状态当作对话或权限状态的唯一事实来源。

### 3.2 Tauri / Rust 核心

Rust 是应用业务状态的唯一事实来源，主要模块如下：

- `InteractionSupervisor`：唯一交互总协调器；创建和取消对话轮次，协调音频、模型、记忆与展示状态。
- `TurnActor`：每轮对话一个实例；管理上下文准备、LLM 流、句子切分、TTS 投递和完成状态。
- `AudioWorker`：串行处理音频控制请求，维护 AudioHost 连接、唤醒词、VAD、ASR 和播放会话。
- `MemoryWorker`：记忆唯一写入口；串行执行候选合并、确认、修改、删除和索引更新。
- `ProviderRegistry`：隔离 LLM、ASR 和 TTS Provider；Provider 不直接修改 UI 或数据库。
- `ActivityStore`：记录需要呈现给用户的当前活动、结果和阻塞项。
- `AttentionResolver`：按优先级从多个活动中选出桌宠当前应该表达的状态。
- `BehaviorScheduler`：把语义状态转换为动作、表情、视线、口型和气泡提示。
- `PetOverlayController`：管理桌宠悬浮窗口、主面板、托盘、快捷键、位置和鼠标穿透。

MVP 同一时间只允许一个前台 `TurnActor`。记忆整理可以在该轮完成后异步执行，但被取消或未完成的轮次不得自动提交长期记忆。

### 3.3 Swift AudioHost.app

AudioHost 是随主应用启动和退出的持久辅助进程，而不是每次识别临时启动的脚本。它负责：

- 麦克风权限、音频设备变化和 `AVAudioEngine` 生命周期。
- 麦克风 PCM 采集、短环形缓冲和音频播放。
- Apple Speech 会话及 `partial / final` 结果。
- `AVSpeechSynthesizer.write(_:toBufferCallback:)` 生成音频缓冲。
- 向 Rust 报告播放进度、振幅包络、设备与权限状态。

AudioHost 不理解角色、LLM、记忆或业务页面。其崩溃只影响语音能力，不应导致聊天和记忆数据损坏。

## 4. 数据所有权与同步

| 数据 | 唯一事实来源 | 其他层的职责 |
| --- | --- | --- |
| 当前对话轮次、取消代次 | Rust `InteractionSupervisor` | React 展示投影；AudioHost 执行带代次的命令 |
| 麦克风、ASR、播放实时状态 | Swift AudioHost，Rust 保存已确认快照 | React 只展示 Rust 发布的状态 |
| 聊天记录、记忆、角色配置 | SQLite，经 Rust Repository 访问 | React 不直连数据库；AudioHost 不读取数据库 |
| 桌宠位置、尺寸、显示器、可见性 | Rust 配置存储 | React 提交意图，窗口控制器应用并回报 |
| 动作、表情、口型 | Rust 派生语义状态；React 执行渲染 | 动画完成事件可回传，但不能反向改写业务状态 |
| API Key | macOS Keychain，经 Rust 访问 | 不进入前端持久存储、日志或 SQLite |

UI 启动或重载时先请求完整 `AppSnapshot`，之后消费增量事件。若发现事件序号断裂，UI 丢弃本地投影并重新拉取快照，不尝试自行补状态。

## 5. 状态模型

避免使用一个巨大枚举表达所有组合。MVP 采用四组相互正交的状态：

```text
ListeningMode: Disabled | Armed | QuietHours
InputState:    Stopped | WakeListening | Capturing | Recognizing | Finalizing | Recovering
TurnState:     Idle | Preparing | Generating | Delivering | Completed | Cancelled | Failed
PlaybackState: Idle | Buffering | Playing | Stopping
```

- `ListeningMode` 表示用户授权与产品模式。
- `InputState` 表示麦克风到识别文本的实时阶段。
- `TurnState` 表示一次文字或语音对话的业务生命周期。
- `PlaybackState` 表示 TTS 队列与扬声器状态。

桌宠的 `idle / listening / thinking / speaking / needs-input / blocked / ready / sleeping` 是以上状态与活动信息的**派生结果**，不是另一套可被任意模块写入的业务事实。

### 代次与迟到事件

所有跨模块消息都携带：

- `schema_version`
- `session_id`
- `turn_id`（与具体对话轮次相关时）
- `generation`（取消代次）
- `sequence` 或 `request_id`

取消顺序固定为：先递增 `generation`，再广播取消，最后清空对应队列。任何旧 `generation` 的 ASR、LLM、TTS 或 UI 事件都直接丢弃。这样可以避免上一轮的迟到结果进入下一轮。

## 6. 通信协议

### 6.1 React 与 Rust

- Tauri Command：用户意图和需要返回值的低频请求，例如开始对话、停止朗读、确认记忆、修改设置。
- Tauri Event：后端状态、流式文本、活动变化和错误通知。
- 不使用内部 WebSocket；只有外部 Provider 明确需要时才使用 HTTP/SSE/WebSocket。
- 原始 PCM、逐采样振幅等高频数据不得经过 Tauri JSON Event。
- LLM token 在 Rust 内按时间或字符阈值合并，再批量推送 UI，减少 WebView 事件压力。

统一事件信封示例：

```json
{
  "schema_version": 1,
  "event_id": "01J...",
  "session_id": "session-...",
  "turn_id": "turn-...",
  "generation": 4,
  "sequence": 12,
  "type": "turn.delta",
  "payload": {}
}
```

### 6.2 Rust 与 AudioHost

使用当前用户私有的 Unix Domain Socket：

- 控制通道：UTF-8 JSON Lines，传输握手、开始/停止、Provider 参数、状态、错误和心跳。
- 媒体通道：长度前缀的二进制帧，传输 PCM 和必要的振幅数据。
- Socket 文件权限设为当前用户可读写；连接握手校验协议版本、会话随机令牌和能力列表。
- AudioHost 断线后由 Rust 限次重启并重建状态；重连前所有旧会话失效。

控制协议与媒体协议必须独立版本化。不得把 PCM 编码进 JSON 或 Base64 长消息。

## 7. 语音交互流程

### 7.1 唤醒与识别

```mermaid
sequenceDiagram
    participant M as 麦克风 / AudioHost
    participant A as Rust AudioWorker
    participant S as InteractionSupervisor
    participant P as Apple Speech
    participant L as LLM Provider
    participant T as TTS / 播放

    M->>M: 保存约 1 秒 PCM 环形缓冲
    M->>A: 本地 PCM（仅供 KWS / VAD）
    A->>A: Sherpa KWS 检测唤醒词
    A->>S: wake.detected
    S->>M: start_asr(pre_roll_ms)
    M->>P: 环形缓冲 + 后续实时音频
    P-->>M: partial / final
    M-->>S: asr.partial / asr.final
    S->>L: 创建 TurnActor 并流式生成
    L-->>S: 文本增量
    S->>T: 完整句子分片
    T-->>S: 播放进度 / 振幅 / 完成
```

具体规则：

1. `Armed` 模式下 AudioHost 持续采集，但唤醒前只把音频送到本机 Rust KWS/VAD，绝不发送给 LLM 或云端 ASR。
2. AudioHost 保存约 1 秒环形缓冲。检测到唤醒后，Apple Speech 从 pre-roll 开始，避免吞掉唤醒词后的首字。
3. `partial` 只用于字幕和端点辅助，不能写入聊天记录或长期记忆。
4. `final`、VAD 端点、超时和用户手动结束共同决定本轮输入封口；只有封口文本进入 LLM。
5. Apple Speech 是否完全本地取决于系统、语言和设备能力，产品界面必须区分“本地唤醒”和“识别可能使用系统在线服务”。

### 7.2 TTS、口型与文本一致性

- `TurnActor` 对 LLM 输出做安全的句子切分，完整句子才进入 TTS 队列。
- TTS Provider 接口允许有界预生成多个分片，但播放严格按 `chunk_sequence` 排序；Apple Provider 初始并发设为 1，验证稳定后再提高。
- AudioHost 从语音缓冲计算振幅包络，Rust 将其降采样后发送给 React 驱动基础口型。
- 每轮分别记录 `generated_text`、`displayed_text` 和 `spoken_text`，避免打断后把未说出的内容标记为已朗读。
- 停止时立即失效当前 `generation`，清空未播放分片，并把当前消息标记为 `interrupted`。

### 7.3 打断策略

MVP 必须保证快捷键、点击和托盘命令可以立即停止 TTS 并开启新一轮输入。

“角色说话时用自然语音打断”需要回声消除，否则扬声器声音可能触发 VAD、唤醒词或 ASR。实施前先完成 `VoiceProcessingIO` / AEC 可行性验证：

1. 播放期间检测到疑似人声后先降低 TTS 音量。
2. 短窗口内二次确认来自麦克风的人声或有效识别片段。
3. 确认后递增代次、停止播放并切入输入状态。

如果 AEC 在目标设备上不稳定，MVP 关闭“播放中语音打断”，保留快捷键和点击打断，不伪装成已经解决。

## 8. 对话与 Provider 编排

一次前台轮次遵循：

```text
用户输入
  -> 输入封口
  -> 检索已确认记忆
  -> 组装角色核心设定 + 会话上下文 + 记忆数据
  -> LLM 流式生成
  -> UI 批量增量 + 句子 TTS
  -> 完成或取消
  -> 保存聊天记录
  -> 异步生成记忆候选
```

- Provider 只暴露能力与会话接口，不参与窗口、角色动作或数据库事务。
- LLM、ASR 和 TTS 请求均接收取消令牌、超时和 `generation`。
- Provider 错误转换为统一错误类型，再由 `InteractionSupervisor` 决定重试、降级或提示用户。
- 本地与云端 Provider 使用相同上层事件，不让 UI 感知供应商差异。

## 9. 记忆生命周期

```mermaid
flowchart LR
    Chat["已完成聊天记录"] --> Extract["记忆提取器"]
    Explicit["用户明确说“记住”"] --> Candidate["MemoryCandidate"]
    Extract --> Candidate
    Candidate --> Queue["MemoryWorker 串行队列"]
    Queue --> Review{"是否确认"}
    Review -->|自动提取| Pending["pending"]
    Review -->|用户明确要求| Confirmed["confirmed"]
    Pending -->|用户确认| Confirmed
    Pending -->|用户拒绝| Rejected["rejected"]
    Confirmed --> Index["SQLite + FTS5"]
    Index --> Retrieve["相关记忆检索"]
    Retrieve --> Prompt["作为不可信数据注入上下文"]
```

### 9.1 记忆类型与状态

MVP 记忆范围：

- `user_global`：用户稳定信息与偏好。
- `character_relationship`：用户与当前角色的共同经历和关系信息。
- `preference`：可单独修改的偏好项。
- `event`：有时间上下文的重要事件。

生命周期状态：`pending | confirmed | rejected | superseded`。

- 用户明确触发“记住”可以直接进入 `confirmed`，界面仍需可撤销。
- 模型自动提取只能进入 `pending`；只有 `confirmed` 记忆参与后续提示词检索。
- 冲突信息不覆盖旧记录，而是通过 `supersedes_id` 建立替代关系。
- 角色核心设定是独立只读数据源，不通过记忆写入接口修改。

每条记忆至少保存来源消息、作用域、创建与修改时间、置信度、敏感标记、版本和状态。删除需要同步移除检索索引，并保留最少量的本地审计信息或执行物理删除，由用户设置决定。

### 9.2 检索策略

- MVP 使用 SQLite FTS5，不引入 Embedding 或向量数据库。
- 中文和日文优先验证 FTS5 `trigram` tokenizer；极短查询使用规范化字段或受限 `LIKE` 回退。
- 检索综合文本相关性、时间、新鲜度、重要度和作用域，结果数量设置硬上限。
- 记忆作为引用数据放入清晰分隔的提示词区域，不能被解释成系统指令，降低历史内容中的提示注入风险。

## 10. 桌宠交互组织

参考 Codex Pets 的不是外观素材，而是“用一个轻量桌面角色表达后台活动”的交互模型。

### 10.1 可直接借鉴的结构

- **悬浮层与主面板分离**：桌宠窗口保持轻量，聊天、设置和记忆管理放在独立主面板；气泡可依附桌宠位置。
- **Activity Store**：对话、权限、记忆确认和错误都表示为活动条目，而不是散落在组件局部状态中。
- **Attention Resolver**：多个活动同时发生时，桌宠只表达最值得关注的一项。
- **活动入口**：点击桌宠或状态气泡打开对应对话、记忆确认或错误上下文。
- **位置持久化**：保存显示器、相对位置、尺寸、可见性和鼠标穿透设置；显示器消失时回落到可见区域。
- **减少动态效果**：跟随 macOS Reduce Motion，并提供低功耗模式，降低帧率和非必要动画。

MVP 活动优先级：

```text
NeedsInput > Blocked > Ready > Running > Idle
```

- `NeedsInput`：等待麦克风权限、记忆确认或用户选择。
- `Blocked`：音频进程、网络或 Provider 错误导致流程无法继续。
- `Ready`：回复、记忆操作等结果已就绪但尚未查看。
- `Running`：正在聆听、识别、思考或朗读。
- `Idle`：无前台活动，进入角色日常待机表现。

### 10.2 角色包边界

角色能力拆为四类配置，避免“角色设定”和“程序能力”耦合：

- Persona：身份摘要、语言风格、价值观和关系边界。
- Avatar：VRM 模型、动作、表情和渲染映射。
- Voice：TTS Provider、声音参数和授权信息。
- Behavior：状态到动作的映射、主动行为频率和安静时段。

MVP 仍只实现单角色，但数据结构不把“天童爱丽丝”写死在业务代码中。仓库不复制 Codex Pets 或《蔚蓝档案》的内部代码、角色资源和受版权保护素材。

## 11. 错误恢复与退出

- AudioHost 心跳中断：停止接受新语音请求，使当前音频代次失效，限次重启；失败后进入 `Blocked`，文字聊天保持可用。
- 麦克风权限拒绝：进入 `NeedsInput`，提供系统设置入口，不循环弹窗。
- LLM 流中断：保留已生成文本并标记 `incomplete`，不自动创建长期记忆。
- 数据库迁移失败：停止写入并进入只读保护模式，禁止静默重建数据库。
- 应用退出顺序：停止接收新轮次 → 取消当前代次 → 停止录音与播放 → 刷新日志和数据库事务 → 关闭 AudioHost → 退出窗口。

所有日志默认不记录 API Key、完整音频和完整提示词。诊断导出必须由用户主动触发并展示包含的数据范围。

## 12. 安全与隐私边界

- 持续监听开关、麦克风占用和当前数据去向必须在 UI 中可见。
- 唤醒前 PCM 只存在于短环形缓冲与本地进程；超出窗口立即覆盖，不落盘。
- 对话音频默认不保存；若后续增加保存功能，必须单独授权并提供清理入口。
- API Key 只存 Keychain；UDS 使用用户私有目录、严格权限和会话令牌。
- 角色提示词、检索记忆和工具输出分区组装，外部文本按不可信数据处理。
- 角色与声音资源记录来源和授权，不把第三方受保护素材提交到公开仓库。

## 13. 开始实现前的冻结门槛

以下内容完成评审后，才初始化 Tauri / Rust / React 业务骨架：

1. 四组状态的转移表，以及取消、超时、重连和应用退出路径。
2. Tauri 与 AudioHost 的命令、事件、错误码和版本协商字段。
3. SQLite 表结构、FTS5 tokenizer 验证、迁移策略和删除语义。
4. AudioHost 启动、签名、权限、心跳、崩溃重启和安全退出约定。
5. AEC 与播放中语音打断的可行性实验和验收标准。
6. 唤醒对话、文字对话、手动打断、记忆确认四条关键时序测试。
7. 角色包最小 manifest，以及没有授权角色资产时的开发占位方案。

其中第 5 项若验证失败，不阻塞整个 MVP，但必须明确降级为快捷键和点击打断。

## 14. 参考项目与资料

- [realtime-ai-translation](https://github.com/mingheyuan/realtime-ai-translation)：Provider / Session、macOS Speech Bridge、partial/final 和资源生命周期。
- [realtime-ai-translation `src/asr.rs`](https://github.com/mingheyuan/realtime-ai-translation/blob/main/src/asr.rs)
- [realtime-ai-translation `src/streaming.rs`](https://github.com/mingheyuan/realtime-ai-translation/blob/main/src/streaming.rs)
- [realtime-ai-translation macOS Speech Bridge](https://github.com/mingheyuan/realtime-ai-translation/blob/main/bridges/macos-speech/MacOSSpeechBridge.swift)
- [Warashi 中文 README](https://github.com/inni918/warashi/blob/main/README.CN.md)：角色交互、语音链路和模块组织。
- [Warashi Memory System Design](https://github.com/inni918/warashi/blob/main/MEMORY_SYSTEM_DESIGN.md)
- [Warashi TTS Manager](https://github.com/inni918/warashi/blob/main/src/open_llm_vtuber/conversations/tts_manager.py)
- [Warashi Conversation Handler](https://github.com/inni918/warashi/blob/main/src/open_llm_vtuber/conversations/conversation_handler.py)
- [Codex Pets 官方说明](https://learn.chatgpt.com/docs/pets)：悬浮桌宠、活动优先级、位置持久化和减少动态效果。
- [Apple Speech Audio Buffer Recognition](https://developer.apple.com/documentation/speech/sfspeechaudiobufferrecognitionrequest)
- [AVSpeechSynthesizer Buffer API](https://developer.apple.com/documentation/avfaudio/avspeechsynthesizer/write%28_%3Atobuffercallback%3A%29)
- [Apple Voice-Processing I/O Audio Unit](https://developer.apple.com/documentation/audiotoolbox/audio-unit-voice-i-o)

参考项目只用于学习设计与边界，不意味着复制其代码、资产或许可不兼容的内容。
