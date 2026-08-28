# VRM 角色运行时与桌宠窗口

## 1. 范围

MVP 只支持 VRM。Live2D 需要不同渲染器、动作系统、资源格式和授权链，不与 VRM 同期引入。角色表现层不得持有对话、记忆或 Provider 状态，只消费 Rust Core 的统一快照和短时表现事件。

初始角色概念可参考《蔚蓝档案》天童爱丽丝的性格方向，但公开仓库不内置未经授权的角色模型、贴图、语音、台词合集、Logo 或游戏素材。开发阶段使用自制/明确授权的占位 VRM；最终角色包必须单独完成授权确认。

## 2. 渲染适配层

React 侧通过稳定接口封装 `three`/`@pixiv/three-vrm`：

```ts
interface AvatarRenderer {
  load(manifest: AvatarManifest): Promise<void>;
  setSemanticState(state: AvatarSemanticState): void;
  setExpression(name: string, weight: number): void;
  setLipLevel(level: number): void;
  setLookTarget(target: { x: number; y: number } | null): void;
  playAction(action: string, options?: { loop?: boolean; priority?: number }): void;
  hitTest(x: number, y: number): AvatarHit[];
  resize(width: number, height: number, dpr: number): void;
  dispose(): void;
}
```

具体实现使用：

- `GLTFLoader + VRMLoaderPlugin` 加载模型；
- `vrm.update(delta)` 更新约束、spring bone 与 look-at；
- `VRMExpressionManager` 驱动 blink、口型和情绪；
- `AnimationMixer`/VRM Animation 驱动 idle/listen/think/speak/action；
- 加载后执行许可的优化，如移除不必要顶点、合并 skeleton/morph；
- 对角色 skinned mesh 设置适当的 frustum 处理，避免透明小窗边缘错误裁剪。

这些做法来自 three-vrm 官方 examples，但项目内部只能依赖上述 adapter，不能让业务组件直接调用 three-vrm 对象。

## 3. 角色包 manifest

```json
{
  "schema_version": 1,
  "id": "dev-placeholder",
  "display_name": "Development Placeholder",
  "model": "model/avatar.vrm",
  "license": {
    "name": "...",
    "source": "...",
    "redistribution_allowed": true,
    "attribution": "..."
  },
  "camera": { "target_height": 1.25, "distance": 2.2 },
  "scale": 1.0,
  "expressions": {
    "neutral": "neutral",
    "happy": "happy",
    "sad": "sad",
    "surprised": "surprised",
    "blink": "blink",
    "mouth_open": "aa"
  },
  "animations": {
    "idle": "animations/idle.vrma",
    "listening": "animations/listening.vrma",
    "thinking": "animations/thinking.vrma",
    "speaking": "animations/speaking.vrma",
    "sleeping": "animations/sleeping.vrma"
  },
  "hit_areas": ["head", "body"]
}
```

启动校验 model/animation 路径必须位于角色包内，拒绝 `..`、远程 URL 和未知 schema。`redistribution_allowed=false` 的资源不可进入安装包或 Git。

## 4. 语义状态到表现层

```text
AvatarSemanticState =
  idle | listening | thinking | speaking | sleeping |
  needs_input | ready | recovering | error
```

优先级从高到低：

1. 一次性用户动作反馈（点击、拖拽结束）；
2. error/needs_input/recovering；
3. speaking/listening/thinking；
4. ready/sleeping/idle 基础循环；
5. additive look-at、blink、breathing、lip-sync。

- 高优先级动画结束后回到当前语义状态，而不是固定回 idle。
- 状态切换使用 150–300ms cross-fade；取消旧 turn 会立即清除 speaking lip level。
- blink 由随机间隔本地生成；错误/思考等表情只调节权重，不写业务状态。
- 没有对应动作时回退到 idle + 表情，不让资源缺失破坏对话。

## 5. 口型与注视

- MVP 使用 AudioHost 回传的归一化振幅驱动 VRM preset expression `aa`。
- 输入先做 attack 约 40ms、release 约 120ms 的包络平滑，低于噪声阈值归零。
- UI 超过 250ms 未收到 level 时自动衰减到 0；`chunk_finished` 立即归零。
- 后续可加入音素/viseme，但不改变 `setLipLevel` 之外的业务接口。
- look-at 默认跟随窗口内光标并限制角度；用户离开窗口后缓慢回正。禁止默认启用摄像头注视追踪。

## 6. 点击、拖拽与点击穿透

桌宠透明无边框窗口分为两种模式：

- **交互模式**：整个窗口接收鼠标；角色 hit area 决定点击动作，窗口边界内的透明区域也可能拦截点击。
- **点击穿透模式**：整个窗口忽略鼠标；必须保留全局快捷键和托盘菜单用于恢复交互。

MVP 不承诺逐像素透明区域穿透。该能力需要 macOS 原生命中测试或高频切换窗口 ignore-mouse 状态，先作为透明窗口 Spike 的独立加分项，不能成为基础拖拽与恢复入口的前置条件。

角色拖拽只改变窗口位置，不移动 3D 模型世界坐标。结束时把位置交给窗口状态服务保存；恢复时复用 Tauri window-state 的“必须与现有显示器相交”原则，若显示器已移除则回到主屏安全边界。

### 已实现的窗口控制基线

- Rust `PetSettings` 是可见性、点击穿透、缩放和修订号的事实来源，使用 SQLite `app_settings` 的 `desktop.pet` 键持久化。
- 缩放通过 Tauri 物理窗口尺寸实现，允许范围为 60%–150%；React 只提交带 `expected_revision` 的更新意图。
- 点击穿透使用 Tauri `set_ignore_cursor_events`，主面板和托盘都保留恢复入口。
- 系统托盘提供打开主面板、显示/隐藏桌宠、切换点击穿透和退出；主窗口关闭按钮默认隐藏而不是终止后台进程。
- 全局 `CommandOrControl+Shift+Space` 切换主面板显示，确保桌宠隐藏或穿透时仍能恢复控制。

命中测试只在 pointer down/move 的受控频率运行。点击动作由 `AvatarHit(area)` 映射到本地表现事件；它不能直接写记忆或拼接 LLM system prompt。

## 7. 渲染与电量预算

- 正在动画/交互时目标 60fps；纯 idle 可降到 30fps；长期静止且窗口不活跃降到 15fps。
- 窗口隐藏、屏幕锁定或页面不可见时暂停 render loop 和 spring bone 更新。
- DPR 设置上限（默认 2）；窗口缩放时延迟重建 render target。
- 角色包加载失败必须释放已创建的 geometry/material/texture/mixer，显示轻量 fallback，而不是白屏。
- 同时只加载一个 VRM；换装/多角色不进入 MVP。
- Spike 记录 idle/active CPU、GPU、内存、首帧时间和 30 分钟资源增长，指标值在占位模型确定后冻结。

## 8. 安全与测试

- WebView 禁止角色包执行脚本；manifest 只描述资源和数值映射。
- CSP 禁止任意远程模型；用户导入后复制到应用管理目录并校验扩展名、大小和 manifest。
- 许可证与来源在设置页可查看。
- 测试覆盖模型缺失、表达式缺失、动作缺失、损坏 VRM、超大纹理、快速 show/hide、显示器拔插、点击穿透恢复和取消时口型归零。
