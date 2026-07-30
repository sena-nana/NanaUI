# UI 视觉验收

`ui-snapshots` 使用与原生窗口相同的 `WorkspaceState::view`、`GalleryState::view`、主题和 Iced WGPU renderer，在离屏纹理生成 PNG。它不是重新制作的静态 mock。

当前输出：

- `workspace-dark.png`、`workspace-light.png`；
- `workspace-custom-radius-dark.png`；
- `workspace-lilia-viewport-dark.png`；
- `workspace-github-dark.png`、`workspace-live2d-dark.png`；
- `workspace-nodes-dark.png`、`workspace-search-dark.png`、`workspace-preview-dark.png`；
- `workspace-settings-appearance-dark.png`、`workspace-settings-appearance-light.png`；
- `workspace-settings-lilia-viewport-dark.png`；
- `workspace-settings-layout-dark.png`、`workspace-settings-about-dark.png`；
- `component-gallery-controls-dark.png`、`component-gallery-controls-light.png`；
- `component-gallery-loading-dark.png`；
- `component-gallery-surfaces-dark.png`；
- `component-gallery-surface-cards-dark.png`；
- `component-gallery-context-menu-dark.png`；
- `component-gallery-dialog-dark.png`。

本机 2026-07-30 在 WGPU 30 上重新生成并检查：

- 36px 标题栏、左侧 Code/Github/Live2D 切换、居中动态标题和 28px 真实操作按钮完整；
- Code、Github 与 Live2D 均只有一个 220px 左侧栏；普通列表与设置分类在同一位置互斥切换；
- 普通导航、设置返回与设置分类复用同一 `SidebarRow`，行高、圆角、内距和状态层级一致；
- Github 不注册底部区，Live2D 使用 timeline，Code 使用 console；主内容与 inspector 边界完整；
- 节点列表、搜索结果、预览页与关于页均使用真实状态单独渲染，未以默认页代替覆盖；
- 通用侧栏的分区、28px 左对齐单行导航、同一 28px 图标中心线、独立滚动主体和固定 Footer 边界正确；
- Footer 设置入口为 26×26 低强调图标按钮，点击进入真实设置状态；
- 设置侧栏、返回操作、分类选中、外观与工作区设置卡在 dark/light 下对齐；
- 外观页的标准圆角为真实滑杆，内部派生四档圆角；自定义圆角快照以悬停分区
  标题和选中列表项确认两者共享完整宽度、28px 高度和运行时控件圆角；
- 1280×720 工作区的 220/780/280 列与 34/450/200 行逐像素对齐；
- workspace region 由公共 `WorkspaceRegions` / `workspace_view` 动态组合；8px resize 命中区覆盖边缘而不消耗 grid track；
- Noto Sans SC 四个字重、标题 0.2px tracking 和分区标题 0.5px tracking 已由同源字体数据渲染；
- 节点画布网格、节点、连接线和裁剪区域正确；
- dark/light 的文字、语义色、边框与 LiliaUI 五档选中状态均可辨；
- TextInput invalid、TextArea、Checkbox、Switch、Slider、Scrollable/ListItem 正常；
- Controls 顶部三张卡片等宽同高，下方两张卡片等宽同底，dark/light 边线一致；
- Surface/Card 的基础、抬升、选中层级清晰；
- 普通卡片与按钮型卡片边缘一致，内部文字保持左对齐并在纵向居中；
- 文本按钮、图标按钮和 loading 的“图标 + 文本”内容均在按钮内双轴居中；
- 主题、添加、关闭、信息和展开箭头使用 24×24 矢量坐标，状态圆点使用居中几何，不依赖字体基线；
- Inspector 标题、滚动控件和右侧边缘统一使用 12px 内容轴线；
- Feedback 状态卡与操作卡同顶同底，内部两个全宽操作按钮居中；
- Context Menu 未被父容器裁剪，危险操作具有独立颜色；
- Dialog scrim、Header/Body/Footer 和按钮层级正确。

首次实现快照时，普通 Text 未显示而 Canvas/TextArea 正常。根因是 snapshot 工具在 GPU 读取前丢弃 `UserInterface::Cache`，使普通文本上传缓存的弱引用失效。当前实现让 Cache 保持到 screenshot 完成，从资源生命周期根因修复。

另一处证据链问题是未向 `UserInterface` 发送 `RedrawRequested`，导致 Iced Button
仍保留初始 Disabled 状态，快照中的按钮错误地以低透明度绘制。当前 renderer 在
draw 前执行真实 redraw update，快照才用于状态和颜色验收。

运行命令：

```bash
cargo run --release -p nana-ui --example ui-snapshots --locked
```

截图工具会执行 GPU→CPU 回读和 PNG 编码，只属于验收工具。正式窗口 Demo 与宿主纹理直显不会复用该读取逻辑。

当前命令生成 21 张无损 PNG。离屏快照可以证明布局和像素内容，但不能证明原生
窗口材质、鼠标命中、IME、窗口 resize 或不同 DPI 显示器上的最终表现；这些仍
需在解锁的真实窗口和目标平台补测。
