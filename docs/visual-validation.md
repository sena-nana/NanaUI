# UI 视觉验收

`ui-snapshots` 使用与原生窗口相同的 `WorkspaceState::view`、`GalleryState::view`、主题和 Iced WGPU renderer，在离屏纹理生成 PNG。它不是重新制作的静态 mock。

当前输出：

- `workspace-dark.png`、`workspace-light.png`；
- `workspace-github-dark.png`、`workspace-live2d-dark.png`；
- `component-gallery-controls-dark.png`、`component-gallery-controls-light.png`；
- `component-gallery-loading-dark.png`；
- `component-gallery-surfaces-dark.png`；
- `component-gallery-surface-cards-dark.png`；
- `component-gallery-context-menu-dark.png`；
- `component-gallery-dialog-dark.png`。

本机 2026-07-30 在 WGPU 30 上重新生成并检查：

- 36px 标题栏、左侧 Code/Github/Live2D 切换、居中动态标题和 28px 真实操作按钮完整；
- Code 的 global/section/resources、Github 的 global/resources/Pull Requests、Live2D 的 resources 起始区顺序正确；
- Github 不注册底部区，Live2D 使用 timeline，Code 使用 console；主内容与 inspector 边界完整；
- workspace region 由公共 `WorkspaceRegions` / `workspace_view` 动态组合；8px resize 命中区在空闲态不形成粗分隔线；
- 节点画布网格、节点、连接线和裁剪区域正确；
- dark/light 的文字、语义色、边框与 LiliaUI 五档选中状态均可辨；
- TextInput invalid、TextArea、Checkbox、Switch、Slider、Scrollable/ListItem 正常；
- Controls 顶部三张卡片等宽同高，下方两张卡片等宽同底，dark/light 边线一致；
- Surface/Card 的基础、抬升、选中层级清晰；
- 普通卡片与按钮型卡片边缘一致，内部文字保持左对齐并在纵向居中；
- 文本按钮、图标按钮和 loading 的“图标 + 文本”内容均在按钮内双轴居中；
- Feedback 状态卡与操作卡同顶同底，内部两个全宽操作按钮居中；
- Context Menu 未被父容器裁剪，危险操作具有独立颜色；
- Dialog scrim、Header/Body/Footer 和按钮层级正确。

首次实现快照时，普通 Text 未显示而 Canvas/TextArea 正常。根因是 snapshot 工具在 GPU 读取前丢弃 `UserInterface::Cache`，使普通文本上传缓存的弱引用失效。当前实现让 Cache 保持到 screenshot 完成，从资源生命周期根因修复。

运行命令：

```bash
cargo run --release -p nana-ui --example ui-snapshots --locked
```

截图工具会执行 GPU→CPU 回读和 PNG 编码，只属于验收工具。正式窗口 Demo 与宿主纹理直显不会复用该读取逻辑。

离屏快照可以证明布局和像素内容，但不能证明原生窗口材质、鼠标命中、IME、窗口 resize 或不同 DPI 显示器上的最终表现；这些仍需在解锁的真实窗口和目标平台补测。
