# MVP 实现状态矩阵

> 更新时间：2026-08-30。本文描述当前代码事实；其余实现文档描述目标架构。两者冲突时，先在这里记录差异，再修改代码和目标规格。

## 已形成可运行闭环

| 能力 | 当前实现 | 验证 |
|---|---|---|
| 桌面容器 | Tauri 2 主面板 + 透明桌宠双窗口 | Rust 编译、前端构建、手工窗口检查 |
| VRM | `three-vrm` 加载、合法性/大小校验、应用数据目录托管、授权模型导入和 fallback 降级 | 前端构建和浏览器渲染检查；正式角色仍待授权 VRM 验收 |
| 默认角色 | 天童爱丽丝的人格摘要进入 LLM system prompt；角色规则与不可信用户记忆分区 | Provider 单元测试、角色资料人工复核 |
| 文字对话 | mock/OpenAI-compatible SSE、单活动 turn、generation + cancellation；API key 可选以支持 Ollama/LM Studio | Rust 单元测试、前端状态测试 |
| 视觉 Provider 配置 | 独立启用开关、endpoint、model 和 Keychain API key 已保存；`screen.observe` 已可在用户确认后生成进程内存中的一次性本地预览，界面显示自动清除倒计时，支持重新读取、手动清除和 5 分钟自动过期，但尚未采集后上传 | Provider 设置 UI、Rust 配置校验、本地截图预览清除与自动过期；视觉模型数据出站仍待单独明确授权和实现 |
| 本地数据 | SQLite migration、会话恢复、消息提交 | Rust repository 测试和真实数据库迁移 |
| 显式记忆 | 创建、FTS 搜索、中文二字片段回退、编辑、物理删除、审计、可选上下文注入；聊天中明确“忘记/删除某条记忆”时支持本地唯一匹配、确认和版本校验删除；模型或明确保存请求先写入持久化候选，审核通过后才转为正式记忆 | SQLite 事务/FTS 测试、候选确认/拒绝幂等测试、保存→自然改写检索→Provider 使用→删除闭环测试、聊天删除解析/唯一匹配测试、UI 检查 |
| 受控网页工具 | 用户明确输入“打开/访问 + HTTP(S) 网址”后，展示确认卡片；允许后调用 macOS 默认浏览器，拒绝或取消不执行；每次请求写入 `tool_runs` 审计表，主面板可查看最近工具记录的摘要投影 | URL 解析/危险 scheme/凭据测试、工具审计测试、前端构建；真实默认浏览器打开仍需手工验收 |
| 一次性本地提醒 | 支持“半小时后提醒我喝水”以及“明天下午三点提醒我开会”等明确绝对日期/时间；确认后写入 SQLite，应用内调度器和 macOS 用户级 `launchd` LaunchAgent 共享本地 claim 去重；主面板可查看、实时显示剩余时间、取消并查看最近已触发/已取消历史，聊天可查询待处理提醒，也可对唯一匹配的提醒发起确认后取消；重启会恢复未完成任务，应用完全退出后仍可由系统投递 | Rust 本地时区解析/迁移/状态转换、提醒历史查询、只读本地查询与 LaunchAgent 脚本生成测试、前端倒计时测试和构建；绝对时间仅限明确且未来 7 天内的一次性提醒，重复规则、贪睡和退出登录期间的投递仍未实现；真实通知中心行为仍需 macOS 实机验收 |
| 桌宠控制 | 角色本体可拖动、单击打开聊天；支持显示、缩放、点击穿透、托盘、窗口恢复和全局快捷键；托盘与 `⌘/Ctrl + Shift + L` 可控制持续监听 | Tauri capability 编译、Rust 编译和手工 UI 检查 |
| 语音基础 | 独立 AudioHost、权限、PTT、partial/final、系统 TTS、停止/新输入打断；持续监听断线后限三次退避恢复 | Swift 编译、UDS 真实握手测试 |
| 持续监听 | Apple Speech 设备端识别检测“爱丽丝”；有命令时 1.1 秒 endpoint，只唤醒无命令时 5 秒返回待机 | 代码保护已验证；仍需首次授权和长时间实机测试 |
| 语音数据路径 | `capture.started.on_device` 投影为“本机识别/系统语音服务”；wake 只允许本机 | Rust/React 类型检查；仍需两类设备能力实机检查 |
| macOS 打包 | Tauri resources 内嵌 `AudioHost.app`；稳定 identity 优先、ad-hoc fallback | debug app bundle 构建、`codesign --verify --deep --strict` |
| 数据清理 | 本地范围摘要；聊天/记忆独立清理；二次确认；保留 Provider/角色/桌宠设置 | SQLite 范围隔离、candidate cascade、FTS rebuild 测试和 UI 检查 |

## 当前 AudioHost 协议（v1.0 子集）

- Rust 创建 `/tmp/dac-<uuid>.sock` 和权限 `0600` 的 token 文件，再用 `/usr/bin/open -n -W -a` 启动 `AudioHost.app`。
- Host 首帧为 `host.hello`，包含 protocol major/minor、256-bit token、launch nonce、PID 和 capability；Rust 校验后回复 `host.ack`。
- Core → Host：`permissions.request`、`permissions.status`、`settings.open_privacy`、`listening.set`、`asr.start`、`asr.finalize`、`asr.stop`、`tts.speak`、`tts.stop`、`shutdown`。
- Host → Core：`permissions.status`、`host.ready`、`capture.started`、`wake.detected`、`asr.partial`、`asr.final`、`tts.started`、`tts.level`、`tts.finished`、`voice.error`、`host.error`。
- ASR 使用 `capture_id + generation + recognition_epoch + revision` 拒绝过期回调；TTS 使用 `turn_id + generation` 拒绝过期播放回调。
- 当前只有 JSONL control socket，不传输 PCM；socket 明确断线已有三次退避恢复，二进制 media socket、heartbeat、request/response ACK 和半开故障检测属于下一协议小版本。

当前已注册的语音 Tauri commands：`voice_request_permissions`、`voice_open_privacy_settings`、`voice_set_listening_mode`、`voice_ptt_start`、`voice_ptt_finish`、`playback_stop`、`voice_update_settings`；工具决策 command 为 `tool_decide`。React 不直接连接 AudioHost，所有状态以 Rust `AppSnapshot` 为准。权限被拒绝后，UI 可打开系统设置并显式刷新状态；`restricted` 只提示系统策略限制，不重复请求。

## 与目标规格的明确差距

| 目标 | 当前差距 | 收口条件 |
|---|---|---|
| 稳定 TCC 身份 | 已提供本地签名脚本，但尚未在本机执行 | 用户确认执行脚本后完成首次授权/重建复测 |
| 本地 KWS/VAD | 当前为 Apple Speech 设备端短语检测 fallback | 接入可分发 Sherpa 模型、PCM media socket、VAD 和误唤醒测试 |
| TTS 口型 | 发音区间 pulse 已经由轻量事件驱动 VRM `aa`，但不是音频振幅 | buffer 合成或可靠 PCM/RMS 回调替换近似 level |
| 音频恢复 | control socket 每 2 秒 heartbeat；当前连接连续 6 秒无 pong 或写入失败会在用户仍期望持续监听时做三次退避重启，主动关闭/权限错误会取消恢复；仍无设备切换和睡眠恢复 | 完成半开故障、30 分钟/睡眠/外设验收 |
| 角色表现 | 已有原创程序化爱丽丝预览、idle/listen/think/speak 姿态、眨眼、呼吸和近似口型；授权 VRM 加载后优先替换 | 授权 VRM + 正式 VRMA 动作验收 |
| 记忆候选 | 用户可显式 CRUD；模型或明确保存请求会先创建 `pending` 候选；记忆面板可编辑候选正文并在应用重启后继续审核，确认通过以事务方式写入长期记忆、FTS 和审计，拒绝只标记候选状态 | 更主动的自动提取、`relationship` scope 候选和候选去重仍待补齐 |
| 模型工具循环 | OpenAI-compatible 请求已通过 `ToolRegistry` 发送 `system.open_url`/`timer.create`/`timer.cancel`/`memory.save`/本地 `screen.observe` schema；`ToolPolicy` 负责 JSON、URL、时间、标题、记忆和屏幕观察请求校验；SSE 分片会还原单次 ToolCall，进入确认卡后执行结果回传模型继续生成，每轮最多 3 步；`screen.observe` 仅在用户当前消息明确要求查看屏幕时允许，图片只生成本地临时预览；显式中文意图入口仍保留，`memory.delete` 仅允许用户显式触发，不暴露给模型；主面板可查看最近工具审计摘要，不展示原始参数 | 已有本地 HTTP Provider、集中参数安全校验和 ToolCall 消息测试；Vision 图片出站授权、多工具并行、更细风险分级和完整端到端 UI 验收仍待补齐 |

## 当前验证命令

```bash
./scripts/build-audio-host.sh
cargo clippy --manifest-path apps/desktop/src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml
pnpm test
pnpm build
pnpm package:macos --debug
```

真实 AudioHost 进程握手测试不会请求权限：

```bash
cargo test --manifest-path apps/desktop/src-tauri/Cargo.toml \
  audio::tests::launches_audio_host_and_completes_handshake -- --ignored
```
