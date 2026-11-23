---
title: 工作台：Shell、Workspace、Dock 与 Overlay
---

# 工作台：Shell、Workspace、Dock 与 Overlay

工作台层（Shell / Workspace / Dock / AreaLayout / Overlays）是 Runtime 中提供「桌面应用骨架」的子系统：它不引入新的渲染机制，而是将保留树、布局引擎与输入路由组装成自绘标题栏、多区域工作区、可拆分停靠面板以及模态/非模态浮层。本章的核心主线是**逻辑模型与视图呈现分离**：区域尺寸、折叠过渡、Dock 递归树、AreaLayout 切分模型与外观设置全部保存在可序列化的纯数据模型中，组件投影只负责将模型状态翻译为样式与拓扑变异。

## 职责与边界

- **`nana-ui-core` 持有后端中立的逻辑模型**：`WorkspaceModel` / `WorkspaceLayout` / `RegionState`（`workspace_model.rs`、`layout.rs`）、`AppearanceSettings` / `SettingsModel`（`settings.rs`）以及带版本信封的视图状态持久化 `ViewStateStore`（`persist.rs`）。它们不依赖任何窗口或渲染后端（`crates/nana-ui-core/src/workspace_model.rs:83`）。
- **`nana-ui-runtime` 持有组件组装与交互编排**：`Workspace` / `DesktopShell` / `AppShell` / `AppTitleBar`（`workspace.rs`、`shell.rs`）、`DockNode` / `DockWorkspace` / `Dock`（`dock.rs`，在 Cargo feature `dock` 后面；`Workspace` 与 Shell 始终编入）、DCC 式切分 `AreaLayout`（`area_layout.rs`）与属性检查器 `PropertyGrid`（`property_grid.rs`）、浮层激活与路由（`framework/overlay.rs`、`framework/modal.rs`），以及侧栏、分栏与通知栈组件（`sidebar.rs`、`panel.rs`、`pane.rs`、`pane_section.rs`、`toast_stack.rs`、`overlay_surfaces.rs`）。
- **宿主层（`nana-ui`）只做薄适配**：`WorkspaceController` 仅将 `Instant` 时钟与窗口指针事件翻译为 `WorkspaceMutation`，绝不存储第二份区域状态（`crates/nana-ui/src/workspace.rs:74`）。
- **业务内容与序列化边界**：区域和窗格内部展示什么业务内容、如何路由导航完全归应用所有；只有纯数据逻辑模型写入存档，运行期挂载的子节点句柄与浮层激活态不进入持久化 JSON（`crates/nana-ui-runtime/src/dock.rs:555`）。

## 依赖与邻居

| 层级 / 模块 | 提供什么能力 |
| --- | --- |
| `nana-ui-core` | 区域模型、外观与设置模型、`ViewStateStore`、动效时序（`MotionTiming`） |
| `nana-ui-runtime` | 组件装配与投影、三段式拖拽缩放交互、浮层焦点事务、Dock 与 AreaLayout 树操作 |
| 输入链路（[输入链路](/learn/runtime-input)） | 指针按下/移动经 `dispatch.rs` 责任链进入 `begin_workspace_resize`、`begin_dock_split_resize` 等入口 |
| 布局引擎（[布局引擎](/learn/runtime-layout)） | 求解区域网格轨道、Dock 分割条、AreaLayout 嵌套 Flex 容器与浮层遮罩几何 |
| `nana-ui` 宿主 | 窗口创建、标题栏原生控制按钮、浮动 Dock 独立窗口对账、设置存盘时机 |

## 关键类型与数据

| 类型 | 所在文件与行号 | 作用说明 |
| --- | --- | --- |
| `WorkspaceMutation` | `crates/nana-ui-core/src/workspace_model.rs:13` | 区域交互的 12 种逻辑变异（`ToggleRegion` / `ResizeMove` / `SetViewport` 等） |
| `WorkspaceModel` | `crates/nana-ui-core/src/workspace_model.rs:85` | 区域布局、折叠过渡、拖拽状态与视口事实的唯一权威 |
| `RegionState` / `RegionId` | `crates/nana-ui-core/src/layout.rs:94` / `:15` | 单个区域的结构与持久化合同（`placement` / `scope` / 尺寸约束） |
| `WorkspaceLayout` | `crates/nana-ui-core/src/layout.rs:386` | `Vec<RegionState>` 及标准六区域默认预设 |
| `Workspace` | `crates/nana-ui-runtime/src/workspace.rs:114` | 树上的工作区组件，持有模型快照与轨道 chrome 槽位 |
| `DockNode` / `DockWorkspace` | `crates/nana-ui-runtime/src/dock.rs:59` / `:296` | 递归停靠树（`Item` / `Tabs` / `Split`）与产品级多表面停靠权威 |
| `DockDropZone` | `crates/nana-ui-runtime/src/dock.rs:49` | 拖放落区：`Left` / `Right` / `Top` / `Bottom` / `Tab` |
| `AreaLayout` / `AreaLayoutModel` | `crates/nana-ui-runtime/src/area_layout.rs` | DCC 式无重叠递归区域切分组件与纯数据模型 |
| `PropertyGrid` / `PropertySection` | `crates/nana-ui-runtime/src/property_grid.rs` | 检查器式两列密集属性面板（可折叠分组 + 全网格同步拖拽标签列宽） |
| `RuntimeOverlayKind` | `crates/nana-ui-runtime/src/framework/overlay.rs:14` | `Dialog` / `Panel` / `Menu` / `Tooltip` / `Status` 五种浮层交互行为 |
| `OverlayHost` | `crates/nana-ui-runtime/src/view_components.rs:3478` | 浮层挂载点组件（节点标签 `overlay-host`） |
| `OverlayVisibility` | `crates/nana-ui-runtime/src/overlay_visibility.rs:55` | 媒体控制条与悬浮 chrome 自动隐藏状态机 |
| `PaneTree` / `PaneSection` | `crates/nana-ui-runtime/src/pane.rs:616` / `pane_section.rs:41` | 内容分栏树与带页签行的复合面板区段 |
| `ViewStateStore` | `crates/nana-ui-core/src/persist.rs:146` | 带 `schema_version` 版本信封的视图状态持久化存储 |
| `AppearanceSettings` / `SettingsModel` | `crates/nana-ui-core/src/settings.rs:101` / `:572` | 圆角档位、窗口材质、毛玻璃目标等外观配置与设置页模型 |

## 内部结构

### 1. 逻辑状态与呈现分离

`Workspace` 组件的类型文档（`crates/nana-ui-runtime/src/workspace.rs:108`）写明了核心准则：**"Snapshot the model; `project` never owns a clock"**——区域的可见性、overlay 态、当前宽度与拖拽高亮全部直接查询 `Self::model`，不维护任何镜像缓存。所有交互统一调用 `Workspace::apply(mutation, now)`（`:156`），时间戳 `now` 由外部传入。当 `Workspace` 挂载在页面级桌面外壳 `DesktopShell`（`crates/nana-ui-runtime/src/shell.rs:884`）下时，每次成功变异后通过 `write_back_shell_model`（`workspace.rs:1185`）将最新模型快照同步回父级 Shell。

### 2. 标准六区域与折叠过渡采样

`WorkspaceLayout`（`crates/nana-ui-core/src/layout.rs:390`）默认提供标准六区域：

| 区域 ID | 默认尺寸 | 最小 / 最大约束 | 说明 |
| --- | --- | --- | --- |
| `GlobalNavigation` | `56 px` | `44 / 96 px` | 最左侧全局导航条 |
| `Resources` | `260 px` | `180 / 520 px` | 左侧资源面板，可折叠、可拖拽缩放 |
| `PrimaryToolbar` | `34 px` | — | 主区域（`Primary` scope）顶部工具栏 |
| `Primary` | 自适应填充 | 最小 `320 px` | `fill_priority = 1` 的核心主工作区 |
| `Inspector` | `280 px` | `200 / 560 px` | 右侧属性检查器，可折叠、可拖拽缩放 |
| `Diagnostics` | `200 px` | `96 / 520 px` | 主区域底部诊断与控制台面板 |

区域折叠动画 `RegionTransition`（`workspace_model.rs:44`）使用 `EaseInOutCubic` 在 `WORKSPACE_REGION_TRANSITION_DURATION = 260 ms`（`:10`）内对 `from_extent` 与 `to_extent` 插值；几何采样时通过 `with_transient_extents`（`layout.rs:450`）将瞬时宽度写入临时布局快照，持久化模型本身绝不被过渡中间值污染。

### 3. 三段式指针拖拽交互（Workspace / Dock / AreaLayout）

工作台层的所有拖拽缩放交互均遵循统一的 `begin / update / end` 三段式，由输入链路的 `dispatch_pointer` 驱动：

| 交互类型 | 三段式入口锚点 | 核心行为 |
| --- | --- | --- |
| Workspace 区域缩放 | `begin_workspace_resize`（`workspace.rs:1041`）/ `update :1074` / `end :1100` | 应用 `ResizeStart` + `ResizeMove`，并在同一事务内执行 `request_focus` + `capture_pointer`（`:1063-1064`） |
| Dock 分割条拖拽 | `begin_dock_split_resize`（`dock.rs:2602`）/ `update :2647` / `end :2701` | 唯一分割比例公式 `dock_split_ratio_from_pointer`（`:796`），范围钳制在 `0.05..=0.95` |
| Dock 窗格跨区拖放 | `dock.rs:2771` / `:2808` / `:2851` | 按 `DockDropZone`（`Left/Right/Top/Bottom/Tab`）决定拆分或合并为 Tab |
| AreaLayout 分隔条与角块切分 | `crates/nana-ui-runtime/src/area_layout.rs` | `2 px` 间隙拥有至少 `6 px` 抓取热区；从区域头部左上/右上角块向内拖切分新区域，向兄弟半区拖则吞并合并 |

### 4. Dock 递归树、分栏槽位与 DCC 区域切分

- **`DockWorkspace` 与浮动窗事件**：`DockNode`（`dock.rs:59`）是 `Item` / `Tabs` / `Split` 三态递归树；`DockWorkspace`（`:296`）管理主树与多个浮动窗口表面（`DockSurfaceSpec`，`:329`）。当用户将窗格拖出成为独立窗口时，`float_item_at`（`:396`）从树中摘除该项并返回 `DockWorkspaceEvent::OpenFloating`，交由宿主打开原生窗口（窗口键由 `dock_surface_window_key:251` 确定性派生），Runtime 自身绝不直接触碰平台窗口句柄。
- **`PaneTree` 与 `PaneSection`**：`PaneTree`（`pane.rs:616`）为每个分栏和叶子分配独立的 `PaneSlot`（`:485`），避免将分栏宽度直接写到业务内容节点上被业务重投影冲掉；`PaneSection`（`pane_section.rs:41`）封装「标题 + 页签行 + 弹性主体」复合卡片。
- **`AreaLayout` 与 `PropertyGrid`**：面向三维与动画工具的 `AreaLayout` 将区域树映射为嵌套 Flex 容器，结构变化时仅重建切分容器与分隔条，仍在的区域原位保留头部、内容与正在拖拽的角块；切换工作区预设（`set_area_layout`）时未出现的区域子节点隐藏等待而不销毁。配套的 `PropertyGrid` 提供带 `6 px` 同步拖拽分隔条的公共标签列、可折叠分组、多选混合值（`mixed`）与带原因说明的禁用行遮罩。

### 5. Overlays：同事务焦点迁移与自动隐藏

`AppShell`（`crates/nana-ui-runtime/src/shell.rs:704`）组装自绘标题栏 `AppTitleBar`（`shell.rs:82`，窗口按钮发出类型化的 `WindowChromeAction:38`）、填充主体与 `OverlayHost`（`view_components.rs:3478`）。

`overlay_kind_for_role`（`framework/overlay.rs:44`）根据无障碍角色将浮层映射为五种 `RuntimeOverlayKind`：`Dialog` 与 `Menu` 阻断底层指针并困住 Tab 焦点（focus trap），`Panel` 仅自身接收指针而不锁底层，`Tooltip` 与 `Status` 完全被动且不抢焦点（`ToastStack` 位于 `toast_stack.rs:43`，先完成布局再显示，悬停或聚焦时暂停倒计时）。激活浮层统一调用 `AppContext::activate_overlay`（`framework/modal.rs:221`）：**在同一个 Runtime 事务内同时激活浮层并把焦点移入其内部子树**，彻底消除「弹窗已显示但焦点还留在底层输入框」的中间帧。

### 6. 版本信封持久化（`ViewStateStore`）

区域布局（`WorkspaceModel::layout_json`）、Dock 树（`DockWorkspace::save_to_store`）与外观设置（`AppearanceSettings::save_to_store`）统一通过 `ViewStateStore`（`crates/nana-ui-core/src/persist.rs:146`）写入带 `schema_version` 的版本信封。读取时若遇到版本号不匹配或 JSON 损坏，会自动删除坏条目、递增 `SCHEMA_MISMATCHES` / `CORRUPTIONS` 诊断计数并优雅回退至默认布局（`:174-185`）。

## 协作与数据流

以「拖拽检查器边缘调整宽度」为例：

```text
指针按下 → 输入 dispatch 链命中手柄 → begin_workspace_resize（workspace.rs:1041）
  → WorkspaceModel 应用 ResizeStart / ResizeMove（尺寸钳制）
  → 同一事务内 request_focus + capture_pointer + write_back_shell_model
  → Workspace 组件重投影，将新宽度写入 MutationQueue
  → UiWorld::commit → RuntimeLayoutEngine 增量重排相邻轨道
  → 帧边界 RuntimeDocument::flush 排空系统工作 → UiScene::apply_delta → 上屏
```

## 代码地图

| 文件路径 | 核心内容 |
| --- | --- |
| `crates/nana-ui-core/src/workspace_model.rs` | `WorkspaceMutation`、`WorkspaceModel`、折叠过渡采样与 JSON 序列化 |
| `crates/nana-ui-core/src/layout.rs` | `RegionId`、`RegionState`、`WorkspaceLayout`、`NarrowBehavior` |
| `crates/nana-ui-core/src/settings.rs` | `AppearanceSettings`、`SettingsModel`、`SettingsState` |
| `crates/nana-ui-core/src/persist.rs` | `ViewStateStore` 版本信封、`LocalStorageAdapter` |
| `crates/nana-ui-runtime/src/shell.rs` | `AppTitleBar`、`AppShell`、`DesktopShell`、`WindowChromeAction` |
| `crates/nana-ui-runtime/src/workspace.rs` | `Workspace` 组件、轨道组装、三段式 resize 与 hooks |
| `crates/nana-ui-runtime/src/dock.rs` | `DockNode`、`DockWorkspace`、`Dock` 组件、拖拽/键盘分割与持久化 |
| `crates/nana-ui-runtime/src/area_layout.rs` | `AreaLayout` 与 `AreaLayoutModel`（DCC 式区域切分、合并与预设切换） |
| `crates/nana-ui-runtime/src/property_grid.rs` | `PropertyGrid`、`PropertySection`、`PropertyRow`（检查器式属性面板） |
| `crates/nana-ui-runtime/src/framework/{overlay,modal,auto_overlay}.rs` | `RuntimeOverlayKind`、`activate_overlay` 同事务焦点迁移、自动隐藏路由 |
| `crates/nana-ui-runtime/src/{overlay_surfaces,overlay_visibility,panel,pane,pane_section,sidebar,toast_stack}.rs` | 对话框/抽屉、媒体条自动隐藏、命名面板、分栏树、侧栏与通知栈 |
| `crates/nana-ui/src/workspace.rs` | `WorkspaceController` 宿主适配器 |

## 延伸阅读

- [保留树：UiWorld](/learn/runtime-tree)：组件投影产生的变异如何原子提交，帧内系统工作如何排空
- [输入链路](/learn/runtime-input)：拖拽手柄、分割条与浮层指针如何被路由到本章的交互入口
- [布局引擎](/learn/runtime-layout)：区域轨道、Dock 分割与 AreaLayout 的盒子几何如何求解
- [工作区、停靠与区域布局（参考）](/reference/workspace)
