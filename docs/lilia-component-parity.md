# LiliaUI 公开组件对应矩阵

本文档以 LiliaUI `8f62756` 的 `@lilia/ui` 公开入口和
`tests/perf/componentPerformance.test.ts` 为盘点基线。后者会枚举所有公开 Vue
组件，并要求每个组件进入性能场景，因此公开入口而不是源码目录是组件范围的权威
来源。LiliaGithub 等消费应用的业务组件不属于 NanaUI。

“完成”必须同时满足：

1. NanaUI 有可被消费者直接使用的公开 Rust API，而不只是私有 Gallery 代码或样式
   函数；
2. 组件的可见操作发送真实应用消息，disabled/loading/selected/invalid 等状态有
   明确合同；
3. 使用统一 `ThemeTokens`、字体和 `ThemeMetrics`；
4. 有与行为风险相称的状态测试、Gallery 实际交互或真实渲染快照；
5. 进入同配置的组件性能与内存测量。

## 当前已实现

| LiliaUI | NanaUI | 说明 |
| --- | --- | --- |
| `UiButton` | `Button` | ghost/primary/secondary/warning/danger/text、三档尺寸、disabled、loading |
| `UiIconButton` | `IconButton` | 方形尺寸、selected/disabled、可见 tooltip |
| `UiCard` | `Card` | surface/outlined/raised/flat/selected、标题与 loading |
| `UiInteractiveCard` | `InteractiveCard` | selected/disabled 与真实选择消息 |
| `UiListItem` | `ListItem` | 三档行高、leading/content/trailing、selected/disabled 与真实选择消息 |
| `UiFormField` | `FormField` | label、hint、error 与任意原生控件内容 |
| `UiEmptyState` | `EmptyState` | 图标、标题、信息与真实 action 内容 |
| `UiProgress` | `Progress` | 数值约束、标签与真实取消消息 |
| `UiSkeleton` | `Skeleton` | 可配置尺寸的语义表面占位 |
| `UiSpinner` | `Spinner` | 仅在调用方提供活动 phase 时重绘 |
| `UiStatusBadge` | `StatusBadge` | neutral/info/success/warning/danger |
| `UiToast` | `Toast` | 四种 tone、描述与真实 dismiss 消息 |
| `UiValidationMessage` | `ValidationMessage` | warning/danger |
| `UiCheckbox` | `Checkbox` | 三档行高、checked、disabled、invalid 与真实 toggle 消息 |
| `UiInput` | `Input` | 三档行高、disabled、invalid 与真实输入消息 |
| `UiRangeField` | `RangeField` | 三档行高、范围、单位读数与真实 change 消息 |
| `UiSwitch` | `Switch` | 三档单行高度、label/hint、disabled、invalid 与真实 toggle 消息 |
| `UiTextarea` | `Textarea` | 调用方拥有 Content，支持 disabled、invalid 与真实 Action |
| `UiSegmentedControl` | `SegmentedControl` | 三档行高、通用值类型、图标、disabled option 与真实选择消息 |
| `UiTabs` | `Tabs` | 三档行高，与分段控件共享选择合同，使用独立的 Tab 表面 |
| `UiSelect` | `Select` | Iced 原生 pick-list、三档尺寸、loading/disabled/invalid；自定义菜单仍由 `Dropdown` 补齐 |
| `UiXYPad` | `XYPad` | 指针与触摸拖动、Shift 主轴锁定、量化、键盘步进、Input/Change 两阶段事件 |
| `Dropdown` | `Dropdown` | 单选/多选、hint、受控选择事件；使用原生菜单的键盘导航和视口约束 |
| `SearchDropdown` | `SearchDropdown` | Iced combo-box 的过滤、键盘导航、hint 与受控选择状态 |
| `UiDialog` | `Dialog` | 标题、描述、正文、Footer、尺寸、关闭/外部点击/内部交互消息 |
| `ConfirmDialog` | `ConfirmDialog` | 复用 Dialog，支持 danger 与真实确认/取消消息 |
| `UiDrawer` | `Drawer` | 左右方向、正文、Footer、关闭与内部交互消息；L2 `nana-drawer` / `side` 已映射 |
| `Tooltip` | `Tooltip` | 统一 placement、delay、viewport padding 与视觉样式 |
| `UiPopover` | `Popover` | 自定义 Iced overlay、受控打开、四向锚定、Escape/外部点击关闭与嵌套交互 |
| `ActionMenu` | `ActionMenu` | 触发器锚定、默认起始边对齐、标准菜单间距与视口约束 |
| `ActionMenuItem` | `ActionMenuItem` | 三档行高、leading/hint/active/danger/disabled 与真实消息 |
| `AnchoredActionMenu` | `AnchoredActionMenu` | 四种锚点方向和视口边界约束 |
| `ContextMenuHost` | `ContextMenuHost` | 层级菜单、搜索、确认标签、危险操作和统一关闭事件 |
| `OverlayHost` | `OverlayHost` | 保持视觉与事件顺序的原生 Stack 宿主 |
| `CalendarHeatmap` | `CalendarHeatmap` | 完整周模型、可配置等级、单 Canvas 缓存、指针/触摸命中和 tooltip |
| `UiImageViewer` | `ImageViewer` | 接收宿主渲染内容、指针中心滚轮缩放、鼠标/触摸平移与关闭合同；不强制编解码器或 CPU 像素副本 |
| `TitleBar` | `AppTitleBar` / `WindowChromeState` | 原生窗口动作与拖拽合同 |
| `LiliaSidebarFrame` | `SidebarFrame` | 固定 top/footer 与独立滚动正文 |
| `LiliaSidebarRow` / `LiliaSidebarNavRow` | `SidebarRow` | 三档行高、状态、tone、层级、leading/trailing、disclosure |
| `LiliaSidebarSection` / `SidebarCollapse` | `SidebarSection` / `SidebarSectionState` | 真实展开状态与按需帧订阅 |
| `LiliaSidebarFooter` | `SidebarFooter` / `SidebarFooterButton` | 固定 Footer 与真实消息 |
| `SettingsRow` | `SettingsRow` | label、hint、stacked/divided/loose |
| `LiliaSettingsSidebar` | `settings_sidebar` | 稳定 Tab ID 与真实选择状态 |
| `LiliaSettingsPage` | `settings_page` | 真实设置内容，不显示未接入项目 |
| `SettingsCollapsibleCard` | `SettingsCollapsibleCard` | 受控折叠、disabled、独立 accessory 与键盘可用按钮 |
| `LiliaAppearanceSection` | `AppearanceSection` | 宿主拥有主题/圆角状态，统一事件覆盖主题、半径、工作区圆角和重置 |
| `LiliaAboutSection` | `AboutSection` / `AboutMetadata` | 消费者注入名称、版本与描述，不绑定应用常量 |
| `LiliaAppShell` | `app_shell` | 标题栏、工作区和 overlay 组合 |
| `LiliaDesktopShell` | `DesktopShell` | 标题栏、动态 Workspace Region、导航 Footer、检查器、底部面板和 overlay 便捷组合 |
| `PopupShell` | `PopupShell` | 普通弹窗与透明状态窗布局 |
| `PopupTitleBarFrame` | `PopupTitleBarFrame` | 回主窗口、新建、最小化、关闭和原生拖拽事件 |
| `LiliaWorkspace` | `workspace_view` / `WorkspaceController` | 动态 Region、响应式、持久化、resize、collapse |
| `LiliaWorkspaceRegion` 与六个 preset | `WorkspaceRegion` / `WorkspaceRegions` / `WorkspaceSlots` | 任意 Region ID 加标准六区便捷入口 |
| `LiliaUIProvider` | `ThemeTokens` 与宿主显式状态 | 原生应用不需要 Vue 注入层；主题和密度合同直接传递 |

## 当前缺口

基于上述公开入口，本轮没有未映射的 LiliaUI Vue 组件。Vue Provider、DOM Teleport、
ARIA 属性和路由链接这类平台机制不复制为伪组件；NanaUI 分别用显式
`ThemeTokens`、Iced overlay、原生消息合同与消费应用导航状态承担对应职责。

## 实施与性能门禁

实现按“基础输入 → 选择/搜索 → 浮层/菜单 → 专业展示 → Shell/设置”推进。每一阶段
先复用现有状态和令牌，再扩展 Gallery 的真实交互与快照，避免为对照名称复制私有
实现。

性能验收沿用 `docs/performance-baseline.md` 的同机、Release、固定窗口、相同预热和
采样设置，并扩展以下场景：

- 每个公开组件的 mount/layout/draw/update 路径；
- 500 项 Dropdown、200 项搜索结果、120 项可搜索菜单；
- 20/50 个 Workspace Region、连续 resize 与折叠；
- CalendarHeatmap 和 ImageViewer 的指针交互；
- Gallery 与 hosted GPU demo 的冷启动、静置 CPU、RSS 和可执行文件体积。

性能结论必须报告中位数、p95、样本数与测量环境；内存使用 RSS，并区分空闲稳定值
和交互峰值。不同 Iced/WGPU 版本或屏幕状态的数据不直接互相判定回退。
