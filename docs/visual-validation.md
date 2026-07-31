# UI 视觉验收

`ui-snapshots` 使用与原生窗口相同的 `GalleryState::view`、主题和 Iced WGPU
renderer，在离屏纹理生成 PNG。它不是重新制作的静态 mock。
原生 Gallery 启动任务与离屏 Renderer 都使用 `UI_BASE_TEXT_SIZE` 的 13px 基准，
因此未显式覆盖字号的标准正文不会在真实窗口中回退到 Iced 的 16px 默认值。

当前输出：

- `titlebar-custom-dark.png`、`titlebar-custom-light.png`；
- `titlebar-native-leading-dark.png`；
- `gallery-controls-dark.png`、`gallery-controls-light.png`；
- `gallery-loading-dark.png`；
- `gallery-surfaces-dark.png`、`gallery-surfaces-light.png`；
- `gallery-cards-dark.png`、`gallery-cards-light.png`；
- `gallery-feedback-dark.png`；
- `gallery-context-menu-dark.png`、`gallery-dialog-dark.png`；
- `gallery-workspace-dark.png`、`gallery-sidebar-collapsed-dark.png`；
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
- 设置使用独立 WorkspaceController，主题、标准圆角、主区域圆角和尺寸复位均即时
  更新真实状态；
- 侧栏折叠后主区域贴边圆角正确，dark/light 的文字、语义色和边框均可辨；
- Noto Sans SC 四个字重、标题 tracking、分区标题、滚动条和固定 Footer 正常。
- Controls 的中号按钮、输入、选择与列表主标签共享 13px 基准，辅助文字仍保持
  10–12px 的语义层级，不出现真实窗口独有的 16px 放大。

首次实现快照时，普通 Text 未显示而 Canvas/TextArea 正常。根因是 snapshot 工具
在 GPU 读取前丢弃 `UserInterface::Cache`，使普通文本上传缓存的弱引用失效。当前
实现让 Cache 保持到 screenshot 完成，并在 draw 前发送真实
`RedrawRequested`，因此快照可用于状态和颜色验收。

运行命令：

```bash
cargo run --release -p nana-ui --example ui-snapshots --locked
```

截图工具会执行 GPU→CPU 回读和 PNG 编码，只属于验收工具。正式窗口 Gallery 与
宿主纹理直显不会复用该读取逻辑。离屏快照不能证明原生窗口材质、鼠标命中、IME、
真实窗口 resize、不同 DPI 或 Windows/Linux 最终栅格；这些仍需在对应平台补测。
