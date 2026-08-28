# 天童爱丽丝模型调研与接入决策

> 状态：已确定接入路线，未引入第三方模型文件
>
> 调研日期：2026-08-28

## 结论

项目继续使用 VRM-only，但最终角色不再以当前程序化几何 fallback 作为视觉目标。fallback 仅保留为模型缺失、损坏或加载失败时的安全降级。

首选路线是：取得一个完成度较高的天童爱丽丝二创模型，在授权允许的范围内转换为 VRM，放入本地资源目录，通过现有“导入 VRM”流程加载。模型文件不默认进入 GitHub 仓库。

## 已实现的第一版预览

在等待正式授权 VRM 的期间，项目已经加入 `apps/desktop/src/avatar/arisPreview.ts`。它不读取或复制第三方模型、贴图和立绘，而是根据公开资料中反复出现的造型信息重新组合 Three.js 几何：

- 深色至青色渐变的长发和侧边高马尾轮廓；
- 白黑制服、青色领带、金色领饰和分层外套；
- 蓝色虹膜与高光、腮红、基础口型和眨眼表现；
- 青色/金色光环、掌机和小型科技配件；
- idle、listening、thinking、speaking、sleeping 状态下的呼吸、注视、眨眼、口型和轻微姿态变化。

这版预览的目的，是让当前 MVP 具备可检查的角色构图和交互表现；它仍然不是最终 VRM，也没有 humanoid、VRMA 或真实 spring bone 资源。获得授权的 VRM 导入后会自动隐藏该预览，不需要改动对话、记忆或语音模块。

## 候选与用途

### A. BOOTH 制服版：首选质量目标与候选资产

来源：[ヴァイスハンマー - 天童アリス（制服）](https://booth.pm/ja/items/5756040)

页面列出的信息包括：

- 面向 VRChat 制作，售价为 5,000 日元；
- 提供 UnityPackage、FBX 和贴图，不是直接可用的 VRM；
- 爱丽丝本体约 131,933 三角面，另有 Supernova 和掌机配件；
- 支持 PhysBone、口型同步、31 种表情以及服装、姿势和表情菜单。

这组规格已经达到“完整角色模型”的方向，明显高于当前 fallback，适合作为最终效果和验收标准。

授权上不能直接假设可用于本项目：页面要求遵守《蔚蓝档案》二次创作规范，禁止提取后再分发，并要求 VRChat 上传保持 Private。购买前还需要向作者确认以下范围：

1. FBX/UnityPackage 是否允许转换成 VRM；
2. 是否允许个人本地 macOS 桌面应用使用；
3. 是否允许随个人安装包分发，或只能由用户自行导入；
4. 是否允许在 GitHub 公开项目中只提交 manifest 和截图，不提交模型本体。

在这些问题得到明确答复前，资源只作为候选，不下载进仓库。

### B. ArtStation 成品：视觉质量参考

来源：[ArtStation - Blue Archive “Tendou Aris”](https://www.artstation.com/artwork/Ry654E)

这是建模和材质展示作品，适合参考脸部比例、长发体积、制服结构、配件和渲染质感，但展示页不等于可下载或可授权的运行时资源。它只作为视觉 benchmark，不作为项目依赖。

### C. 免费 VRM 配布：开发导入测试

来源：[Bilibili - 天童爱丽丝 VRM 模型配布](https://www.bilibili.com/video/BV1gN411p7MH/)

这是更接近当前运行链路的 VRM 候选，可以用于测试导入、骨骼、表情、口型和窗口适配。公开说明同时提示模型仍有穿模等问题，因此不作为最终角色；使用前仍需保留作者署名并确认当前授权条款。

### D. 明确排除的资源

- [VRoid Hub 的天童アリス模型](https://hub.vroid.com/en/characters/2274581710877816164/models/6503418241330881708)：页面标记为不可下载、不可用于其他目的，只能在 VRoid Hub 查看。
- [CGTrader BA Arisu](https://www.cgtrader.com/3d-models/character/woman/anime-character-ba-arisu)：有 OBJ、FBX、STL、BLEND 等格式和 PBR 信息，但不是 VRM；购买页的通用 royalty-free 标识不能自动覆盖《蔚蓝档案》二创模型在本桌面应用中的使用范围，除非完成单独的授权核验。
- [Good Smile Aris (Maid)](https://www.goodsmile.info/en/product/15107)：官方手办页面，可用来参考官方造型、发型和配件，但不是数字模型。

## 接入流程

```text
已授权模型
    ↓
FBX/UnityPackage → Blender/UniVRM 转换
    ↓
VRM 0.x/1.0 校验
    ↓
本地导入 → $APPDATA/characters/aris/
    ↓
three-vrm 加载 → 表情 / 口型 / look-at / spring bone / 状态动作验收
```

代码侧已经具备受控导入链路：文件选择器只接受 `.vrm`，Rust 校验容器和 VRM 扩展后复制到应用数据目录，WebView 只允许访问角色管理目录中的模型。资源包清单模板位于 [`resources/characters/aris/manifest.example.json`](../../resources/characters/aris/manifest.example.json)。

## 质量验收建议

最终模型不以面数单项判断，而以以下组合验收：

- 近景脸部和头发的材质质量；
- 表情、眨眼、口型同步和视线转动；
- 头发、裙摆、领带等 spring bone 的稳定性；
- 五种 MVP 语义状态下的动作表现；
- 透明桌宠窗口中不同尺寸下的构图；
- 首帧时间、空闲 CPU/GPU、显存和 30 分钟运行稳定性；
- 授权记录覆盖“个人本地使用、VRM 转换、安装包分发”三个维度。

直到获得满足上述条件且授权清晰的模型，项目仍保留 fallback 作为故障安全机制，但不会把它当成产品角色效果。
