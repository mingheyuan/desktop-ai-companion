# 爱丽丝角色资源包（本地授权资源）

这个目录是天童爱丽丝 VRM 资源的本地接入位置。仓库只保留接入说明和清单模板，不内置第三方模型、贴图、语音或游戏素材。

## 当前决策

- 目标表现：接近高质量 VRChat/角色展示模型，而不是程序化几何占位模型。
- 运行格式：VRM 0.x 或 VRM 1.0；桌面运行时使用 `three-vrm`。
- 角色来源：必须是自制、用户拥有，或获得明确授权的资源。
- 当前状态：等待已获授权的 `.vrm` 文件；不要把付费模型或受限模型复制到此目录后提交 Git。
- 当前预览：应用内置一版无外部资产的程序化角色预览，代码位于 `apps/desktop/src/avatar/arisPreview.ts`；它不是正式 VRM 资源。
- 正式制作：优先使用 Blender + [VRM Add-on for Blender](https://github.com/saturday06/VRM-Addon-for-Blender)，Unity 来源再考虑 [UniVRM](https://github.com/vrm-c/UniVRM)；运行时继续由 `three-vrm` 接入。
- 原创生成器：`scripts/character/generate_aris_vrm.py` 会根据已记录的公开视觉规格生成一个无第三方资产的可编辑 VRM 原型，并写入本地 `model/`；它用于开发和回归，不宣称是官方模型。
- CC0 底座研究：`scripts/character/build_aris_from_cc0_vroid.py` 可以在提供本地 CC0 VRM 时保留其脸部、身体和表情细节，再叠加本项目原创的爱丽丝风格发型、制服配色、光环和掌机；它是候选研究版本，不是官方模型。

## 本地目录约定

```text
resources/characters/aris/
├── README.md
├── manifest.example.json
└── model/
    └── avatar.vrm       # 本地文件，默认被 Git 忽略
```

如果本机已安装 Blender 和官方 VRM Add-on，可从仓库根目录生成本地原型：

```bash
BLENDER=/path/to/Blender \
VRM_ADDON_ROOT=/path/to/VRM-Addon-for-Blender \
BLENDER_VRM_AUTOMATIC_LICENSE_CONFIRMATION=true \
"$BLENDER" --background --python scripts/character/generate_aris_vrm.py -- \
  --out resources/characters/aris/model/aris-original.vrm \
  --blend resources/characters/aris/model/aris-original.blend \
  --render /tmp/aris-original-preview.png
```

`.vrm`、`.blend` 和 Blender 的备份文件默认不提交；桌宠仍通过“导入 VRM”把本地模型复制到应用数据目录。

## CC0 底座研究命令

[OpenGameArt 的 VRoid Studio CC0 models](https://opengameart.org/content/vroid-studio-cc0-models) 页面明确说明其中的指定样例由 VRoid 发布并以 CC0 提供。下载并核对来源后，可以运行：

```bash
BLENDER=/path/to/Blender \
VRM_ADDON_ROOT=/path/to/VRM-Addon-for-Blender \
BLENDER_VRM_AUTOMATIC_LICENSE_CONFIRMATION=true \
"$BLENDER" --background --python scripts/character/build_aris_from_cc0_vroid.py -- \
  --base /path/to/HairSample_Female.vrm \
  --out resources/characters/aris/model/aris-cc0-study.vrm \
  --blend resources/characters/aris/model/aris-cc0-study.blend \
  --render /tmp/aris-cc0-study.png
```

这个脚本不会联网下载模型，也不会把底座二进制提交到仓库；`--base` 必须由使用者自行取得并保留其来源记录。它只作为更高细节的本地候选，正式采用前仍需检查面部、动作、表情、春骨和《蔚蓝档案》二创授权范围。

应用当前通过设置页的“导入 VRM”选择文件，然后由 Rust 复制到应用数据目录：

```text
$APPDATA/characters/aris/
```

因此，开发和个人运行不要求把模型放入仓库；把 `.vrm` 放在这里主要是方便本地验收和记录资源来源。

## 进入项目验收前的最低要求

1. 模型可以在桌面近距离显示，脸部、头发、制服和主要配件具备完整材质，不出现明显缺面或纯色占位。
2. 具备 humanoid 骨骼、正常的头部和身体朝向，并能被 `three-vrm` 读取。
3. 至少支持 `blink`、`aa`、一个正向情绪和一个惊讶情绪；缺失表情时必须记录并允许降级。
4. 头发、裙摆、领带等可动部分在 `vrm.update(delta)` 下稳定，不出现持续抖动或严重穿模。
5. 在本项目透明桌宠窗口中完成 idle、listening、thinking、speaking、sleeping 五种状态检查。
6. 记录模型来源、作者、购买/授权凭证、允许的使用场景、是否允许 VRM 转换和是否允许随应用分发。

公开图片只用于整理造型规格和视觉验收，不作为可直接打包的模型或贴图来源。

详细检查项见 [`docs/character/aris-model-research.md`](../../../docs/character/aris-model-research.md) 和 [`docs/implementation/vrm-runtime.md`](../../../docs/implementation/vrm-runtime.md)。

## 注意

BOOTH 或其他平台的“可下载”不等于“可以打包进应用或公开分发”。在授权范围没有明确覆盖“本地桌面应用”和“VRM 转换”之前，只能把模型用于个人本地验证，不能提交二进制或发布安装包。
