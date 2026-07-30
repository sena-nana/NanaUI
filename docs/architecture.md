# NanaUI 架构

## 职责边界

当前仓库包含 `nana-ui` 与 `nana-window` 两个 crate。控件、主题、工作区框架和
WGPU View 属于 `nana-ui`；系统窗口材质与必须依赖原生句柄的 macOS 标题区交互
桥接属于 `nana-window`，普通控件不会直接访问平台窗口 API。

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
```

框架与 Demo 的边界是明确的：

- `WorkspaceController` 只拥有布局、交互和视口状态；
- `WorkspaceRegions` 将应用内容绑定到动态注册的 Region ID；
- `WorkspaceSlots` 只为标准六区提供便捷构造，不是框架结构上限；
- `workspace_view` 统一施加区域尺寸、裁剪、表面层级和分隔条；
- Sidebar 原语只负责通用结构与交互消息，不内置应用链接、状态或路由；
- Settings model 只维护稳定 Tab 和恢复规则，具体设置值仍由应用状态拥有；
- `WorkspaceState` 只保存节点、文档、搜索、预览和设置等示例业务状态。

因此消费者不需要复制 `WorkspaceState`，也不需要自己重写区域编排和 resize
事件流；只需注册自己的区域合同和内容。

## 窗口 Chrome 合同

`AppTitleBar` 统一组合 leading、居中标题、trailing 与窗口控制区；
`WindowChromeState` 只维护拖拽手势和最大化显示状态，并向宿主发出
drag/minimize/toggle-maximize/close 语义动作。标准 Iced 应用通过公共控制器执行
`iced::window` Task，宿主事件循环则直接消费同一动作；普通控件不读取原生句柄。

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

Iced Engine 接收宿主 `Device`/`Queue`，不会再次请求设备；`GpuTextureView`
直接采样 scene 的 `TextureView`，不进行 CPU 回读或图片编码。

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

Demo 直接采用 LiliaGithub 实际 AppShell 的单侧栏结构：Code、Github 与 Live2D
都只注册一个 220px `Resources` 起始 Region；项目导航、资源列表和设置 Footer
位于同一 `SidebarFrame`，设置态在同一位置替换普通列表。Github 不注册 bottom，
Live2D 将 bottom role 设为 timeline。Inspector、Toolbar 与 Bottom 只演示
Workspace Region 能力，不构成额外侧栏。布局切换器位于应用标题栏，标题随当前
工作区变化。

原生实现共享视觉层级和交互语义，不引入 Vue、DOM、CSS 或 LiliaUI 运行时依赖。
