# 架构

这篇给**改 NanaUI 的人**。写应用请看 [框架如何运行](how-it-works.md) 和 [开始](start.md)。

工作区、窗口、GPU、控件的消费合同分别在 [工作区](workspace.md)、[窗口](window.md)、[实时画面](gpu.md)、[控件](components.md)。这里只固定 crate 边界和所有权，避免再长出第二套权威树。

## Crate 边界

```text
应用 / Demo
    │
    ▼
nana-ui                 宿主适配器：run_runtime、控件再导出、SceneWgpuPainter、
    │                   FrameBinding（gpu feature）
    ├── nana-frame-exchange  跨线程最新帧（只依赖 wgpu）。生产端 crate，
    │                       渲染库不必依赖 nana-ui
    ├── nana-ui-runtime 保留树权威（UiWorld）、内建控件、Shell、Workspace、
    │                   Dock、GPU 槽。不依赖 WGPU
    ├── nana-ui-scene   绘制图权威（UiScene）。依赖 runtime，不依赖 WGPU
    ├── nana-ui-core    共享合同：Style Model、主题令牌、WorkspaceModel、几何
    ├── nana-ui-platform  WindowId、输入、IME、剪贴板、显示器与全屏合同
    │                     （与 winit 转换隔离）
    └── nana-window     系统材质、标题栏拖拽 / 客户区 chrome / 缩放；
                        普通控件不得拿窗口句柄

Vue + JS L1/L2（可选宿主）
    nana-ui-vue + nana-js-v8 + nanavue-runtime / nanavue-components
    写入同一棵 UiWorld，不是另一套窗口

文本引擎（Phase 0–8，产品唯一的排版权威）
    nana-text            文本 IR、稳定代际 ID、结构化 diff 与迁移语料；
                         字体层（注册 / 代际 / 匹配 / 变体坐标 / fallback）；
                         shaping（分段 / BiDi / HarfRust / ShapeRun cache）；
                         layout（Label fast path / UAX #14 断行 / 行内视觉序 /
                         行盒度量 / 对齐 / 省略号 / layout cache）；
                         retained layout 句柄（TextLayoutStore）。
                         测量与绘制都经它：nana-ui/text_engine.rs 持有进程
                         唯一的引擎，nana_text.rs（NanaTextShaper）测量，
                         scene_paint/text/（NanaRenderer::text）画。
                         见 [文本引擎](text-engine.md)

图标目录（可选，独立构建，不在 workspace members）
    nana-icons-tabler    Tabler outline 全量 `Icon` 常量（生成物，见
                          scripts/generate_tabler_catalog.py）。应用直用常量，
                          链接器剔除未引用图标；不属于产品绘制路径
```

依赖方向：`nana-ui`（适配器 + painter）→ `nana-ui-runtime` 与 `nana-ui-scene`；`nana-ui-scene` → `nana-ui-runtime`。`SceneWgpuPainter` 在 `nana-ui` 里注入宿主 Window / Surface / Device / Queue。`scripts/check-engine-boundary.py` 保持 Runtime / Scene 对绘制后端中立。

`nana-text` → `nana-ui-core`，且只取排版词汇（变体轴 / kerning / line-break / word-break / text-align / direction / writing-mode / wrap-break / line-height / feature），由同一个脚本按 allowlist 守住。`nana-ui-runtime` 依赖 `nana-text`：保留文本节点的 revision、分级 dirty graph 与 retained `TextLayout` 句柄以它为词汇（#95）。#99 起 Rust、NanaVue 与 Vue/CSS 的文本都由它测量，`NanaRenderer::text` 画 Runtime 保留的那份 layout；cosmic-text 与 cryoglyph（含参照引擎）已从 `Cargo.lock` 删除，脚本按全工作区禁止任何产品 crate 再有通向它们（或它们改名的 fork）的非 dev 边。

产品路径：

```text
nana_ui::runtime → UiWorld → ExtractedNode → UiScene → SceneWgpuPainter
```

动画意图编译为 Motion IR（`nana-ui-core::motion`）。`AnimationSpec` 是这条 IR 的 timing / playback 子集（id、target、start、duration、frame interval、easing、iteration / direction / fill / pause），不是第二套 timeline。

**逻辑值 ≠ 呈现值。** `UiWorld` 是逻辑 / base 权威；瞬时呈现存在 transient `PresentationStore` overlay（按 node + property 查询；`applies=false` 时用 `applied_value()`）。例如 `opacity: 0 → 1` 时逻辑透明度在开始时已是目标值，overlay 负责过渡，而不是每帧把 `UiWorld` 写成 `0.01`、`0.02`。业务读属性得到逻辑状态；绘制、命中、焦点、无障碍在需要时按同一 timestamp 求 presentation。动画完成走 start / completion **deadline**，不靠逐帧 CPU sample 才知道结束。

Compositor-safe 属性按 `AnimationClass::Compositor` 分类，不得实现成每帧改 UiWorld 属性。Compositor track 另编译为 generational `MotionDescriptor` slab：start/retarget/cancel 更新 descriptor，稳态帧只按 timestamp 走同一 `evaluate_track`。产品 present 禁止 CPU readback。Scene layer / GPU Quad 分流见 [`runtime-scene.md`](runtime-scene.md)。

默认执行类由 `AnimatableProperty::animation_class()` 决定，组件不能改 class：

| 属性 | 默认 Class | 执行路径 | fallback / 性能含义 |
| --- | --- | --- | --- |
| `transform` / `opacity` | Compositor | overlay；Quad 走 GPU `evaluate()`；非 Quad 走 CPU overlay | 稳态不写 `UiWorld`；非 Quad 仍是 CPU presentation |
| `clip` / `clip-path` | Compositor | overlay；Painter 目前只把 transform/opacity 的 `motion_id` 交给 shader | 稳态不写 `UiWorld`；clip 呈现仍 CPU overlay |
| `shader-parameter` | Compositor | overlay；需注册 typed codec | 无默认 Quad GPU 路径 |
| `color` / `background` / `blur` / `filter` / `shadow` | Paint | CPU 插值 | 可能每 sample 脏 paint/extract；不是 filter GPU |
| `width` / `height` / `padding` / `margin` | Layout | CPU layout，写 px | 每 sample layout；不要偷成 scale |
| `font-size` / `font-axis` | Layout | 非 compositor（排版 / 绘制也会受影响） | [#85](https://github.com/sena-nana/NanaUI/issues/85) 不强制 GPU |
| `display` | Discrete | snap | 不插值 |

`#8` 的 `animations_considered` / `animation_deadlines_scanned` 稀疏门禁仍有效。compositor-only 稳态结构门禁（无 query 时 UiWorld / layout / style / extract / CPU sample 均为 0）见 [`perf/README.md`](../perf/README.md) 的 `compositor-steady`。开发诊断走 `AnimatableProperty::diagnostic_hint()` 与 `UiWorld::inspect_motion()`；hint 文案以代码为准，文档不硬编码整句，也不写进产品 UI。

FLIP / list-move 是显式策略，不是把 width 动画偷成 scale。

`component-gallery` 是独立 Demo crate：分类导航和示例状态不属于 `nana-ui` 公共 API。

## 所有权

| 对象 | 所有者 |
| --- | --- |
| Window、Surface、Device、Queue | 宿主（`run_runtime` 或应用的 `HostedGpuContext`） |
| 最新帧槽池（`FrameExchange`） | 生产线程；`FrameInbox` / `FrameBinding` 在窗口侧取样 |
| 业务状态、配置盘、Region / pane **内容** | 应用 |
| 树、样式、未滚动布局、命中、焦点、IME、无障碍 | `UiWorld` |
| Motion IR（曲线、timing、属性分类、CPU 求值、presentation overlay、MotionDescriptor slab） | `nana-ui-core::motion`；Runtime `AnimationSpec` 是同一套 timing 子集；L3 `transition` / `Spring::to` / `Timeline` 编译进这条 IR；内建 hover/switch/spinner/surface 不再平行自管时钟；`PresentationStore` 是 IR overlay；descriptor 只覆盖 `AnimationClass::Compositor` |
| 绘制图 | `UiScene`（`CompositorLayer` 持有 presentation 与 `CompositorMotionBinding`；`SceneWgpuPainter` 上传 MotionDescriptor storage + time uniform） |
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
