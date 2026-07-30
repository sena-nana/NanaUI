# LiliaUI 表现对齐基线

NanaUI 不依赖 Vue、DOM 或 CSS，但视觉与交互参数直接以当前 LiliaUI 源码为基线。第一阶段 Demo 已逐项核对以下来源：

| NanaUI 层 | LiliaUI 基线 | 已对齐内容 |
| --- | --- | --- |
| `theme` | `packages/theme/src/styles/tokens.css` | dark/light 语义色、五档交互状态色、16px 基准圆角 |
| `shell` | `TitleBar.vue`、`app-shell.css`、`examples/workspace-regions` | 36px 标题栏、左侧布局切换、居中工作区标题、28px 操作按钮、6px 边缘内距 |
| `workspace` | `LiliaWorkspace.vue`、`workspace.css`、`LiliaWorkspaceRegion.vue` | 动态 Region 注册顺序、role/placement/scope、尺寸约束、折叠/overlay 响应式行为、8px resize 命中区、2px 状态指示线与双击复位 |
| `widgets` | `UiButton.vue`、`UiInput.vue`、`UiTextarea.vue`、`UiCheckbox.vue`、`UiSwitch.vue`、`UiRangeField.vue` | md 控件尺寸、圆角、按钮内容双轴居中、hover/pressed/selected/disabled/invalid 层级 |
| `selection` | `UiListItem.vue`、`UiTabs.vue`、`UiSegmentedControl.vue` | 34px 行高、方向键/Home/End 导航、独立 selected-hover/pressed |
| `overlays` | `Tooltip.vue`、`overlay.css`、`action-menu.css` | 350ms tooltip、6px gap、菜单密度、Dialog 尺寸/Scrim/Header/Body/Footer |

结构区域和内容卡片保持不同语义：侧栏、检查器和底部面板为方形、无边框的 workspace region；只有 Card 与主内容裁剪边界使用圆角。resize handle 空闲时透明，悬停或拖动时只显示居中的 2px 指示线。

同一网格行中的 Card 使用等分宽度和统一高度；菜单、列表、侧栏按钮显式左对齐，
其余文本、图标及组合操作按钮在自身边界内水平、垂直居中。

`ui-snapshots` 使用真实 `WorkspaceState::view` / `GalleryState::view` 和 Iced WGPU 30 renderer 生成 Code/Github/Live2D、dark/light、控件、表面、菜单与对话框快照。`WorkspaceState::view` 本身通过公共 `WorkspaceRegions` / `workspace_view` 组合，因此快照覆盖的是动态框架路径而非单独制作的 Demo 布局。LiliaUI 的实际 `workspace-regions` 示例用于逐一核对三套区域结构与标题栏切换；仓库自身的 `tests/visual/__screenshots__/lilia-layer.png` 用于核对 selected list/card、按钮、输入框与 light/dark 表面层级。

仍不能由离屏快照证明的项目是原生窗口材质、鼠标命中、IME、真实窗口 resize、不同 DPI，以及 Windows/Linux 的最终字体栅格化。它们需要在对应平台的真实窗口中继续验收；这不改变当前 Demo 的 UI 结构与主题合同。
