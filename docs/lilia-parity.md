# LiliaUI 表现对齐基线

NanaUI 产品路径是 Runtime / UiScene，由 `SceneWgpuPainter` 绘制。视觉与交互参数直接以当前 LiliaUI 源码为基线。第一阶段 Demo 已逐项核对以下来源：

| NanaUI 层 | LiliaUI 基线 | 已对齐内容 |
| --- | --- | --- |
| `theme` | `packages/theme/src/styles/tokens.css` | dark/light 语义色、五档交互状态色、16px 基准圆角 |
| `shell` | `LiliaGithub/src/layouts/AppShell.vue`、`TitleBar.vue`、`useNativeWindowChrome.ts`、`app-shell.css` | 36px 自绘标题栏、三列居中、28px 窗口按钮、4px 拖拽阈值、macOS 78px 原生交通灯 inset、Windows/Linux 自绘 controls |
| `workspace` | `LiliaWorkspace.vue`、`workspace.css`、`LiliaWorkspaceRegion.vue` | 动态 Region 注册顺序、role/placement/scope、尺寸约束、折叠/overlay 响应式行为、主区域 `edge-start` / `edge-end` 自动圆角、覆盖在区域边缘的 8px resize 命中区、2px 状态指示线与双击复位 |
| `widgets` | `UiButton.vue`、`UiInput.vue`、`UiTextarea.vue`、`UiCheckbox.vue`、`UiSwitch.vue`、`UiRangeField.vue` | md 控件尺寸、圆角、按钮内容双轴居中、hover/pressed/selected/disabled/invalid 层级 |
| `selection` | `UiListItem.vue`、`UiTabs.vue`、`UiSegmentedControl.vue` | 34px 行高、方向键/Home/End 导航、独立 selected-hover/pressed、分段控件内外同心圆角 |
| `sidebar` | `LiliaGithub/src/layouts/SecondaryPanel.vue`、`SidebarFooter.vue`、`LiliaSidebarFrame.vue`、`LiliaSidebarSection.vue`、`LiliaSidebarRow.vue` | 单个 220px ResourcePanel、可滚动主体、固定 Footer、26px 设置图标按钮、统一 28px 列表/分区行、折叠与 selected/disabled/tone |
| `settings` | `settings.ts`、`SettingsSidebar.vue`、`SettingsPage.vue`、`SettingsRow.vue` | 稳定 Tab、别名回退、返回导航、复用 28px `SidebarRow` 分类、普通/full-page 页面、label/hint 行，以及可即时预览的主题与标准圆角 |
| `overlays` | `Tooltip.vue`、`overlay.css`、`action-menu.css` | 350ms tooltip、6px gap、菜单密度、Dialog 尺寸/Scrim/Header/Body/Footer |

实际 LiliaGithub AppShell 只注册一个 `id="sidebar"` 的 ResourcePanel。普通
`SecondaryPanel` 与 `LiliaSettingsSidebar` 在该 Region 内互斥切换，不存在常驻
global rail、section navigation 与 resources 三列侧栏。NanaUI Gallery 同样只
保留一个分类侧栏；1280×800 工作区分类采用以下矩阵：

| 区域 | x | y | width | height |
| --- | ---: | ---: | ---: | ---: |
| titlebar | 0 | 0 | 1280 | 36 |
| sidebar | 0 | 36 | 220 | 764 |
| primary toolbar | 220 | 36 | 780 | 34 |
| primary | 220 | 70 | 780 | 550 |
| inspector | 1000 | 36 | 280 | 764 |
| bottom | 220 | 620 | 780 | 180 |

resize handle 是绝对覆盖层，不参与上述 grid track 尺寸。Inspector、Toolbar 与
Bottom 只在工作区分类启用；切换到控件、表面或反馈时隐藏，但不会增加第二层左侧
导航，也不会丢失用户调整过的尺寸和折叠状态。

结构区域和内容卡片保持不同语义：侧栏、检查器和底部面板为方形、无边框的
workspace region；只有 Card 与主内容裁剪边界使用圆角。主区域不是固定四角：
展开的 start/end Region 会让相邻侧保留页面圆角，缺少对应 Region 时主区域贴边，
该侧上下两角归零。这还原的是 LiliaUI 在 `60e0e2b` 移除前由
`data-edge-start` / `data-edge-end` 驱动的原始行为。resize handle 空闲时透明，
悬停或拖动时只显示居中的 2px 指示线。

同一网格行中的 Card 使用等分宽度和统一高度；菜单、列表、侧栏按钮显式左对齐，
其余文本、图标及组合操作按钮在自身边界内水平、垂直居中。

设置 Demo 不复制 Vue Router 或 localStorage。独立设置 Workspace 保留应用工作区
状态，`SettingsState`、`ThemeMode`、`AppearanceSettings` 与 `WorkspaceLayout`
交由宿主组合持久化。侧栏分区标题与列表项读取同一运行时控件圆角，不再维护两份
样式常量。`AppearanceSettings` 同时持久化主区域圆角开关：默认开启贴边自适应
圆角，关闭后四角均为直角；旧配置缺少开关字段时直接拒绝，不进行迁移。

NanaUI 内置并注册 LiliaUI `fonts.css` 对应的 Noto Sans SC
400/500/600/700 字体面；标题使用同样的 0.2px tracking，分区与 Card 标题使用
0.5px tracking。这样文字宽度来自同一字体数据，而不是依赖平台 fallback。

独立 `component-gallery` crate 的 `ui-snapshots` 使用真实 `GalleryState::view`
和 Iced WGPU 30 renderer 生成四个 Gallery 分类、dark/light、设置、菜单与对话框
快照，并单独覆盖 custom controls 与 native-leading 标题栏。Gallery 本身通过
公共 `WorkspaceRegions` / `workspace_view` 组合，因此快照覆盖动态框架路径而非
单独制作的静态布局。
LiliaGithub 的实际 AppShell、SecondaryPanel、SidebarFooter 与 LiliaUI token/CSS
源码仍是结构、尺寸和颜色的权威依据。

仍不能由离屏快照证明的项目是原生窗口材质、鼠标命中、IME、真实窗口 resize、
不同 DPI，以及 Windows/Linux 的最终抗锯齿差异。字体族与字重数据已经统一，
但最终栅格仍需在对应平台真实窗口中验收。
