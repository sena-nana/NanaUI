# NanaUI 架构

## 职责边界

当前仓库包含公共组件库 `nana-ui`、平台边界 `nana-window` 与独立 Demo
`component-gallery`。控件、主题、工作区框架和 WGPU View 属于 `nana-ui`；
系统窗口材质与必须依赖原生句柄的 macOS 标题区交互桥接属于 `nana-window`；
Gallery 页面、状态、快照与基准属于 Demo crate。

**产品 UI 前端默认且唯一为 NanaUI（Iced）。** 可选的 Vue/JS 引擎只做状态与命令桥
（`nana-ui-vue` + QuickJS XOR V8）；不引入 WebView、Blitz DOM/CSS 或第二套 wgpu。
Blitz 移除说明见
[`docs/performance/2026-08-06-blitz-removed-nana-frontend.md`](performance/2026-08-06-blitz-removed-nana-frontend.md)。

**三层兼容（桥接合同，非 `nana-ui` 公共依赖）**：

三层都写入同一 **Style Model = Tokens + Semantics + Layout**（见
`nana_ui_core::style_model`），再由 `nana-ui` widgets **唯一绘制**。  
L1 不是「CSS→仅 ThemeTokens」：布局进 Layout，已知 class 进 Semantics，主题档位进
Tokens；任意业务 CSS 色值不得污染正式 token。

L1/L2/L3 是三种输入合同，不是三个运行时模式或三套渲染实现。L1 与 L2 共享一个
`MessageBridge` 语义森林；paint 前统一由 Style Model `measure_layout` 提供几何，paint
后仅以 iced `LayoutProbe` 回写为权威。`NanaTreeDocument` 只保存 DOM 兼容状态和几何
缓存，不再维护独立合成布局。DesktopShell Region 投影为快照级一次索引与父链传播，
不重复扫描整棵森林。

| 层 | 含义 | 住在哪 |
|----|------|--------|
| L1 | 完整 Tauri Vue：兼容接口 + CSS 子集→Style Model（兼容目标，非公共 CSSOM） | `nana-tauri-demo` / `nana-ui-vue` / `nanavue-runtime` / `nana-ui-web-api` |
| L2 | Vue 直接使用 Nana 组件接口（语义 props→同一 Model，可跳过 CSS） | `nanavue-components` + MessageBridge |
| L3 | Rust 原生入口 + 唯一绘制实现 | `nana-ui` / `nana-ui-core` / Gallery |

设计上支持混合显示，尤其是 **L1+L2 同树**。权威细则见
[`vue-nana-renderer-system.md`](vue-nana-renderer-system.md) §0。  
兼容性阶段目标与 Todo：[`compatibility-roadmap.md`](compatibility-roadmap.md)。

```text
应用状态 / 应用消息
        │
        ├── app_shell / AppTitleBar
        │      └── WindowChromeState → 宿主窗口动作
        ├── SidebarFrame / SidebarSection / SidebarRow
        ├── SettingsModel / SettingsState / settings_page
        └── WorkspaceRegions / WorkspaceSlots
                │
                ▼
        workspace_view
                │
                ├── WorkspaceController
                │      ├── WorkspaceLayout
                │      ├── resize / collapse / visibility
                │      ├── JSON persistence
                │      └── viewport geometry
                ├── theme / widgets
                └── application-owned region content
                     （可嵌 L3 控件，或 L1/L2 语义快照）
```

框架与 Demo 的边界是明确的：

- `WorkspaceController` 只拥有布局、交互和视口状态；
- `WorkspaceRegions` 将应用内容绑定到动态注册的 Region ID；
- `WorkspaceSlots` 只为标准六区提供便捷构造，不是框架结构上限；
- `workspace_view` 统一施加区域尺寸、裁剪、表面层级和分隔条；
- Sidebar 原语只负责通用结构与交互消息，不内置应用链接、状态或路由；
- Settings model 只维护稳定 Tab 和恢复规则，具体设置值仍由应用状态拥有；
- `component_gallery::GalleryState` 只保存分类导航、组件交互、外观和设置等
  Demo 状态，不属于 `nana-ui` 公共 API。

消费者只需注册自己的区域合同和内容，不需要复制 Demo 状态，也不需要重写区域
编排和 resize 事件流。

## 窗口 Chrome 合同

`AppTitleBar` 统一组合 leading、居中标题、trailing 与窗口控制区；
`WindowChromeState` 只维护拖拽手势和最大化显示状态，并向宿主发出
drag/minimize/toggle-maximize/close 语义动作。标准 Iced 应用通过公共控制器执行
`iced::window` Task，宿主事件循环则直接消费同一动作；普通控件不读取原生句柄。

标准 Iced 控制器始终绑定明确的 `window::Id`。`new`/`Default` 为单窗口应用提供
便利入口：状态只绑定收到的首个 `Opened`，并过滤其他窗口的生命周期事件；多窗口
应用在 `window::open` 返回 ID 后使用 `for_window` 创建每个窗口独立的状态。
`Opened`、`Resized`、`Closed` 和最大化查询结果都保留来源 ID，所有窗口 Task 直接
使用该 ID，不依赖 `window::latest()`。显式绑定窗口关闭后不会自动接管其他窗口；
重建窗口时由应用调用 `bind`，旧 ID 的迟到结果会因目标不匹配被丢弃。

宿主事件循环仍可通过 `update` 消费相同的语义动作，并通过 `set_maximized` 同步
实际 Winit Window 状态；这个路径不把窗口所有权交给 Iced，也不会执行 Iced
窗口 Task。

macOS 使用 transparent titlebar 与 full-size content view，把 36px NanaUI 标题栏
绘制到窗口顶部，并为左侧原生交通灯保留 78px。Windows/Linux 关闭系统 decorations，
由 `AppTitleBar` 绘制三枚窗口按钮。macOS 默认禁止系统标题区抢占鼠标事件，只有
空白父区域收到按下事件时才通过 `nana-window` 启动 AppKit 原生拖拽；按钮等子控件
会先消费事件。拖拽阈值由 AppKit 负责，其他平台由公共状态机的 4px 阈值负责。
材质与标题栏状态、布局和动作语义仍是彼此独立的宿主合同。

## 工作区合同

`WorkspaceLayout` 按注册顺序持有 `RegionState`。区域合同包含：

| 字段 | 作用 |
| --- | --- |
| `id` | 内建或 `Custom(String)` 稳定标识 |
| `role` | global/section navigation、resources、primary、inspector、timeline、console、utility |
| `placement` | start、primary、end、top、bottom |
| `scope` | workspace 或 primary |
| `size/default/min/max` | 尺寸与约束 |
| `fill_priority` | 多个 primary 区域的填充分配 |
| `collapsible/resizable` | 折叠与拖动能力 |
| `narrow_behavior/collapse_below/responsive_priority` | shrink、collapse、overlay 或 none 响应式策略 |

内建 `RegionId` 为常用工作区提供稳定名字；`Custom(String)` 允许业务增加任意
起始、结束和上下区域。`register` 拒绝重复 ID，`unregister`、JSON restore 与
几何计算都保持剩余区域的注册顺序和结构。

分隔条具有覆盖在区域边缘的 8px 命中区和 2px hover/drag 指示线，不增加 grid
track；拖动按区域位置决定增量方向，双击恢复默认尺寸。折叠或隐藏区域不会
渲染、不会响应 resize，并在几何快照中同时释放空间；overlay 区域则显示但
不占用 primary 空间。

`Tabs::on_reorder` 提供同一 Tab strip 内的可选鼠标/触摸重排合同；回调以“被移动
值 + 其后值（`None` 表示末尾）”表达结果。`TabDragGroup` 通过带 generation 的短期
lease 登记各 strip 当次真实绘制矩形；`TabDragSurface` 将各窗口的物理 origin、逻辑
坐标和 DPI scale 统一到屏幕空间，目标窗口可 relay move/release，并保证一次释放只
产生一次 source/target/before。旧 view 的析构不会删除新 view 的登记。选择、顺序、
Pane/窗口语义和持久化继续由应用持有；接收方还可用 `accepts_external_drop(false)`
保留拖拽源但明确拒绝外部落点。`SelectionOption::draggable(false)` 只排除单个 Tab
的拖拽源和落点，不影响普通选择。`Dock` 不会隐式接管 Editor Tab 的跨 surface 语义。

`ReorderList` 为纵向导航和资源列表提供相同的 before-value 合同，但不承担跨 strip
或跨窗口转移。它只处理点击选择、4px 鼠标/触摸拖拽阈值和插入线；顺序、分组约束、
持久化及业务错误仍由消费应用拥有。列表行必须是被列表统一选择的被动内容，独立按钮
等需要自身指针语义的操作应放在列表行外。`ReorderItem` 默认同时是拖拽源和落点；
`.draggable(false).drop_target(true)` 可声明只接收落点的被动目的行，用于应用拥有的
移动/嵌套面板，但 NanaUI 仍不解释目的值或修改业务资源。

确定性的 `SetRegionCollapsed` 与 `SetRegionSize` action 供设置页和宿主状态同步
使用；它们与拖拽路径共享相同约束。Demo 的设置页使用独立
`WorkspaceController`，因此进入设置不会覆盖应用工作区的尺寸和折叠状态；窗口
尺寸与 DPI 事件会同步给两套控制器。

Region 折叠目标会立即写入 `WorkspaceLayout`，保证序列化与设置页读取到确定
状态；渲染层同时保留一个 240ms 的临时 extent，使用 ease-out 曲线释放或恢复
工作区空间。只有存在过渡时才订阅窗口帧，完成后自动移除临时状态。快速反向
切换会从当前插值位置继续，不重置尺寸或产生跳变。

`WorkspaceGeometry` 将相同布局映射为逻辑与物理像素矩形，供宿主 WGPU View
设置 viewport/scissor。它不创建窗口或 GPU 资源。

## WGPU 边界

NanaUI 当前使用 Iced `0.15.0-dev` 分叉与 WGPU `30.0.0`。`GpuView` 实现
Iced WGPU shader primitive；`RenderSlot` 负责逻辑/物理像素换算与裁剪。

`hosted-gpu-demo` 按职责拆分为 context、scene、panel 与 runner：

- context 创建并配置唯一 GPU 上下文；
- scene 管理应用 WGPU pipeline 与宿主纹理；
- panel 描述 NanaUI 内容和应用状态；
- runner 处理 Winit 事件与同帧调度。

Iced Engine 接收宿主 `Device`/`Queue`，不会再次请求设备；每个附加工具窗口只新增
自己的 Window、Surface 和 Iced renderer，并继续共享同一 Adapter/Device/Queue。
`GpuTextureView` 直接采样 scene 的 `TextureView`，不进行 CPU 回读或图片编码。

## 原生富文本合同

`NativeMarkdown` 只持有 UI 无关的原生块模型。GFM 表格保留列对齐、表头、行和
单元格内 span，不得降级成插入分隔符的普通文本；视图使用等宽列、原生横向滚动
和 `SelectableRichText` 呈现，不依赖 WebView。

`SelectableRichText` 负责 Unicode grapheme 命中、单词/整块选择、链接点击和
Iced clipboard 写入。跨块的完整内容复制由 `NativeMarkdown::plain_text` 提供稳定
纯文本合同，消费应用再通过自己的 Host/权限边界写入系统剪贴板；NanaUI 不持有
应用任务、时间线或宿主状态。

## 与 LiliaUI 的对应关系

| NanaUI | LiliaUI 语义 |
| --- | --- |
| `app_shell` / `AppTitleBar` / `WindowChromeState` | `LiliaAppShell` / `TitleBar` / `useNativeWindowChrome` |
| `WorkspaceController` | `LiliaWorkspace` 的布局上下文 |
| `RegionState` / `WorkspaceRegions` / `workspace_view` | `LiliaWorkspaceRegion` 的注册合同与区域组合 |
| `SidebarFrame` / `SidebarSection` / `SidebarRow` | `LiliaSidebarFrame` / `LiliaSidebarSection` / `LiliaSidebarRow` |
| `SettingsModel` / `settings_sidebar` / `settings_page` | LiliaUI settings model、`SettingsSidebar` 与 `SettingsPage` |
| `GlobalNavigation` | `LiliaGlobalNavigation` |
| `Resources` | `LiliaResourcePanel` |
| `Primary` | `LiliaPrimaryContent` |
| `Inspector` | `LiliaInspector` |
| `Diagnostics` | bottom console/timeline region |

Gallery 直接采用 LiliaGithub 实际 AppShell 的单侧栏结构：一个 220px
`Resources` 起始 Region 承载分类导航与设置 Footer，设置态在同一位置替换普通
列表。控件、表面与反馈分类只显示主内容；工作区分类启用 Toolbar、Inspector 与
Bottom，以真实折叠、resize 和复位操作展示 Workspace Region 能力。标题栏固定为
`NanaUI Gallery`，只承载侧栏、主题与窗口操作。

原生实现共享视觉层级和交互语义，不引入 Vue、DOM、CSS 或 LiliaUI 运行时依赖。

## 编译与加载边界

`nana-ui` 默认不启用可选 feature。消费者按职责显式启用 `calendar`、
`controls`、`feedback`、`image-viewer`、`overlays`、`popover`、`selects`、
`settings-components`、`surfaces` 或 `xy-pad`。Cargo 不会根据消费代码引用
自动推断 feature。`gpu` 与 `bundled-fonts` 是独立的上层边界，完整库能力由
`full` 聚合，完整组件集合由 `components` 聚合。

组件族拥有稳定的 `components::<family>` 子模块，同时保留 crate 根的现有
re-export。未启用模块不参与编译；已启用但未引用的 Rust item 由 Release 链接器
裁剪。内置字体只在 `bundled-fonts` 开启时通过 `include_bytes!` 进入编译图，
关闭后宿主可注册自己的同名字体。

`surfaces` 提供标准卡片之外的紧凑 `DockPanel`、`FormField`、`EmptyState` 与
`LabeledValue`，`feedback` 提供连续采样的 `LevelMeter`。它们只表达通用布局和
语义状态，不包含场景、Cue、连接或音频总线等业务概念。

`component-gallery` 作为独立 workspace crate 显式依赖 `components`，并在自己的
默认 feature 中转发 `bundled-fonts`，因此 `cargo run -p component-gallery`
无需污染组件库默认能力。Iced 只构造当前 `view` 返回的组件树；Demo 用单线程
`OnceCell` 在首次显示反馈页时创建日历模型、首次打开上下文菜单时创建菜单数据。
Workspace 组合与几何计算按 Region 数量线性处理。
