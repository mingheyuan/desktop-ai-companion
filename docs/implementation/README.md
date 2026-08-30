# MVP 实现规格索引

> 状态：v0.4 实现基线已进入编码。文本、桌宠、显式记忆、首个 AudioHost 语音切片、网页打开和一次性本地计时提醒已落地；本文末尾的长期运行与实机 Spike 仍是 MVP 完成门槛。

这组文档把 `docs/architecture.md` 中的模块边界落实为状态、协议、数据结构和时序。实现发生冲突时，约束优先级为：

1. 本目录中的协议和不变量；
2. `docs/architecture.md` 的系统边界；
3. 根目录 `README.md` 的产品愿景。

需求发生变化时必须先修改相应规格，再修改代码，避免前端、Rust Core 与 macOS AudioHost 各自推断行为。

## 文档导航

- [当前实现状态](status.md)：当前代码事实、已验证闭环、协议子集和目标差距；继续编码前优先查看。
- [状态机](state-machines.md)：监听、输入、单轮对话、生成和播放状态，以及取消规则。
- [IPC 与 AudioHost 协议](ipc-protocol.md)：Tauri command/event、UDS 控制帧、PCM 二进制帧和版本协商。
- [SQLite 数据模型](database.md)：会话、消息、记忆候选、长期记忆和迁移规则。
- [Provider 接口](providers.md)：LLM、ASR、TTS 的抽象、流事件、取消和错误分类。
- [AudioHost](audio-host.md)：原生宿主生命周期、权限、持续监听与故障恢复。
- [关键时序](sequences.md)：文本、唤醒词、按键说话、打断和记忆确认的端到端流程。
- [VRM 运行时](vrm-runtime.md)：角色资源包、表现层、口型、点击和性能边界。
- [Live2D 运行时候选](live2d-runtime.md)：从公开 Live2D 桌宠项目提炼的加载、模型目录、适配器和授权边界；尚未决定替换 VRM。
- [代码级参考](../references.md)：固定到 commit 的参考项目、可借鉴点和禁止照搬的边界。

## 建议目录结构

```text
desktop-ai-companion/
├── apps/
│   └── desktop/
│       ├── src/                    # React 设置、聊天和调试 UI
│       └── src-tauri/
│           ├── migrations/         # SQLite 顺序迁移
│           └── src/
│               ├── app/            # 启停、快照、错误上报
│               ├── interaction/    # 状态机与 turn 协调
│               ├── audio/          # AudioHost client、KWS/VAD、播放队列
│               ├── memory/         # 检索、候选、确认与写入
│               ├── providers/      # LLM/ASR/TTS 接口与实现
│               ├── storage/        # SQLite repository 和迁移
│               ├── windowing/      # 桌宠窗口、托盘、快捷键
│               └── protocol/       # IPC 类型与事件 envelope
├── native/
│   └── audio-host/                 # Swift app bundle，固定 TCC 身份
├── contracts/
│   ├── audio-host/v1/              # Rust/Swift 共用 JSON Schema 与 fixtures
│   └── fixtures/                   # 协议兼容测试样本
├── resources/
│   └── characters/                 # 只提交有明确授权的角色清单与资源
└── docs/
```

Rust 是业务状态的唯一写入者。React 只渲染快照和发送意图；AudioHost 只负责 macOS 原生音频能力，不访问 SQLite，不直接调用 LLM，也不决定是否写记忆。

## 并发模型

- Tauri/Rust 主进程运行异步协调器；每次对话由一个 `turn_id + generation` 标识。
- 所有可能晚到的异步结果都必须携带 `turn_id` 和 `generation`；与当前活动代次不一致时直接丢弃。
- MemoryWorker 是记忆表的唯一写入者；对话协调器只能提交候选或确认命令。
- AudioHost 内部用串行控制队列修改麦克风、Apple Speech 和 TTS 生命周期；音频回调线程不得执行网络、SQLite 或 JSON 编解码。
- TTS 可以并发合成，但必须按 `chunk_index` 顺序播放；取消会同时终止生成器、待播队列和当前播放。

## 编码分期

1. **骨架与协议**：建立目录、类型、事件 envelope、SQLite migration runner 和空 Provider。
2. **文本闭环**：文本输入 → LLM 流式输出 → 消息落库 → React/VRM 状态反馈。
3. **AudioHost 闭环**：权限、UDS 握手、PTT、Apple Speech、TTS 和打断。
4. **持续监听**：KWS/VAD、后台 AudioHost、安静时段和监听可视反馈。
5. **记忆闭环**：检索、候选、确认/拒绝、更新/删除和上下文注入。
6. **桌宠体验**：VRM 动作、透明窗口、拖拽、点击穿透、托盘和快捷键。

当前落地映射：1、2 已完成首个闭环；3 已完成权限、UDS、PTT、Apple Speech、系统 TTS 和显式打断；4 暂用 Apple Speech 设备端短语检测 fallback，Rust PCM KWS/VAD 尚未接入；5 已完成用户显式 CRUD、FTS + 中文片段回退、opt-in 上下文注入，以及 `memory.save` 候选的持久化、确认/拒绝和重启后审核，更主动的自动提取仍属于后续增强；6 已完成窗口、角色本体拖拽/单击、导入、程序化姿态、近似口型和控制基础，正式 VRMA 与真实 RMS 口型属于后续增强。详见 [AudioHost 当前可运行切片](audio-host.md#0-当前可运行切片)。

屏幕理解继续通过独立 Tool Runtime 接入，不进入本轮核心 MVP 的完成条件；当前已落地低风险 `system.open_url`、一次性 `timer.create/list/cancel` 和本地 `memory.save` 的显式切片。普通聊天暂不向模型发送工具 schema，显式中文工具请求仍由 Rust 本地完成确认、执行、持久化和审计；模型主动 ToolCall 的单次循环实现保留，待自然对话稳定后再恢复。

## 编码前必须通过的 Spike

- **S1 透明窗口**：macOS 多显示器、全屏空间、点击穿透、拖拽和窗口恢复均可控。
- **S2 VRM**：目标设备上加载授权占位 VRM，空闲 CPU/GPU 和内存达到可接受水平。
- **S3 音频**：AudioHost 获得稳定 TCC 身份；连续监听 30 分钟，Apple Speech 滚动重启不丢最终文本。
- **S4 唤醒链路**：Rust KWS/VAD 的 PCM 延迟、误唤醒率和睡眠恢复可接受；模型许可证可分发。
- **S5 取消一致性**：连续打断、快速重启和 AudioHost 崩溃后，无幽灵字幕、幽灵语音或错误记忆写入。

Spike 失败时回到规格调整，不允许在多个模块中同时加入临时旁路。

## MVP 完成定义

- 文本输入和语音输入都能完成一轮对话，LLM、TTS、字幕和 VRM 表现状态一致。
- 唤醒词、持续监听、PTT、停止播放和新输入打断都可用，麦克风使用状态始终可见。
- 退出后重新启动能恢复会话和用户明确保存的记忆。
- 用户可明确创建、搜索、编辑和删除记忆；删除后 FTS 与本地片段检索中不残留正文。
- Provider 或 AudioHost 失败会进入可解释的降级状态，不使整个桌宠窗口失去响应。
- MVP 仅支持 VRM；不包含 Live2D、自动屏幕截图和任意工具执行。
