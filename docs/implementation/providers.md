# Provider 接口与流式处理

## 1. 目标

业务协调器只依赖 LLM、ASR、TTS 的稳定接口，不依赖某家 SDK。Provider 实现负责认证、协议差异、速率限制和错误映射；它不能直接修改 UI、SQLite 记忆或全局状态。

MVP 基线：

- LLM：一个 OpenAI-compatible SSE provider，支持自定义 base URL、model 和 Keychain credential ref；开发模式也支持被 Git 忽略的 `.env` 环境变量，并通过 Agent Loop 将历史消息、模型流和后续工具结果串起来。
- ASR：Apple Speech（经 AudioHost）为 macOS 默认；离线实现保留注册位，不阻塞 MVP。
- TTS：Apple AVSpeechSynthesizer（经 AudioHost）为默认；云端 TTS 作为后续实现。
- KWS/VAD：属于音频管线，不伪装成 ASR Provider。

## 2. 公共能力与错误

```rust
struct ProviderCapabilities {
    streaming: bool,
    cancellation: bool,
    locales: Vec<String>,
    voices: Vec<String>,
    max_input_units: Option<u32>,
}

enum ProviderErrorKind {
    Authentication,
    PermissionDenied,
    RateLimited,
    Timeout,
    Unavailable,
    InvalidRequest,
    Cancelled,
    Protocol,
    Internal,
}

struct ProviderError {
    kind: ProviderErrorKind,
    retryable: bool,
    retry_after_ms: Option<u64>,
    diagnostic_id: String,
    safe_message: String,
}
```

错误正文必须脱敏。日志不得包含 Authorization header、Keychain 内容、完整 system prompt、原始音频或未裁剪屏幕图像。

所有 start 方法接收 cancellation primitive；drop stream 也必须触发取消。Provider 回调先经过 generation guard，再进入状态机。

## 3. LLM Provider

```rust
trait LlmProvider: Send + Sync {
    fn id(&self) -> &'static str;
    fn capabilities(&self) -> ProviderCapabilities;
    async fn start(
        &self,
        request: LlmRequest,
        cancel: CancellationToken,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<LlmEvent, ProviderError>> + Send>>, ProviderError>;
}

struct LlmRequest {
    messages: Vec<LlmMessage>,
    memory_context: Vec<String>,
    tools: Vec<ToolDefinition>,
    tool_steps: u8,
}

enum LlmEvent {
    TextDelta(String),
    ToolCalls(Vec<ToolCall>),
    Completed,
}
```

普通对话的 Agent Loop 会把当前输入与历史消息交给配置的真实 LLM，只有收到 LLM 的文本完成事件才写入助手消息；不会把用户输入拼接成固定回复。OpenAI-compatible Provider 会在启用工具时发送严格 schema；模型主动 ToolCall 经过 Rust 校验和用户确认后，工具结果作为 `tool` 消息回传模型，继续生成最终文本。当前每次只接受一个调用，每轮最多连续 3 步，连接建立失败最多有限重试；显式中文工具请求仍由 Rust 本地解析、确认和执行。已有校验仍拒绝未知工具、非法 JSON 或危险参数，不能把参数当作普通文本或 Shell 指令执行。
确认卡片默认有 2 分钟生命周期。超时由 Core 负责清理并将对应 `tool_run` 记为 `cancelled`；`memory.save` 的 pending 候选同时标记为 `rejected`。超时路径不调用工具执行器、不产生外部副作用；模型 ToolCall 会收到未执行结果，显式用户请求则结束当前回合。确认、取消、超时三条路径通过同一交互锁串行化，并以工具调用 ID、回合 ID 和 generation 防止旧任务误伤新请求。

### 上下文装配顺序

1. 应用安全边界和输出约束；
2. 角色核心设定与当前角色表现状态；
3. 明确标注为“资料”的已确认 user/relationship memories；
4. 受 token 预算裁剪的最近会话；
5. 当前用户输入。

角色设定和记忆都不能覆盖应用安全边界。对第三方角色的专有台词和素材不内置到公开仓库。

### 流与重试

- SSE 解码器必须处理跨网络 chunk 的 UTF-8/JSON 边界。
- 文本 delta 进入 canonical buffer；UI 事件按 30ms 或 24 字符合批。
- 句子分割器从 canonical buffer 产生不可变 TTS chunk；不得回写 canonical 文本。
- 收到第一个有效 delta 之前，网络超时最多自动重试 1 次；之后失败不自动重放，以免生成两份不同回复。
- 429 只按 `retry_after` 提示用户，不在前台 turn 内长时间静默等待。

## 4. ASR Provider

```rust
trait AsrProvider: Send + Sync {
    fn id(&self) -> &'static str;
    async fn start(
        &self,
        config: AsrConfig,
        cancel: CancellationToken,
    ) -> Result<Box<dyn AsrSession>, ProviderError>;
}

trait AsrSession: Send {
    async fn next_event(&mut self) -> Result<Option<AsrEvent>, ProviderError>;
    async fn finalize(&mut self) -> Result<(), ProviderError>;
    async fn stop(&mut self) -> Result<(), ProviderError>;
}

enum AsrEvent {
    Ready { locale: String },
    Partial { revision: u64, text: String },
    Final { text: String },
}
```

- `Partial` 是替换语义，不是 append；revision 小于等于已见值时丢弃。
- `Final` 每个 capture 最多一次。Apple Speech 内部任务滚动只增加 recognition epoch，不得产生用户级 Final。
- `finalize` 用于 VAD endpoint/PTT release；`stop` 用于取消，后者不能提交 Final。
- AudioHost 断线使 session 返回一次 Unavailable，然后结束；协调器决定是否重连。

这沿用 `realtime-ai-translation` 中 Provider/Session 分离、Swift generation guard 和有界停止的成熟结构，同时增加协议版本、心跳和用户级 Final 约束。

## 5. TTS Provider

```rust
trait TtsProvider: Send + Sync {
    fn id(&self) -> &'static str;
    async fn synthesize(
        &self,
        request: TtsRequest,
        cancel: CancellationToken,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<TtsEvent, ProviderError>> + Send>>, ProviderError>;
}

struct TtsRequest {
    turn_id: String,
    generation: u64,
    chunk_index: u32,
    canonical_text: String,
    spoken_text: String,
    voice: String,
    rate: f32,
    pitch: f32,
}

enum TtsEvent {
    DelegatedToAudioHost,
    AudioFormat { sample_rate: u32, channels: u8, encoding: String },
    AudioFrame { bytes: Bytes },
    Completed,
}
```

Apple TTS Provider 把任务委托给 AudioHost；云端 Provider 将音频帧交给统一播放队列，再经 media socket 播放。两者对协调器暴露相同的 chunk 生命周期。

借鉴 Warashi：多个 chunk 可以并发合成并用 index 缓存，但 sender 只按序下发；`clear/cancel` 必须终止生成器、sender、buffer 和当前音频。系统分别保存：

- `canonical_text`：LLM 原始可记忆文本；
- `display_text`：UI 可选格式化文本；
- `spoken_text`：发音替换、读法或省略标记后的文本。

## 6. ProviderRegistry 与配置

```text
ProviderRegistry
├── LLM profile -> factory -> health state
├── ASR profile -> factory -> health state
└── TTS profile -> factory -> health state
```

- profile 保存 provider id、endpoint、model/voice、locale、超时和 Keychain ref。
- factory 创建客户端，启动时不进行昂贵网络调用。
- health 状态为 unknown/ready/degraded/unavailable，并带最后检查时间。
- 切换 profile 只影响新 turn；活动 turn 继续使用其创建时捕获的 provider 实例。
- 配置更新使用 revision 乐观锁；秘密写 Keychain 成功后才提交 profile 引用。
- 删除 profile 前检查是否为当前默认；删除 Keychain item 失败时明确告警，不假装成功。

## 7. 超时预算

| 操作 | 默认超时 | 处理 |
|---|---:|---|
| LLM 连接/首 token | 15s | 首 delta 前可重试一次 |
| LLM 流空闲 | 30s | 失败并保留已有文本 |
| ASR ready | 15s | 降级到文本输入 |
| ASR finalize | 5s | 失败，不提交 partial |
| TTS 首 chunk | 10s | 保留文本并跳过语音 |
| AudioHost stop ack | 3s | 逻辑取消后重启 host |

超时都可配置但有上下限，避免错误配置永久占用前台 turn。

## 8. Provider 合同测试

每个实现必须通过相同测试套件：

- start → delta/partial → completed/final 的正常流；
- start 前、首事件后和结束竞争时取消；
- 晚到事件被 generation guard 丢弃；
- Unicode 与跨 chunk JSON 边界；
- 认证、429、超时、断线和畸形响应映射；
- 日志脱敏；
- TTS 并发完成乱序但播放有序；
- ASR Final 最多一次，stop 不产生 Final。
