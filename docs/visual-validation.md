# UI 视觉验收

`component-gallery` crate 中的 `ui-snapshots` 驱动同一套 `GalleryState` 与
Runtime 文档（`flush` + `document.scene()`），再用 `SceneWgpuPainter` 把
`UiScene` 画进宿主离屏 WGPU 纹理后 CPU 回读 PNG。它不是重新制作的静态 mock，
也不再以 Iced `UserInterface` / `IcedSceneView` 作为快照宿主。
原生 Gallery 与 Scene 文本路径都使用 `UI_BASE_TEXT_SIZE` 的 13px 基准，
因此未显式覆盖字号的标准正文不会回退到 Iced 的 16px 默认值。

当前输出：

- `titlebar-custom-dark.png`、`titlebar-custom-light.png`；
- `titlebar-native-leading-dark.png`；
- `gallery-controls-dark.png`、`gallery-controls-light.png`；
- `gallery-loading-dark.png`；
- `gallery-surfaces-dark.png`、`gallery-surfaces-light.png`；
- `gallery-cards-dark.png`、`gallery-cards-light.png`；
- `gallery-feedback-dark.png`；
- `gallery-context-menu-dark.png`、`gallery-dialog-dark.png`；
- `gallery-workspace-dark.png`、`gallery-workspace-dock-preview-dark.png`、
  `gallery-workspace-dock-preview-light.png`、`gallery-sidebar-collapsed-dark.png`；
- `dock-preview-retarget-tab-dark.png`、`dock-preview-retarget-tab-light.png`；
- `dock-hover-left-dark.png`、`dock-hover-left-light.png`；
- `gallery-settings-appearance-dark.png`、`gallery-settings-appearance-light.png`；
- `gallery-settings-workspace-dark.png`。

验收重点：

- 36px 标题栏保持几何居中，侧栏、主题和窗口按钮均连接真实状态；
- Gallery 仅使用一个 220px 左侧栏，普通分类与设置分类在同一 Region 互斥切换；
- 控件、表面、反馈和工作区分类使用同一 Gallery 状态，不出现消费应用业务语义；
- 控件覆盖按钮、输入校验、TextArea、Checkbox、Switch、Slider、Scrollable、
  ListItem、loading 与 disabled 状态；
- 原生 Gallery 中的输入框、TextArea、下拉、搜索下拉与 XYPad 在 focused /
  opened 状态只使用中性浅边框且不改变背景，invalid 状态仍保持危险色边框；
- 表面覆盖基础、抬升、选中以及普通、交互、禁用卡片；选中卡片在深浅主题下均
  使用柔和状态背景与浅边框，不使用蓝色强调描边；
- Feedback 覆盖进度、Tooltip、Context Menu 的危险确认和 Dialog 关闭策略；
- 工作区分类真实启用 Toolbar、Inspector 与 Bottom，支持折叠、resize、双击复位，
  离开分类后隐藏辅助 Region，重新进入时保留尺寸和折叠状态；
- Dock 拖拽预览沿用真实布局树：超过拖动阈值后候选目标使用低强度 `accent_soft` 背景和 `accent_on_soft` 细边框提示，停留 80ms 后直接显示最终结构空位；源面板从原位置消失，Tab 插入显示待插入标签并激活空位，深浅主题均不使用饱和遮罩；
- 设置使用独立 WorkspaceController，主题、标准圆角、主区域圆角和尺寸复位均即时
  更新真实状态；
- 侧栏折叠后主区域贴边圆角正确，dark/light 的文字、语义色和边框均可辨；
- Noto Sans SC 四个字重、标题 tracking、分区标题、滚动条和固定 Footer 正常。
- Controls 的中号按钮、输入、选择与列表主标签共享 13px 基准，辅助文字仍保持
  10–12px 的语义层级，不出现真实窗口独有的 16px 放大。

快照工具把 `UiScene` 画到 `Bgra8UnormSrgb` 离屏纹理（`RENDER_ATTACHMENT |
COPY_SRC`），再 `copy_texture_to_buffer` 回读。`UserInterface::Cache` 只属于
已退役的 Iced 快照宿主，不再参与这条路径。

运行命令：

```bash
cargo run --release -p component-gallery --bin ui-snapshots \
  --features snapshots --locked
```

截图工具会执行 GPU→CPU 回读和 PNG 编码，只属于验收工具。正式窗口 Gallery 与
宿主纹理直显不会复用该读取逻辑。离屏快照不能证明原生窗口材质、鼠标命中、IME、
真实窗口 resize、不同 DPI 或 Windows/Linux 最终栅格；这些仍需在对应平台补测。

## 2026-08-18（macOS / Metal）

`ui-snapshots` 通过。相对 2026-08-17 18:47 的顶层 64 张 PNG，61 张字节一致；变化的 3 张是空输出目录下 runtime 像素写入 `migration-first-batch-iced-dark.png`，difference 全黑。Gallery / Scene / Workspace 产品图无回归。PNG 不在 git。

`hosted-gpu-demo --measure-first-frame`（`hosted,bundled-fonts`）三次真实 Surface present：冷 699.980 ms，随后约 225 ms。同一 Device/Queue，正式路径无 `copy_texture_to_buffer`。探针读 host 实际 `MaterialOutcome`（macOS Scene GPU 当前为 solid，非 Vibrancy）；三次 present 不是 Vibrancy 验收。Windows/Linux 未测。
