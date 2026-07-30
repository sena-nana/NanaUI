# LiliaUI 表现对齐基线

NanaUI 不依赖 Vue、DOM 或 CSS，但视觉与交互参数直接以当前 LiliaUI 源码为基线。第一阶段 Demo 已逐项核对以下来源：

| NanaUI 层 | LiliaUI 基线 | 已对齐内容 |
| --- | --- | --- |
| `theme` | `packages/theme/src/styles/tokens.css` | dark/light 语义色、五档交互状态色、16px 基准圆角 |
| `shell` | `LiliaGithub/src/layouts/AppShell.vue`、`TitleBar.vue`、`app-shell.css` | 36px 标题栏、28px 侧栏折叠按钮、居中工作区标题、6px 边缘内距 |
| `workspace` | `LiliaWorkspace.vue`、`workspace.css`、`LiliaWorkspaceRegion.vue` | 动态 Region 注册顺序、role/placement/scope、尺寸约束、折叠/overlay 响应式行为、覆盖在区域边缘的 8px resize 命中区、2px 状态指示线与双击复位 |
| `widgets` | `UiButton.vue`、`UiInput.vue`、`UiTextarea.vue`、`UiCheckbox.vue`、`UiSwitch.vue`、`UiRangeField.vue` | md 控件尺寸、圆角、按钮内容双轴居中、hover/pressed/selected/disabled/invalid 层级 |
| `selection` | `UiListItem.vue`、`UiTabs.vue`、`UiSegmentedControl.vue` | 34px 行高、方向键/Home/End 导航、独立 selected-hover/pressed |
| `sidebar` | `LiliaGithub/src/layouts/SecondaryPanel.vue`、`SidebarFooter.vue`、`LiliaSidebarFrame.vue`、`LiliaSidebarSection.vue`、`LiliaSidebarRow.vue` | 单个 220px ResourcePanel、可滚动主体、固定 Footer、26px 设置图标按钮、统一 28px 列表/分区行、折叠与 selected/disabled/tone |
| `settings` | `settings.ts`、`SettingsSidebar.vue`、`SettingsPage.vue`、`SettingsRow.vue` | 稳定 Tab、别名回退、返回导航、复用 28px `SidebarRow` 分类、普通/full-page 页面、label/hint 行，以及可即时预览的主题与标准圆角 |
| `overlays` | `Tooltip.vue`、`overlay.css`、`action-menu.css` | 350ms tooltip、6px gap、菜单密度、Dialog 尺寸/Scrim/Header/Body/Footer |

实际 LiliaGithub AppShell 只注册一个 `id="sidebar"` 的 ResourcePanel。普通
`SecondaryPanel` 与 `LiliaSettingsSidebar` 在该 Region 内互斥切换，不存在常驻
global rail、section navigation 与 resources 三列侧栏。NanaUI Demo 的
1280×720 Code 基线因此采用以下单侧栏矩阵：

| 区域 | x | y | width | height |
| --- | ---: | ---: | ---: | ---: |
| titlebar | 0 | 0 | 1280 | 36 |
| sidebar | 0 | 36 | 220 | 684 |
| primary toolbar | 220 | 36 | 780 | 34 |
| primary | 220 | 70 | 780 | 450 |
| inspector | 1000 | 36 | 280 | 684 |
| bottom | 220 | 520 | 780 | 200 |

resize handle 是绝对覆盖层，不参与上述 grid track 尺寸；底部内容从 1px 分隔线
后的 y=521 开始。Code/Github/Live2D 的可选 Inspector、Toolbar 与 Bottom
继续用于演示 Workspace Region 能力，但不会增加第二层左侧导航。

结构区域和内容卡片保持不同语义：侧栏、检查器和底部面板为方形、无边框的 workspace region；只有 Card 与主内容裁剪边界使用圆角。resize handle 空闲时透明，悬停或拖动时只显示居中的 2px 指示线。

同一网格行中的 Card 使用等分宽度和统一高度；菜单、列表、侧栏按钮显式左对齐，
其余文本、图标及组合操作按钮在自身边界内水平、垂直居中。

设置 Demo 不复制 Vue Router 或 localStorage。独立设置 Workspace 保留应用工作区
状态，`SettingsState`、`ThemeMode`、`AppearanceSettings` 与 `WorkspaceLayout`
交由宿主组合持久化。侧栏分区标题与列表项读取同一运行时控件圆角，不再维护两份
样式常量。

NanaUI 内置并注册 LiliaUI `fonts.css` 对应的 Noto Sans SC
400/500/600/700 字体面；标题使用同样的 0.2px tracking，分区与 Card 标题使用
0.5px tracking。这样文字宽度来自同一字体数据，而不是依赖平台 fallback。

`ui-snapshots` 使用真实 `WorkspaceState::view` / `GalleryState::view` 和 Iced WGPU 30 renderer 生成 Code/Github/Live2D、dark/light、控件、表面、菜单与对话框快照。`WorkspaceState::view` 本身通过公共 `WorkspaceRegions` / `workspace_view` 组合，因此快照覆盖的是动态框架路径而非单独制作的 Demo 布局。LiliaGithub 的实际 AppShell、SecondaryPanel、SidebarFooter 与 LiliaUI token/CSS 源码是结构、尺寸和颜色的权威依据。

仍不能由离屏快照证明的项目是原生窗口材质、鼠标命中、IME、真实窗口 resize、
不同 DPI，以及 Windows/Linux 的最终抗锯齿差异。字体族与字重数据已经统一，
但最终栅格仍需在对应平台真实窗口中验收。
