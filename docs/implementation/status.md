# MVP 实现状态矩阵

> 更新时间：2026-08-28。本文描述当前代码事实；其余实现文档描述目标架构。两者冲突时，先在这里记录差异，再修改代码和目标规格。

## 已形成可运行闭环

| 能力 | 当前实现 | 验证 |
|---|---|---|
| 桌面容器 | Tauri 2 主面板 + 透明桌宠双窗口 | Rust 编译、前端构建、手工窗口检查 |
| VRM | `three-vrm` 加载、合法性/大小校验、应用数据目录托管、fallback 模型 | 前端构建和浏览器渲染检查 |
| 文字对话 | mock/OpenAI-compatible SSE、单活动 turn、generation + cancellation | Rust 单元测试、前端状态测试 |
| 本地数据 | SQLite migration、会话恢复、消息提交 | Rust repository 测试和真实数据库迁移 |
| 显式记忆 | 创建、FTS 搜索、编辑、物理删除、审计、可选上下文注入 | SQLite 事务/FTS 测试、UI 检查 |
| 桌宠控制 | 显示、缩放、点击穿透、托盘、窗口恢复、全局快捷键；托盘与 `⌘/Ctrl + Shift + L` 可控制持续监听 | Rust 编译和手工 UI 检查 |
| 语音基础 | 独立 AudioHost、权限、PTT、partial/final、系统 TTS、停止/新输入打断；持续监听断线后限三次退避恢复 | Swift 编译、UDS 真实握手测试 |
| 持续监听 | Apple Speech 设备端识别检测“爱丽丝”；1.1 秒 endpoint | 代码保护已验证；仍需首次授权和长时间实机测试 |
| 语音数据路径 | `capture.started.on_device` 投影为“本机识别/系统语音服务”；wake 只允许本机 | Rust/React 类型检查；仍需两类设备能力实机检查 |
| macOS 打包 | Tauri resources 内嵌 `AudioHost.app`；稳定 identity 优先、ad-hoc fallback | debug app bundle 构建、`codesign --verify --deep --strict` |
| 数据清理 | 本地范围摘要；聊天/记忆独立清理；二次确认；保留 Provider/角色/桌宠设置 | SQLite 范围隔离、candidate cascade、FTS rebuild 测试和 UI 检查 |

## 当前 AudioHost 协议（v1.0 子集）

- Rust 创建 `/tmp/dac-<uuid>.sock` 和权限 `0600` 的 token 文件，再用 `/usr/bin/open -n -W -a` 启动 `AudioHost.app`。
- Host 首帧为 `host.hello`，包含 protocol major/minor、256-bit token、launch nonce、PID 和 capability；Rust 校验后回复 `host.ack`。
- Core → Host：`permissions.request`、`permissions.status`、`settings.open_privacy`、`listening.set`、`asr.start`、`asr.finalize`、`asr.stop`、`tts.speak`、`tts.stop`、`shutdown`。
- Host → Core：`permissions.status`、`host.ready`、`capture.started`、`wake.detected`、`asr.partial`、`asr.final`、`tts.started`、`tts.level`、`tts.finished`、`voice.error`、`host.error`。
- ASR 使用 `capture_id + generation + recognition_epoch + revision` 拒绝过期回调；TTS 使用 `turn_id + generation` 拒绝过期播放回调。
- 当前只有 JSONL control socket，不传输 PCM；二进制 media socket、heartbeat、request/response ACK 和 supervisor 自动重启属于下一协议小版本。

当前已注册的语音 Tauri commands：`voice_request_permissions`、`voice_open_privacy_settings`、`voice_set_listening_mode`、`voice_ptt_start`、`voice_ptt_finish`、`playback_stop`、`voice_update_settings`。React 不直接连接 AudioHost，所有状态以 Rust `AppSnapshot` 为准。权限被拒绝后，UI 可打开系统设置并显式刷新状态；`restricted` 只提示系统策略限制，不重复请求。

## 与目标规格的明确差距

| 目标 | 当前差距 | 收口条件 |
|---|---|---|
| 稳定 TCC 身份 | 已提供本地签名脚本，但尚未在本机执行 | 用户确认执行脚本后完成首次授权/重建复测 |
| 本地 KWS/VAD | 当前为 Apple Speech 设备端短语检测 fallback | 接入可分发 Sherpa 模型、PCM media socket、VAD 和误唤醒测试 |
| TTS 口型 | 发音区间 pulse 已经由轻量事件驱动 VRM `aa`，但不是音频振幅 | buffer 合成或可靠 PCM/RMS 回调替换近似 level |
| 音频恢复 | socket 断线会在用户仍期望持续监听时做三次退避重启，主动关闭/权限错误会取消恢复；仍无 heartbeat、设备切换和睡眠恢复 | 完成 heartbeat 与 30 分钟/睡眠/外设验收 |
| 角色表现 | 已有 idle/listen/think/speak 程序化骨骼姿态、眨眼、呼吸和 fallback 口型 | 授权 VRM + 正式 VRMA 动作验收 |
| 记忆候选 | 用户可显式 CRUD；无模型自动候选确认流 | 候选表、确认/拒绝 UI 和冲突策略 |

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
