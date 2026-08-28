# 代码级参考项目

这些仓库以独立浅克隆放在 `/Users/mingyuan.wang/my/` 下；仓库本身只用于阅读和验证设计，不作为 `desktop-ai-companion` 的子模块或运行时依赖。已经复制进主项目的第三方素材会单独记录来源、版本和 attribution。

## 固定参考版本

| 项目 | 本地路径 | 参考 commit | 主要用途 |
|---|---|---|---|
| [realtime-ai-translation](https://github.com/mingheyuan/realtime-ai-translation) | `/Users/mingyuan.wang/my/realtime-ai-translation` | [`7361fe3`](https://github.com/mingheyuan/realtime-ai-translation/commit/7361fe343d20efc68168e891ce3a18ca112aea2e) | Rust ASR Provider/Session、Swift Speech bridge、取消代次 |
| [Warashi](https://github.com/inni918/warashi) | `/Users/mingyuan.wang/my/参考/warashi` | [`8c71103`](https://github.com/inni918/warashi/commit/8c71103978dbd34b8595af34eefd14c3028655f6) | 对话中断、TTS 有序队列、记忆检索与完成后整理 |
| [three-vrm](https://github.com/pixiv/three-vrm) | `/Users/mingyuan.wang/my/参考/three-vrm` | [`76e0109`](https://github.com/pixiv/three-vrm/commit/76e0109ef36f30f620e7c7883c2f893b22ca72d4) | VRM 加载、表情、look-at、动画与资源优化 |
| [Tauri plugins workspace](https://github.com/tauri-apps/plugins-workspace) | `/Users/mingyuan.wang/my/参考/tauri-plugins` | [`9d6d032`](https://github.com/tauri-apps/plugins-workspace/commit/9d6d03269a3dea3e9dbf4635fe0fd68f23f750fa) | 全局快捷键、窗口状态、通知和平台线程约束 |
| [create-tauri-app](https://github.com/tauri-apps/create-tauri-app) | `/Users/mingyuan.wang/my/参考/create-tauri-app` | [`2b8ce19`](https://github.com/tauri-apps/create-tauri-app/commit/2b8ce19844f37aaae8fd483e5a60ac0add9c2924) | 当前 Tauri 2 + React/TypeScript 工程模板与配置字段 |
| [DesktopPetLive2D](https://github.com/Yanchen-J/DesktopPetLive2D) | `/Users/mingyuan.wang/my/参考/desktop-pet-live2d` | [`dc4f701`](https://github.com/Yanchen-J/DesktopPetLive2D/commit/dc4f70111177f760ddf6027b7aaba6ca7417cd3a) | Live2D 桌宠窗口、角色目录、角色配置、双层记忆 |
| [Live2DPet-modern](https://github.com/MARS-HX/Live2DPet-modern) | `/Users/mingyuan.wang/my/参考/live2dpet-modern` | [`2592a65`](https://github.com/MARS-HX/Live2DPet-modern/commit/2592a658478433a0c892399d277462334146b9b6) | Live2D 适配器、参数映射、情绪/表情/动作、视觉记忆和语音入口 |
| [Komorebi](https://github.com/kiskaserver/interactive_assistent) | `/Users/mingyuan.wang/my/参考/komorebi` | [`a2df4dc`](https://github.com/kiskaserver/interactive_assistent/commit/a2df4dcc68ff4a03d7701aa02f49354fde66e7b6) | Tauri 2 + Live2D、屏幕视觉、语音、工具权限和失败降级 |
| [FiguraBlueArchiveCrafters](https://github.com/Gakuto1112/FiguraBlueArchiveCrafters) | `/Users/mingyuan.wang/my/参考/FiguraBlueArchiveCrafters` | [`3cd48d7`](https://github.com/Gakuto1112/FiguraBlueArchiveCrafters/commit/3cd48d7) | Minecraft Figura/Blockbench 方块化角色、分层长发、光环、武器和动画组织 |

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

## 6. Live2D 桌宠参考

### DesktopPetLive2D

重点文件：

- `main/models.js`：扫描 `models/<id>/`，读取 `pet.config.json`，把模型路径和角色设定组合成运行时配置。
- `main/memory.js`：将跨角色共享记忆与当前角色专属记忆分开保存，并在注入 prompt 前合并。
- `renderer/`：透明置顶 Electron 窗口、拖动、点击和 Live2D canvas 的组织方式。

采用：角色资源目录契约、角色配置与模型文件分离、共享/专属记忆的概念，以及“第三方模型不进 Git”的资源边界。

改进：本项目不复制其 Electron 主进程或 JSON 文件存储；业务事实仍由 Rust + SQLite 保存，Live2D 只实现 `AvatarRenderer`，不直接拥有记忆和工具权限。

### Live2DPet-modern

重点文件：

- `src/renderer/model-adapter.js`：用 adapter 统一 Live2D、静态图片和空渲染器，调用方不依赖具体模型 SDK。
- `src/core/emotion-system.js`：把情绪积累、AI 选择和表情/动作播放解耦，并支持按语音时长对齐。
- `src/core/enhance/`：视觉信息、知识池和记忆增强的拆分方式。

采用：模型适配器、模型参数 introspection、表达层与业务状态解耦、表情/动作时长和 TTS 对齐。

改进：本项目将情绪和表现状态收敛到已有 `BehaviorScheduler`；不会让每个模块直接抢占 Live2D 参数，也不会把屏幕截图自动写进长期记忆。

### Komorebi

重点文件：

- `src/components/Live2DCanvas/`：PIXI 场景挂载、模型加载、Cubism 2/3/4/5 兼容、表达式/动作探测和加载失败降级。
- `src/hooks/useListenController.ts`、`src/vad.ts`：持续监听、VAD 和语音状态与 UI 的连接方式。
- `src-tauri/src/commands/vision.rs`、`src-tauri/src/tools.rs`：屏幕观察、工具调用和权限边界。
- `public/live2d/README.md`：模型目录契约和 Cubism Core 不随主项目默认分发的说明。

采用：`Live2DCanvas` 独立渲染边界、模型加载失败不拖垮主界面、模型 URL 与布局参数分离、视觉/语音/工具各自经过 Rust 能力边界。

改进：当前项目保留 Swift AudioHost 和 Rust `ToolRuntime` 的既有设计；只借鉴组件边界，不复制其 provider、配置或自动化实现。

### 许可证与资源边界

以上三个仓库的代码许可证和其中的角色资源许可证不是同一件事。Live2D Cubism Core 是 Live2D 的专有运行时，角色模型还可能有单独的作者条款。当前方案因此规定：

1. 参考仓库只放在本机独立目录，不作为主项目依赖或子模块。
2. 公开仓库中的代码如需改写，先保留 commit、文件路径、许可证和 attribution 记录。
3. 爱丽丝 Live2D 模型只允许用户自行取得并放入本地未跟踪目录；未经许可不修改、不上传、不随安装包分发。
4. 在 Live2D Spike 通过前，主项目默认渲染后端仍是 VRM；不同时把两个 SDK 和两套角色资产塞进 MVP。

详细的候选运行时契约、Spike 验收项和当前未决选择见 [`implementation/live2d-runtime.md`](implementation/live2d-runtime.md)。

## 7. Minecraft 方块化角色参考

### FiguraBlueArchiveCrafters

重点文件：

- `src/avatars/13a_Aris/models/main.bbmodel`：普通服爱丽丝的 Blockbench 模型，包含头部、身体、四肢、前后发、侧马尾、光环和服装标识等分层结构。
- `src/avatars/13a_Aris/models/gun.bbmodel`：Supernova/轨道炮的独立模型。
- `src/avatars/13a_Aris/models/ex_skill_1.bbmodel`、`scripts/rail_gun.lua`：技能动画、蓄力和武器表现的拆分方式。
- `src/avatars/13a_Aris/scripts/blue_archive_character.lua`：角色配置、模型列表、语音和动作入口。

采用：直接接入经用户授权用于个人学习的 `13a_Aris/models/main.bbmodel`，通过 `arisBbmodelPreview.ts` 解析 Blockbench 的 outliner、分层几何和内嵌像素贴图；同时保留 `arisBlockPreview.ts` 作为无资源时的程序化降级预览，并借鉴上游的模型结构/行为脚本分离思路。

改进：当前桌宠的 `avatar=block` 预览已使用复制到本项目的普通服模型文件，但没有复制上游 Lua 逻辑、语音或其他版本资源；资源来源、署名和非商业学习用途说明见 [`THIRD_PARTY_NOTICES.md`](../THIRD_PARTY_NOTICES.md)。

授权边界：上游 LICENSE 将作者原创程序/数据结构与 Blue Archive 的角色设计、视觉标识区分开，并要求遵守官方二次创作规则；当前只纳入普通服模型作为个人学习预览素材，具体边界见 `THIRD_PARTY_NOTICES.md`。

## 使用规则

1. 参考仓库只用于阅读、Spike 和测试思路；主项目构建不得依赖这些绝对路径。
2. 需要复制上游代码或内容资产时，先记录文件、commit、许可证和改动，再单独提交 attribution。
3. 未经单独复核，不复制第三方语音、Lua 逻辑或其他内容资产；已纳入的模型素材必须遵守 `THIRD_PARTY_NOTICES.md` 中的边界。
4. 上游更新不会自动进入方案；升级参考 commit 后先复核差异，再修改本文和相关实现规格。
