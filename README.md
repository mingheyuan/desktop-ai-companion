# Desktop AI Companion

一个常驻桌面的交互式 AI 角色项目。

项目希望结合“钢琴女友”类产品的陪伴感、记忆管理和低频互动，以及桌宠应用的便捷性、可移动性和可动表现，再加入自然的语音交互能力。

当前阶段已确定 MVP 的产品范围、技术选型和 v0.4 实现基线，并已进入业务实现。现有代码可以启动 Tauri 双窗口应用，运行透明桌宠、受控 VRM 导入、文字流式对话、可取消 turn、mock/OpenAI-compatible LLM Provider、SQLite 会话恢复，以及由 Rust 管理的桌宠显示、缩放、点击穿透、托盘和全局快捷键；显式记忆管理已接入，语音链路仍在实现中。工具调用与屏幕感知作为后续 Tool MVP 单独规划。

## 初始角色设定

项目的默认角色原型为《蔚蓝档案》的**天童爱丽丝**（Tendou Aris / Arisu）。

初始实现方向：

- 保留她认真、直接、略显生硬但逐渐学会表达情感的交流感觉。
- 保留游戏开发部成员、重度游戏爱好者和复古游戏语汇等核心设定摘要。
- 默认将用户称为“老师”，但关系称谓应当可以在设置中调整。
- 角色背景知识、用户记忆和运行时生成的互动记忆分开保存。
- 角色核心设定默认只读；需要通过配置或用户确认后才能修改。

角色资料记录在 [`docs/character/aris.md`](docs/character/aris.md)。

### 资料与素材边界

- 只收集公开可核验的角色背景、关系、行为特征和语言风格摘要。
- 不将原作完整剧情、台词、立绘、语音或游戏资源直接提交到仓库。
- 角色模型和声音只使用自制、用户拥有或明确授权的素材。
- 项目定位为个人开发原型；后续如公开发布，需要重新检查《蔚蓝档案》相关知识产权和素材授权。

## 产品愿景

让 AI 不只是一个聊天窗口，而是一个可以生活在桌面上的角色：

- 平时安静地存在于桌面上。
- 用户需要时可以快速聊天、说话或查看状态。
- 角色拥有可管理、可追溯、可控制的长期记忆。
- 角色能够通过动作、表情、视线和语音表达状态。
- 用户可以自由移动、缩放、隐藏或唤起角色。
- 主动互动有边界，支持安静时段和用户授权。

## 计划中的核心能力

### 角色与桌宠

- 无边框透明桌面角色窗口。
- 始终置顶、跨桌面和多显示器支持。
- 拖动、缩放、隐藏、唤起和鼠标穿透。
- 托盘菜单、全局快捷键和快速聊天入口。
- 闲置、聆听、思考、说话、开心、疲惫和睡眠等状态。
- 角色动作、表情、视线、呼吸、眨眼和口型同步。

### 交互

- 文字聊天。
- 语音输入和语音回复。
- 唤醒词和可关闭的持续监听。
- 按键说话、点击、双击、拖拽和角色注视等桌面交互。
- 对话过程中显示角色当前状态。
- 支持低频、异步的消息或信件式互动。
- 支持主动问候、提醒和安静时段控制。

### 记忆管理

记忆不是简单地保存全部聊天记录，而是分层管理：

- **会话记忆**：当前对话所需的上下文。
- **用户记忆**：姓名、偏好、习惯和重要事件。
- **关系记忆**：共同经历、互动历史和关系变化。
- **角色记忆**：角色背景、价值观、性格和行为边界。
- **情绪状态**：短期状态，可随时间衰减。

记忆系统需要支持：

- 搜索、查看、编辑、删除和导出。
- 用户明确控制“记住”和“不要记住”。
- 记录记忆来源、时间、重要程度和置信度。
- 处理重复记忆和相互冲突的信息。
- 角色核心人格不能被普通对话无条件覆盖。

### 工具与屏幕感知（规划能力）

- 在用户明确要求下，用默认浏览器打开经过校验的网页。
- 创建、查看和取消本地计时提醒，后续可选择同步到 Apple 提醒事项。
- 按需读取当前窗口、显示器或用户选定区域，不默认持续录屏。
- 屏幕图片可以交给独立的 Vision Provider，并使用单独的 endpoint、模型和 Keychain key。
- 工具调用具有参数校验、风险分级、授权确认、取消、超时和审计记录。
- “看见屏幕”不自动获得点击、输入、发送消息、删除数据或执行 Shell 的权限。

### 语音

语音交互会参考本人的 [realtime-ai-translation](https://github.com/mingheyuan/realtime-ai-translation) 项目，重点借鉴：

- 可替换的 ASR Provider / Session 结构。
- macOS 原生 Speech 桥接。
- 可选的本地 Sherpa-ONNX 识别。
- `partial` / `final` 语音识别事件。
- WebSocket 或事件流推送。
- 独立的原生悬浮窗口和系统权限处理。
- 模型、进程和麦克风资源按需加载与释放。

本项目在参考结构上补充语音合成、说话时的口型同步、TTS 打断和连续对话能力。

MVP 包含唤醒词和持续监听，并满足以下边界：

- 持续监听默认只用于本地唤醒词检测。
- 唤醒前的音频不发送给 LLM 或云端服务。
- 唤醒后才开始采集并处理一轮对话音频。
- 用户可以通过托盘、快捷键或设置随时关闭监听。
- 需要明确显示当前是否处于监听状态。

## 设计原则

1. **本地优先**：聊天记录、记忆和角色配置默认保存在本地。
2. **用户可控**：主动行为、麦克风、屏幕感知和记忆写入都应有清晰的开关。
3. **状态驱动**：动作和表情由统一的角色状态驱动，而不是由各个功能模块直接控制角色。
4. **可替换服务**：ASR、LLM、TTS、Embedding 和角色渲染器都保留替换空间。
5. **渐进式实现**：先完成稳定的桌宠和对话闭环，再扩展长期记忆、主动行为和工具能力。
6. **尊重素材授权**：角色模型、声音和第三方 SDK 均需单独确认授权范围。

## 项目文档

- [`docs/architecture.md`](docs/architecture.md)：MVP 内部架构、数据所有权、状态模型、语音与记忆流程。
- [`docs/implementation/README.md`](docs/implementation/README.md)：可直接指导编码的状态机、协议、数据库、Provider、AudioHost、时序和 VRM 规格索引。
- [`docs/tools-and-vision.md`](docs/tools-and-vision.md)：工具调用、闹钟/提醒、屏幕观察、独立多模态 Provider 与真实参考项目。
- [`docs/character/aris.md`](docs/character/aris.md)：初始角色原型资料与实现边界。
- [`docs/references.md`](docs/references.md)：固定到 commit 的代码级参考仓库、具体文件和采用边界。

## MVP 范围（当前确认）

MVP 目标是先完成一个可日常使用的桌面 AI 伙伴闭环，默认首发平台为 macOS：

### MVP 必须具备

- 透明、无边框、始终置顶的桌宠窗口。
- 角色拖动、缩放、隐藏、唤起、托盘菜单和快捷键。
- 基础闲置、聆听、思考和说话状态。
- 文字聊天和流式回复。
- 按键说话。
- 唤醒词和可关闭的持续监听。
- 实时语音识别、语音回复、TTS 停止与打断。
- 聊天记录本地保存。
- 用户明确触发的记忆保存。
- 记忆查看、搜索、编辑和删除。
- 后续对话检索相关记忆。
- 麦克风状态、持续监听开关和数据清理入口。

### MVP 暂不包含

- 多角色系统和移动端。
- 工具执行、按需屏幕理解和通用 GUI 自动化；已作为后续 Tool MVP 单独规划。
- 自主生活、主动发消息和复杂情绪成长。
- 完整换装系统。

### MVP 完成标准

用户启动程序后，可以看到并移动桌宠；点击角色可以聊天；用户可以输入文字、按住快捷键说话，或使用唤醒词开始一轮语音对话；AI 回复时角色能够显示状态并朗读；用户明确保存一条记忆后，下一次相关对话能够检索并使用它；用户可以在记忆界面删除这条记忆。

## MVP 技术栈（锁定）

### 平台与桌面容器

- 首发平台：macOS 13+。
- 桌面容器：Tauri 2。
- Rust 负责窗口、托盘、快捷键、权限协调、进程管理和核心业务。
- 支持透明、无边框、始终置顶和可切换鼠标穿透的桌宠窗口。

### 前端与角色渲染

- React + TypeScript + Vite。
- Three.js + `@pixiv/three-vrm`。
- MVP 只支持 VRM 角色，不同时引入 Live2D。
- Zustand 管理前端 UI 状态；角色行为由后端事件和统一状态机驱动。
- 首批动作只覆盖闲置、聆听、思考、说话和睡眠。

### 核心服务

- Rust 2021 + Tokio。
- 类型化 Tauri Command 负责低频请求，Tauri Event 负责状态和批量流式事件；原始音频不经过 WebView JSON 通道。
- `reqwest` 连接 OpenAI-compatible API，并使用 SSE 接收流式回复。
- 支持云端模型，也支持 Ollama、LM Studio 等本地兼容服务。
- API Key 使用 macOS Keychain 保存，普通设置保存在本地配置中。

### 记忆与数据

- SQLite + FTS5，Rust 侧使用 `rusqlite`。
- MVP 不引入向量数据库和 Embedding 服务。
- 会话和关系状态使用独立数据表；用户/关系长期记忆在统一 `memories` 表中按 scope 与 character_id 隔离。
- 记忆写入先生成候选，由用户明确确认后保存。

### 语音链路

- 持久 Swift `AudioHost.app` 负责麦克风、`AVAudioEngine`、系统权限、PCM 音频采集和播放。
- Rust 与 AudioHost 之间使用 Unix Domain Socket：JSONL 控制通道与二进制 PCM 媒体通道分离。
- Apple Speech 作为第一版默认 ASR Provider，提供 `partial / final` 结果。
- Sherpa-ONNX 作为可选的本地 ASR Provider。
- Sherpa-ONNX KeywordSpotter 负责本地唤醒词检测，VAD 负责判断一轮语音结束。
- `AVSpeechSynthesizer` Buffer API 作为第一版本地 TTS，支持停止、打断、播放进度和基础口型数据。
- 持续监听只保留本地唤醒词检测；唤醒前音频不发送给 LLM 或云端。

语音 Provider 会沿用 [realtime-ai-translation](https://github.com/mingheyuan/realtime-ai-translation) 中的 Provider / Session、Bridge、partial/final 和资源按需加载思路，但不会把翻译状态机直接复制到对话系统。

### 构建与验证

- JavaScript 包管理器：pnpm。
- Rust 构建：Cargo。
- macOS 应用打包：Tauri Bundler。
- Rust 单元测试：`cargo test`。
- 前端单元测试：Vitest。
- 关键交互测试：后续使用 Playwright 或 Tauri 对应的端到端方案。

## 内部实现基线（v0.4）

完整方案见 [`docs/architecture.md`](docs/architecture.md)。当前确认的关键决策：

- Rust 核心是业务状态唯一事实来源；React 保存可重建的展示投影，Swift AudioHost 保存瞬时音频状态，SQLite 保存持久数据。
- `InteractionSupervisor` 统一编排对话；每轮使用独立 `TurnActor`，通过 `generation` 和取消令牌屏蔽迟到事件。
- 监听模式、输入、对话和播放使用四组正交状态；活动 turn 另有 GenerationState 表达 LLM 与 TTS 并行，动作、表情和口型是统一状态派生结果。
- WebView 与 Rust 使用类型化 Tauri IPC；Rust 与持久 AudioHost 使用 UDS JSONL 控制通道和二进制 PCM 通道。
- 记忆通过单一 `MemoryWorker` 串行写入；自动提取只生成待确认候选，只有已确认记忆参与检索。
- 桌宠悬浮层与主面板分离，并使用 `ActivityStore + AttentionResolver` 处理多个活动的呈现优先级。
- 播放中自然语音打断必须先验证 AEC；若不稳定，MVP 明确降级为快捷键和点击打断。
- 后续工具由 Rust `ToolRuntime` 统一注册、授权、执行和审计；模型不能直接操作系统。
- 屏幕理解采用按需单帧和独立 Vision Provider，聊天 Key 与视觉 Key 默认隔离。

上述细节已经写入 [`docs/implementation/`](docs/implementation/README.md)。进入编码前还需完成一次规格评审，并通过透明窗口、VRM、AudioHost、唤醒链路和取消一致性五项技术 Spike。

## 后续可替换点

- Tauri 可以在跨平台或复杂窗口需求出现时重新评估 Electron。
- VRM 可以在确有需求时增加 Live2D 渲染器。
- Apple Speech、Sherpa-ONNX、TTS 和 LLM 都通过 Provider 接口隔离。
- SQLite FTS5 可以在记忆规模扩大后增加向量检索。
- Sherpa-ONNX 是 MVP 的默认本地唤醒词方案；Porcupine 可作为需要更成熟自定义唤醒词模型时的可选方案。
- 内置工具可以在后续通过官方 Rust MCP SDK 扩展外部 MCP Server，但不改变本项目的权限策略。
- 屏幕采集优先验证 Rust `scap`；需要更细 macOS 控制时再评估 `screencapturekit-rs`。

## 建议的产品阶段

### 阶段 0：产品与技术决策

- 评审并冻结 v0.4 内部实现基线。
- 复核已完成的角色状态转移、IPC 事件、记忆数据模型和 SQLite 迁移设计。
- 复核已定义的 ASR、LLM、TTS、AudioHost 和角色渲染器接口。
- 验证 AEC、Apple Speech pre-roll、FTS5 中文检索等高风险点。
- 最后初始化 Tauri、Rust 和前端项目骨架。

### 阶段 1：桌宠最小闭环

- 桌面角色窗口。
- 移动、缩放、隐藏、唤起和托盘控制。
- 基础闲置动作。
- 文字聊天。

### 阶段 2：语音闭环

- 按键说话。
- 唤醒词和可关闭的持续监听。
- 流式语音输入。
- AI 语音回复。
- 聆听、思考和说话状态。

### 阶段 3：记忆闭环

- 本地会话存储。
- 记忆提取、检索和冲突处理。
- 记忆管理界面。
- 用户确认和删除机制。

### 阶段 4：工具与按需屏幕理解

- 受控 Tool Runtime、风险策略和审计记录。
- 打开网页与本地计时提醒。
- 按需单帧屏幕观察。
- 独立 Vision Provider 与 Keychain key。
- 可选 Apple 提醒事项与本地 MCP 扩展。

### 阶段 5：角色表现与主动陪伴

- 表情、动作、视线和口型同步。
- 主动问候、异步消息和安静时段。
- 情绪和关系状态。

## 当前状态

项目已完成首个可运行实现切片：React/Tauri 双窗口、three-vrm 运行时适配、Rust 状态核心、流式 LLM Provider、取消代次、SQLite v1 schema、会话恢复和 Provider 设置均已通过编译或单元测试。API key 通过应用设置写入 macOS Keychain，不进入 SQLite。原生 AudioHost、语音和记忆闭环仍待继续实现。

下一步继续实现 Provider 设置与 Keychain、VRM 资源导入和桌宠控制，再接入 AudioHost、PTT/ASR/TTS、唤醒词和记忆闭环。

## 本地开发

```bash
pnpm install
pnpm build
cargo test --workspace
pnpm tauri dev
```

应用默认使用 `mock` Provider，因此无需 API key 即可验证文字流式对话。点击聊天面板右上角设置可切换 OpenAI-compatible endpoint/model，并将 API key 保存到 macOS Keychain；开发期也可使用环境变量 `DESKTOP_AI_LLM_API_KEY` 作为回退。

## 目录约定

- `docs/`：产品、架构、实现规格和调研记录。
- `apps/desktop/`：React/Tauri 桌面应用。
- `native/AudioHost/`：Swift 原生音频宿主。
- `contracts/`：Rust/Swift 协议 schema 与兼容 fixtures。
- `resources/characters/`：仅存放自制、用户拥有或明确授权的角色资源。

## License

License 待项目方案确定后补充。
