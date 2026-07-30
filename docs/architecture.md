# NanaUI 架构

## 职责边界

当前仓库包含 `nana-ui` 与 `nana-window` 两个 crate。控件、主题、工作区框架和
WGPU View 属于 `nana-ui`；系统窗口材质属于 `nana-window`，普通控件不会直接
访问平台窗口 API。

```text
应用状态 / 应用消息
        │
        ├── app_shell / app_title_bar
        │
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
- `WorkspaceState` 只保存节点、文档、搜索、预览和设置等示例业务状态。

因此消费者不需要复制 `WorkspaceState`，也不需要自己重写区域编排和 resize
事件流；只需注册自己的区域合同和内容。

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

分隔条具有 8px 命中区和 2px hover/drag 指示线；拖动按区域位置决定增量方向，
双击恢复默认尺寸。折叠或隐藏区域不会渲染、不会响应 resize，并在几何快照中
同时释放空间；overlay 区域则显示但不占用 primary 空间。

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
| `app_shell` / `app_title_bar` | `LiliaAppShell` |
| `WorkspaceController` | `LiliaWorkspace` 的布局上下文 |
| `RegionState` / `WorkspaceRegions` / `workspace_view` | `LiliaWorkspaceRegion` 的注册合同与区域组合 |
| `GlobalNavigation` | `LiliaGlobalNavigation` |
| `Resources` | `LiliaResourcePanel` |
| `Primary` | `LiliaPrimaryContent` |
| `Inspector` | `LiliaInspector` |
| `Diagnostics` | bottom console/timeline region |

Demo 直接复现 LiliaUI `workspace-regions` 示例的三种结构：Code 使用 global +
section + resources，Github 动态增加 Pull Requests 且不注册 bottom，Live2D
不注册 global navigation 并将 bottom role 设为 timeline。布局切换器位于应用
标题栏，标题随当前工作区变化。

原生实现共享视觉层级和交互语义，不引入 Vue、DOM、CSS 或 LiliaUI 运行时依赖。
