# 架构

这篇给**改 NanaUI 的人**。写应用请看 [框架如何运行](how-it-works.md) 和 [开始](start.md)。

工作区、窗口、GPU、控件的消费合同分别在 [工作区](workspace.md)、[窗口](window.md)、[实时画面](gpu.md)、[控件](components.md)。这里只固定 crate 边界和所有权，避免再长出第二套权威树。

## Crate 边界

```text
应用 / Demo
    │
    ▼
nana-ui                 宿主适配器：run_runtime、控件再导出、SceneWgpuPainter
    ├── nana-ui-runtime 保留树权威（UiWorld）、内建控件、Shell、Workspace、
    │                   Dock、GPU 槽。不依赖 WGPU
    ├── nana-ui-scene   绘制图权威（UiScene）。依赖 runtime，不依赖 WGPU
    ├── nana-ui-core    共享合同：Style Model、主题令牌、WorkspaceModel、几何
    ├── nana-ui-platform  WindowId、输入、IME、剪贴板（与 winit 转换隔离）
    └── nana-window     系统材质、标题栏拖拽 / 客户区 chrome / 缩放；
                        普通控件不得拿窗口句柄

Vue 兼容（可选）
    nana-ui-vue + nana-js-v8 + nanavue-runtime / nanavue-components
    写入同一棵 UiWorld，不是另一套窗口

图标目录（可选，独立构建，不在 workspace members）
    nana-icons-tabler    Tabler outline 全量 `Icon` 常量（生成物，见
                          scripts/generate_tabler_catalog.py）。应用直用常量，
                          链接器剔除未引用图标；不属于产品绘制路径
```

依赖方向：`nana-ui`（适配器 + painter）→ `nana-ui-runtime` 与 `nana-ui-scene`；`nana-ui-scene` → `nana-ui-runtime`。`SceneWgpuPainter` 在 `nana-ui` 里注入宿主 Window / Surface / Device / Queue。`scripts/check-engine-boundary.py` 保持 Runtime / Scene 对绘制后端中立。

产品路径：

```text
nana_ui::runtime → UiWorld → ExtractedNode → UiScene → SceneWgpuPainter
```

`component-gallery` 是独立 Demo crate：分类导航和示例状态不属于 `nana-ui` 公共 API。

## 所有权

| 对象 | 所有者 |
| --- | --- |
| Window、Surface、Device、Queue | 宿主（`run_runtime` 或应用的 `HostedGpuContext`） |
| 业务状态、配置盘、Region / pane **内容** | 应用 |
| 树、样式、未滚动布局、命中、焦点、IME、无障碍 | `UiWorld` |
| 绘制图 | `UiScene` |
| 系统材质与标题栏 chrome | `nana-window` |
| Workspace 尺寸 / 折叠 | `WorkspaceModel`（`WorkspaceController` 只做指针与时钟转换） |
| Dock 树 | Runtime `DockWorkspace`（`nana_ui::dock::*` 是宿主适配器） |

GPU 主版本锁定 workspace `wgpu = "30.0.0"`，依赖图里只有一个主版本。禁止第二套 Device / Queue，禁止正式路径 CPU 回读。

## 三种输入，一棵树

Rust 控件、Vue HTML 1:1 控件 / `nana-*`、以及 Vue 的 HTML/CSS 子集，都写入同一 Style Model（Tokens + Semantics + Layout），再进入同一 `UiWorld`。它们是三种输入合同，不是三个运行时。

Vue 的 DOM/CSS facade **不**复制树拓扑。host op 进待提交队列，`flush_host_frame` 才 commit。`event_flags` 权威是 `UiWorld` 的 `EventListeners`；GPU slot 权威是 Runtime `CustomRenderNode`。JS 查询用的盒子是绘制阶段投影，滚动不写回 Runtime `LayoutBox`。`PendingHostOps` 镜像、`WidgetKind`、`ComponentSupport` 都不是第二套实例化 ABI。

内建与插件控件走同一份 `ComponentRegistry` / `register_component`。`NativeComponentRegistry` 只服务 JS host 命令，不是这条 Runtime ABI。

WebView 不是产品 UI。`nana-ui` 没有 `browser` feature。盒模型对照在 workspace 外的 `tools/css-parity-webview`，不得链进产品 crate。应用内打开网页若落地，仍是 Runtime 节点 + HostTexture，见 [应用内浏览器](gpu.md#应用内浏览器)（未实现）。

## 编译边界

`nana-ui` 默认 feature 为空。消费者显式打开 `hosted`、`bundled-fonts`、各控件族。`gpu` 与字体是独立上层边界，`hosted` / `gpu` 确实控制 `mod` 是否编译。

控件族 feature（`calendar`、`charts`、`controls`、`graph-canvas` 等）是空 feature，只切 `nana-ui` 的再导出与 `ComponentSupport::compiled`。控件本体在 `nana-ui-runtime`，`nana_ui::runtime` 的全量再导出让它们始终可达，也始终参与编译。见 [应用 API](application-api.md)。
