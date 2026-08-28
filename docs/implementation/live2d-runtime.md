# Live2D 运行时候选方案

> 状态：候选 Spike，尚未替换当前 VRM 运行时
>
> 更新日期：2026-08-29

本文记录从 `DesktopPetLive2D`、`Live2DPet-modern` 和 `Komorebi` 提炼出的 Live2D 接入边界。它的目的不是现在就切换技术，而是让后续 Spike 有明确输入、输出和停止条件，避免 VRM 与 Live2D 的实现细节互相污染。

## 1. 为什么保留 Live2D 候选

当前程序化 VRM fallback 的脸部、头发和制服仍然只是视觉占位；如果能合法取得完整的 Live2D Cubism 角色，2D 模型通常已经包含绘制层、参数、表情、动作和物理配置，更适合先验证“好看的桌宠 + 对话 + 语音”闭环。

Live2D 的代价是它不是 3D 模型：角色主要在二维绘制空间中变形，不能像 VRM 一样自由旋转、从背面观察或使用 humanoid/VRMA 动作。它还依赖 Live2D 专有的 Cubism Core 运行时，以及每个模型自己的授权条款。

因此当前决策是：

- `VRM` 仍是当前默认和已接入的渲染后端；
- `Live2D` 作为可替换的 `AvatarRenderer` 候选；
- MVP 不同时内置两套第三方角色资源；
- 只有在本地模型加载、语音口型、交互、许可证和包体成本都通过 Spike 后，才决定是否把 Live2D 设为默认。

## 2. 资源契约

Live2D 模型不是一个单文件。Cubism 4/5 的典型目录至少包含：

```text
aris/
├── aris.model3.json
├── aris.moc3
├── aris.2048/texture_00.png
├── motions/*.motion3.json
├── expressions/*.exp3.json
├── aris.physics3.json
└── manifest.json
```

`manifest.json` 只记录本地来源、作者、模型版本、允许的使用方式和是否允许修改/分发；它不能替代作者许可证。模型目录放在应用数据目录或本地未跟踪资源目录，不能因为主项目是 MIT 就自动变成 MIT。

当前爱丽丝候选 [BOOTH Live2D 天童アリス](https://booth.pm/ja/items/7367630) 页面显示为免费 VTS 模型，但同时明确禁止二次修改和再分发。因此它最多用于用户自行下载后的本机加载测试，不得提交 GitHub、不得放进公开安装包，也不能为了适配项目而重打包模型。

## 3. 渲染边界

业务层只依赖下列语义接口，不直接引用 PIXI、Cubism Core 或某个模型的参数名：

```text
AvatarRenderer
  load(source) -> ready | failed
  resize(layout)
  setLookAt(x, y)
  setExpression(expression_id)
  playMotion(group, index, priority)
  setMouthOpen(value)
  setVisibility(visible)
  update(delta_seconds)
  dispose()
```

调用关系固定为：

```text
InteractionSupervisor
  -> BehaviorScheduler
  -> AvatarPresentationState
  -> AvatarRenderer adapter
  -> VRM renderer 或 Live2D renderer
```

LLM、记忆、语音和工具模块不能直接调用 `setExpression` 或修改 Live2D 参数。它们只产生语义事件，例如 `listening`、`thinking`、`speaking`、`happy`、`surprised`；`BehaviorScheduler` 决定具体表达式、动作和优先级。

## 4. Live2D 加载流程

```text
用户导入模型目录或选择 model3.json
  -> 校验路径只在角色资源目录内
  -> 解析 model3.json / model.json
  -> 初始化 PIXI canvas 和 Cubism runtime
  -> 加载 moc、纹理、physics、motion、expression
  -> 读取实际可用的参数、表情和动作名称
  -> 生成 AvatarRenderer capability snapshot
  -> BehaviorScheduler 按 capability 降级
```

借鉴 `Komorebi` 的失败策略：Cubism Core 缺失、模型文件损坏、表达式不存在或运行时初始化失败时，只让角色渲染层进入 `failed`，主面板、文本对话、记忆和工具仍然可用。界面显示 placeholder 或当前安全 fallback，不把渲染错误升级成应用崩溃。

借鉴 `Live2DPet-modern` 的参数策略：加载完成后通过模型自身声明或运行时 introspection 建立参数映射；不要假设每个模型都有相同的 `ParamAngleX`、`ParamMouthOpenY` 或表情文件名。口型和视线能力必须由 capability snapshot 声明，缺失时回退为文本/基础 idle。

## 5. 与语音和状态机的关系

Live2D 只接收已经确定的表现状态：

| 语义状态 | Live2D 表现 | 触发来源 |
|---|---|---|
| `idle` | 默认 idle motion、眨眼、轻微呼吸 | `BehaviorScheduler` |
| `listening` | 视线跟随、倾听表情或短动作 | AudioWorker / InputState |
| `thinking` | 思考表情或循环动作 | `TurnActor` |
| `speaking` | 口型参数 + TTS 音频振幅，必要时播放动作 | PlaybackState |
| `sleeping` | 睡眠表情/动作，关闭主动监听 | QuietHours / BehaviorScheduler |
| `error` | 安全表情或静态 placeholder | 统一错误状态 |

`Live2DPet-modern` 的“按语音时长对齐表达式”可以借鉴，但音频事实仍由当前项目的 `AudioWorker` / AudioHost 提供。渲染层不能自己创建 TTS、监听麦克风或重试语音请求。

## 6. 与记忆、视觉和工具的关系

借鉴 `DesktopPetLive2D` 的角色配置概念，但在本项目中拆成三层：

1. `CharacterManifest`：角色名称、模型来源、版权边界和可用表现能力。
2. `PersonaProfile`：天童爱丽丝的初始性格、称呼、语气和对话约束。
3. `MemoryRepository`：用户事实、角色关注点、确认/拒绝状态和检索索引。

Live2D 模型文件不能携带或覆盖核心 persona，也不能直接写入长期记忆。屏幕截图同样只通过 `ScreenObserver -> VisionProvider -> ToolResult` 进入当前轮次；除非用户明确要求记住，否则不进入模型资源或长期记忆。

## 7. Cubism Core 与分发

Cubism Core 属于 Live2D 专有运行时，不能因为应用代码是 MIT 就随意复制、修改或再分发。Spike 至少验证以下两种运行模式：

- 用户按官方条款自行提供本地 `live2dcubismcore.min.js`；
- 用户明确允许联网时，从官方来源加载 Core。

如果未来要发布安装包，必须单独完成 Live2D SDK、Cubism Core、模型、纹理、动作和语音资源的授权清单；在此之前，主仓库只提交适配器代码、schema、manifest 示例和不含第三方资源的测试 fixture。

## 8. Spike 验收门槛

不改业务架构，只做一个独立的 Live2D renderer spike，满足以下条件才进入技术选型：

1. macOS 上能从本地角色目录加载 Cubism 3/4 模型，并能安全销毁和重新加载。
2. 能执行 idle、眨眼、视线跟随、至少一个表情和一个动作。
3. 能用 AudioHost 已有振幅事件驱动口型，不重复实现播放链路。
4. 模型目录、参数、表达式缺失时能 graceful fallback。
5. 桌宠窗口仍支持拖动、点击、缩放、透明和置顶；模型不遮挡控制入口。
6. 模型资源、Cubism Core 和主项目 MIT 代码的许可证边界可以分别记录。
7. 运行 30 分钟无明显内存增长；切换模型、关闭窗口和退出应用不残留 PIXI ticker、纹理或事件监听。

Spike 通过后再比较 VRM 与 Live2D 的默认路线；Spike 未通过前不把当前 `VrmCanvas` 改成双引擎混合，也不把未经授权的爱丽丝模型提交到仓库。
