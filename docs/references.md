# 代码级参考项目

这些仓库以独立浅克隆放在 `/Users/mingyuan.wang/my/` 下，只用于阅读和验证设计，不作为 `desktop-ai-companion` 的子模块、运行时依赖或提交内容。提交任何复制/改写代码前仍需逐项核对上游许可证和 attribution 要求。

## 固定参考版本

| 项目 | 本地路径 | 参考 commit | 主要用途 |
|---|---|---|---|
| [realtime-ai-translation](https://github.com/mingheyuan/realtime-ai-translation) | `/Users/mingyuan.wang/my/realtime-ai-translation` | [`7361fe3`](https://github.com/mingheyuan/realtime-ai-translation/commit/7361fe343d20efc68168e891ce3a18ca112aea2e) | Rust ASR Provider/Session、Swift Speech bridge、取消代次 |
| [Warashi](https://github.com/inni918/warashi) | `/Users/mingyuan.wang/my/参考/warashi` | [`8c71103`](https://github.com/inni918/warashi/commit/8c71103978dbd34b8595af34eefd14c3028655f6) | 对话中断、TTS 有序队列、记忆检索与完成后整理 |
| [three-vrm](https://github.com/pixiv/three-vrm) | `/Users/mingyuan.wang/my/参考/three-vrm` | [`76e0109`](https://github.com/pixiv/three-vrm/commit/76e0109ef36f30f620e7c7883c2f893b22ca72d4) | VRM 加载、表情、look-at、动画与资源优化 |
| [Tauri plugins workspace](https://github.com/tauri-apps/plugins-workspace) | `/Users/mingyuan.wang/my/参考/tauri-plugins` | [`9d6d032`](https://github.com/tauri-apps/plugins-workspace/commit/9d6d03269a3dea3e9dbf4635fe0fd68f23f750fa) | 全局快捷键、窗口状态、通知和平台线程约束 |
| [create-tauri-app](https://github.com/tauri-apps/create-tauri-app) | `/Users/mingyuan.wang/my/参考/create-tauri-app` | [`2b8ce19`](https://github.com/tauri-apps/create-tauri-app/commit/2b8ce19844f37aaae8fd483e5a60ac0add9c2924) | 当前 Tauri 2 + React/TypeScript 工程模板与配置字段 |

## 1. realtime-ai-translation

重点文件：

- [`src/asr.rs`](https://github.com/mingheyuan/realtime-ai-translation/blob/7361fe343d20efc68168e891ce3a18ca112aea2e/src/asr.rs)：`AsrProvider`/`AsrSession` 分层、Ready/Partial/Final/Error 事件、短 UDS 路径、启动和停止超时。
- [`bridges/macos-speech/MacOSSpeechBridge.swift`](https://github.com/mingheyuan/realtime-ai-translation/blob/7361fe343d20efc68168e891ce3a18ca112aea2e/bridges/macos-speech/MacOSSpeechBridge.swift)：app bundle TCC 身份、串行控制队列、generation guard、Apple Speech task 滚动。
- [`src/streaming.rs`](https://github.com/mingheyuan/realtime-ai-translation/blob/7361fe343d20efc68168e891ce3a18ca112aea2e/src/streaming.rs)：revision 驱动的 partial 替换与过期结果拒绝。
- [`scripts/build-macos-speech.sh`](https://github.com/mingheyuan/realtime-ai-translation/blob/7361fe343d20efc68168e891ce3a18ca112aea2e/scripts/build-macos-speech.sh)：使用 `swiftc` 构建 app bundle 并优先采用稳定本地签名身份。
- [`scripts/setup-macos-codesigning.sh`](https://github.com/mingheyuan/realtime-ai-translation/blob/7361fe343d20efc68168e891ce3a18ca112aea2e/scripts/setup-macos-codesigning.sh)：在用户钥匙串创建自签名本地开发 identity，避免 AudioHost 重建后 TCC 身份漂移。

采用：Provider/Session 生命周期、先 bind 后 launch、短 UDS 路径、app bundle TCC 身份、本地签名脚本、bounded stop、回调 generation guard。

改进：本项目增加版本握手、随机 token、心跳、独立 control/media socket、持久 AudioHost 和 app-private 短运行目录；不会直接复用无版本 JSONL 或把滚动 task 的 final 当用户最终输入。

## 2. Warashi

重点文件：

- [`tts_manager.py`](https://github.com/inni918/warashi/blob/8c71103978dbd34b8595af34eefd14c3028655f6/src/open_llm_vtuber/conversations/tts_manager.py)：TTS 并发生成、sequence buffer 与严格有序发送。
- [`conversation_utils.py`](https://github.com/inni918/warashi/blob/8c71103978dbd34b8595af34eefd14c3028655f6/src/open_llm_vtuber/conversations/conversation_utils.py)：canonical response 与字幕/翻译/语音文本分离，等待后端生成和前端播放两个完成点。
- [`conversation_handler.py`](https://github.com/inni918/warashi/blob/8c71103978dbd34b8595af34eefd14c3028655f6/src/open_llm_vtuber/conversations/conversation_handler.py)：按客户端取消活动任务和 interruption signal。
- [`single_conversation.py`](https://github.com/inni918/warashi/blob/8c71103978dbd34b8595af34eefd14c3028655f6/src/open_llm_vtuber/conversations/single_conversation.py)：每轮检索、对话结束后的后台记忆整理。
- [`memory_fts.py`](https://github.com/inni918/warashi/blob/8c71103978dbd34b8595af34eefd14c3028655f6/src/open_llm_vtuber/memory_fts.py)：SQLite FTS5 trigram、CJK 查询和结果上限。

采用：generated/displayed/spoken 分离、TTS 可并发但播放有序、取消覆盖生成器和队列、完整轮次后才整理记忆、CJK trigram 检索。

改进：本项目使用单前台 turn + generation 不变量、记忆候选需用户确认、短查询 fallback、严格 context budget 和本地 SQLite 物理删除。

## 3. three-vrm

重点文件：

- [`basic.html`](https://github.com/pixiv/three-vrm/blob/76e0109ef36f30f620e7c7883c2f893b22ca72d4/packages/three-vrm/examples/basic.html)：`GLTFLoader + VRMLoaderPlugin`、`vrm.update(delta)` 和加载后优化。
- [`expressions.html`](https://github.com/pixiv/three-vrm/blob/76e0109ef36f30f620e7c7883c2f893b22ca72d4/packages/three-vrm/examples/expressions.html)：preset expression、blink 和口型权重。
- [`animations.html`](https://github.com/pixiv/three-vrm/blob/76e0109ef36f30f620e7c7883c2f893b22ca72d4/packages/three-vrm/examples/animations.html)：动画加载与 mixer 管线。
- [`VRMHumanoid.ts`](https://github.com/pixiv/three-vrm/blob/76e0109ef36f30f620e7c7883c2f893b22ca72d4/packages/three-vrm-core/src/humanoid/VRMHumanoid.ts)：通过 `getNormalizedBoneNode` 访问标准化 head/spine 等骨骼，作为无 VRMA 时的程序化姿态入口。

采用：官方 loader/plugin、expression manager、look-at、animation mixer、加载后优化和每帧 update。

改进：业务代码只依赖 `AvatarRenderer` adapter；资源包必须带许可证 manifest，并有状态优先级、fallback、帧率和 dispose 规范。示例模型不自动复制进主仓库。

## 4. Tauri plugins workspace

重点文件：

- [`global-shortcut/src/lib.rs`](https://github.com/tauri-apps/plugins-workspace/blob/9d6d03269a3dea3e9dbf4635fe0fd68f23f750fa/plugins/global-shortcut/src/lib.rs)：快捷键注册/注销的平台主线程处理与事件入口。
- [`window-state/src/lib.rs`](https://github.com/tauri-apps/plugins-workspace/blob/9d6d03269a3dea3e9dbf4635fe0fd68f23f750fa/plugins/window-state/src/lib.rs)：窗口位置/尺寸持久化，以及恢复位置需与当前显示器相交的保护。
- [`positioner`](https://github.com/tauri-apps/plugins-workspace/tree/9d6d03269a3dea3e9dbf4635fe0fd68f23f750fa/plugins/positioner)：托盘/屏幕相关定位能力。
- [`notification`](https://github.com/tauri-apps/plugins-workspace/tree/9d6d03269a3dea3e9dbf4635fe0fd68f23f750fa/plugins/notification)：本地通知能力；未来闹钟可复用。

采用：优先使用官方插件承载快捷键、窗口恢复和通知基础能力。

改进：透明桌宠窗口的点击穿透、VRM hit-test、全屏空间行为和恢复交互入口仍由本项目显式设计，不能把 window-state 插件当完整桌宠方案。

## 5. create-tauri-app

重点文件：

- [`template-react-ts`](https://github.com/tauri-apps/create-tauri-app/tree/2b8ce19844f37aaae8fd483e5a60ac0add9c2924/templates/template-react-ts)：React 19、TypeScript 6、Vite 8 和 Tauri CLI 的当前组合。
- [`src-tauri` base template](https://github.com/tauri-apps/create-tauri-app/tree/2b8ce19844f37aaae8fd483e5a60ac0add9c2924/templates/_base_/src-tauri)：Tauri 2 config、capabilities、Rust lib/bin 和 build script 结构。

采用：以官方当前模板为工程骨架来源，并保持 `src-tauri` capability 与 config schema 可由 Tauri CLI 校验。

改进：本项目使用 workspace 目录、主面板/桌宠双窗口、透明 macOS private API、Rust 状态核心和更严格 CSP，不保留示例 greet/opener 逻辑。

## 使用规则

1. 参考仓库只用于阅读、Spike 和测试思路；主项目构建不得依赖这些绝对路径。
2. 需要复制上游代码时，先记录文件、commit、许可证和改动，再单独提交 attribution。
3. 不复制第三方角色模型、贴图、语音或其他内容资产。
4. 上游更新不会自动进入方案；升级参考 commit 后先复核差异，再修改本文和相关实现规格。
