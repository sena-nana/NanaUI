# 架构合同

本文面向参与 NanaUI 框架开发与维护的贡献者，固定工作区内各 crate 的职责边界与对象所有权，防止演进过程中长出第二棵树、第二套渲染器或第二个 GPU 设备。

- 如果你正在使用 NanaUI 开发桌面应用，请先阅读 [框架如何运行](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/how-it-works.md) 与 [快速开始](https://github.com/sena-nana/NanaUI/blob/main/docs/guide/quick-start.md)。
- 如果你想按模块与源码行号深入学习各子系统的内部实现，请阅读 [架构学习](/learn/) 板块。
- 工作区、窗口、实时画面与内置控件的消费侧合同分别见 [工作区](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/workspace.md)、[窗口](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/window.md)、[实时画面](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/gpu.md) 与 [控件](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/components.md)。

## Crate 边界与依赖方向

```text
应用 / Demo
    │
    ▼
nana-ui                  宿主适配器：run_runtime、控件再导出、SceneWgpuPainter、
    │                    FrameBinding、窗口外输出 ExternalSurface（gpu feature）
    ├── nana-gpu         GPU 后端合同：GpuContext（设备、代次、能力、丢失状态、纹理与提交顺序）、
    │                    FrameContext（独占 encoder 的单帧上下文）、GpuTexture。
    │                    WGPU 是唯一后端，wgpu-interop 是显式逃生口
    ├── nana-frame-exchange  跨线程最新帧交换（建在 nana-gpu 之上并再导出合同类型；
    │                        生产端渲染库只需依赖它，不必依赖 nana-ui）
    ├── nana-ui-runtime  保留树权威（UiWorld）、内置控件、布局引擎、输入路由、
    │                    Shell、Workspace、Dock、AreaLayout、GPU 槽声明（不依赖 WGPU）
    ├── nana-ui-scene    绘制图权威（UiScene）与帧图（FramePlan）（依赖 runtime，不依赖 WGPU）
    ├── nana-ui-charts   图表模型：ChartOption、刻度、布局、命中与 GPU 标记数组（仅依赖 nana-ui-core）
    ├── nana-ui-diagram  Mermaid 图表：解析、布局与主题无关的矢量显示列表（依赖 nana-ui-core 与 nana-ui-charts 的刻度）
    ├── nana-ui-core     共享基础合同：Style Model、主题令牌、WorkspaceModel、Motion IR、几何
    ├── nana-ui-platform 平台中立窗口合同（WindowId、描述符、事件、命令、显示器）并再导出输入合同
    └── nana-window      原生窗口句柄、系统材质、标题栏拖拽、客户区 chrome、缩放与全屏

文本引擎（产品唯一的排版权威）
    nana-text            文本 IR、稳定代际 ID、字体层（注册/代际/匹配/变体轴/覆盖率回退）、
                         整形（分段/BiDi/HarfRust/ShapeCache）、行布局（单行快路径/UAX #14 断行/
                         视觉重排/行盒度量/对齐/省略号/LayoutCache）与保留布局句柄（TextLayoutStore）。
                         测量与绘制共用进程内唯一引擎（nana-ui/text_engine.rs）：
                         nana_text.rs（NanaTextShaper）负责测量，scene_paint/text/ 负责绘制。
                         详见 [文本引擎](text-engine.md)

图标目录（可选，独立构建，不在 workspace members 中）
    nana-icons-tabler    Tabler outline 全量 Icon 常量（由 scripts/generate_tabler_catalog.py 生成）。
                         应用直接引用常量，未引用图标由链接器剔除，不属于产品绘制核
```

主线依赖单向流动：`nana-ui` 依赖 `nana-ui-scene` 与 `nana-ui-runtime`；`nana-ui-scene` 依赖 `nana-ui-runtime`；三者均依赖仅构建在 `nana-ui-core` 之上的 `nana-ui-charts`。`nana-ui-runtime` 的 `rich-text` 控件族另外引入 `nana-ui-diagram`，Scene 只看到它录成的 `PaintRecording`。`nana-ui` 与 `nana-frame-exchange` 依赖 `nana-gpu`。`SceneWgpuPainter` 位于 `nana-ui` 内，建立在宿主的 `GpuContext` 之上，每一帧画进宿主的 `FrameContext`。

### 无窗口输出与边界守护脚本

操作系统窗口并非输出 API 的唯一语义根：Window 与无窗口 `PresentationDriver` 共用真实的 presentation 编排。普通 Window-only 直接画进 Surface；`RenderTargetPlanner` 在消费者或显示需求变化时缓存兼容分组，兼容 Window/外部消费方共享一次 canonical 渲染，仅最终 transfer 独立计数；像素密度、文本 AA、HDR 等不兼容需求明确分组。`ExternalSurface` 为无窗口、嵌套与无头消费方保留有界 GPU 槽位和真实提交租约：静态生产端跳过渲染和提交，消费方只采样已完成版本。整个路径仍只有一份 `UiWorld → UiScene` 和同设备 `SceneWgpuPainter`，产品不提供 CPU 回读 fallback（详见 [窗口外输出](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/output.md)）。

`scripts/check-engine-boundary.py` 在 CI 中静态守卫以下边界：

1. **后端中立与 `wgpu` 签名隔离**：保持 `nana-ui-runtime` 与 `nana-ui-scene` 对图形后端中立；`nana-gpu`、`nana-frame-exchange`、`nana-ui`、`nana-ui-devtools` 的公开签名只有在 `wgpu-interop` feature 下才允许出现 `wgpu::*`；`nana_gpu::__framework` 仅供这些框架 crate 自身源码内部使用（详见 [实时画面](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/gpu.md#gpu-合同与-wgpu-逃生口)）。
2. **排版词汇白名单**：`nana-text` 仅从 `nana-ui-core` 导入白名单内的排版与动态布局代价词汇（变体轴、字距、断行、对齐、方向、书写模式、行高、中日文标点挤压与 `dynamic_layout`）。
3. **单一文本引擎**：`nana-ui-runtime` 的保留文本节点修订号、分级脏图及 `TextLayout` 句柄均以 `nana-text` 为唯一权威；`cosmic-text`、`cryoglyph` 与 `glyphon`（含改名 fork）已从 `Cargo.lock` 彻底移除，严禁任何工作区成员引入通向它们的非 dev 依赖边。

## 核心产品路径与 Motion IR 合同

产品渲染路径只有一条：

```text
nana_ui::runtime → UiWorld → ExtractedNode → UiScene → SceneWgpuPainter
```

### 逻辑状态与呈现 Overlay 分离

所有动画意图统一编译为 `nana-ui-core::motion`（Motion IR），Runtime 的 `AnimationSpec`（含 id、target、start、duration、frame_interval、easing、iteration、direction、fill、pause）是同一套时序与播放子集，而非第二套时间线：

- **`UiWorld` 是逻辑与基础样式的唯一权威**，过渡期间的瞬时呈现值保存在瞬态的 `PresentationStore` overlay 中，按「节点 + 属性」查询（`applies = false` 时使用 `applied_value()`）。例如 `opacity: 0 → 1` 启动时，节点的逻辑透明度立即是目标值 `1`，过渡过程由 overlay 承载，绝不每帧往 `UiWorld` 写 `0.01`、`0.02`。业务代码读取属性时拿到的是逻辑状态；绘制、命中测试、焦点与无障碍在需要时按同一时间戳求取呈现值。
- **完成事件与末值保持（Hold）**：动画完成由启动与结束的 **deadline** 驱动，不靠逐帧 CPU 采样判断结束。带 `fill: forwards` 的轨道结束后，末值作为 **hold** 留在 overlay 上：
  - 在 Rust L3 的 `transition()`、`motion(Spring)`、`Timeline`（`MotionLayer::Runtime`）中，hold 保持到该属性下一次被写入**不同的逻辑值**为止；组件重新投影写回相同的值不算新写入，不会打断 hold。
  - 任何 hold 均可通过同 ID 重新启动、`StopAnimation`（对已结束的 hold 同样生效）或移除节点来清除。CSS `transition` 不留 hold，因为级联样式本身就是终点。
- **取值校验与越界钳制**：轨道的目标值与关键帧按样式规则在提交时严格校验，越界或非有限值直接返回 `InvalidAnimation`（例如 `opacity` 超出 `0..=1` 或 `width` 为负数，框架不会静默替调用方钳制）。起点值只要求有限（因为被打断的弹簧或回弹轨道可能正处于越界区间）。起点与终点之间的采样过程不钳制，以保留回弹曲线与弹簧的有意过冲，最终由消费端钳制到可呈现范围（CPU/GPU 合成器将 `opacity` 钳到 `0..=1`，`width`/`height`/`padding` 下限为 `0`，字体变体轴由 `nana-text` 钳到字体自身轴范围）。
- **同属性多轨道优先级**：同一属性同时存在多条活动轨道时，依次按 `MotionLayer`（`CSS transition > Runtime > CSS animation`，与 CSS 级联规范一致）→ 启动时刻（`start`）→ 轨道 `id` 决出唯一胜出者，不做多轨道混合。

### 动画属性执行分类

`AnimationClass::Compositor` 属性严禁实现为每帧修改 `UiWorld`。合成器轨道额外编译进代际化的 `MotionDescriptor` 表（仅在启动、retarget、取消时更新描述符），稳态帧只按时间戳执行 `evaluate_track`。默认分类由 `AnimatableProperty::animation_class()` 固定：

| 属性 | 默认分类 | 执行路径 | 回退与性能边界 |
| --- | --- | --- | --- |
| `transform` / `opacity` | `Compositor` | 呈现 overlay；Quad 走 GPU `evaluate()`，非 Quad 走 CPU overlay | 稳态不写 `UiWorld`；非 Quad 仍为 CPU 呈现 |
| `clip` / `clip-path` | `Compositor` | 呈现 overlay；Painter 当前仅将 transform / opacity 的 `motion_id` 传给着色器 | 稳态不写 `UiWorld`；裁剪呈现走 CPU overlay |
| `shader-parameter` | `Compositor` | 呈现 overlay（需注册类型化编解码器） | 无默认 Quad GPU 路径 |
| `color` / `background` / `blur` / `filter` / `shadow` | `Paint` | CPU 插值 | 每次采样可能标脏 paint / extract；无 GPU 滤镜轨道 |
| `width` / `height` / `padding` / `margin` | `Layout` | CPU 布局求值，写回像素尺寸 | 每次采样触发局部布局；严禁偷换成 `scale` 变换 |
| `font-size` | `Layout` | 非合成器路径（影响排版与绘制） | 不强制走 GPU |
| `font-variation-settings`（`FontAxis(tag)`，每轴一条轨道） | `Layout` | 轨道存于 `PresentationStore`，样式解析时叠入计算样式的 `font_variations` 并向下继承（显式声明该轴的子树不受影响）；按 `SHAPE_STYLE` 脏图触发重新整形、排版与栅格化（采样值未变则零工作） | 每次有效采样均为真实字形实例，严禁替换成几何缩放（[#85](https://github.com/sena-nana/NanaUI/issues/85)）。字体缺失的轴照常计时但不生效，通过 `inspect_motion` 的 `ineffective_reason` 与 `nana.css` 警告上报，不映射为 `wght` |
| `display` | `Discrete` | 离散跳变（snap） | 不做插值 |

合成器稳态性能门禁（无查询时 `UiWorld`、布局、样式、抽取与 CPU 采样计数均为 0）定义在 [`perf/README.md`](https://github.com/sena-nana/NanaUI/blob/main/perf/README.md) 的 `compositor-steady`。开发期诊断通过 `AnimatableProperty::diagnostic_hint()` 与 `UiWorld::inspect_motion()` 获取，提示文案不写进产品 UI。

## 全局对象所有权表

| 对象 / 状态 | 权威所有者 | 边界说明 |
| --- | --- | --- |
| `Window`、`Surface` | 宿主（`run_runtime` 或 `EmbeddedRuntime` 的嵌入方） | 框架按窗口借用，普通控件不可触碰原生句柄 |
| 设备：`GpuContext`（代次、能力、丢失状态、提交顺序） | 宿主 | 粘性丢失记在 `GpuContext::is_lost()`；替换设备即创建新的 `GpuContext`（新 `DeviceGeneration`）。依赖图里只有 NanaUI 自己的 wgpu fork（`third_party/wgpu`，沿用上游包名与版本 30.0.1） 这一份 wgpu |
| 单帧：`FrameContext`（encoder、提交与丢弃） | 宿主 | Painter 与资源生产者只往里录制命令；帧被丢弃（drop/discard）时 Painter 登记的保留写入自动回滚 |
| 跨线程帧槽池（`FrameExchange`） | 生产线程 | 窗口侧通过 `FrameInbox` / `FrameBinding` 采样；已声明的色彩空间元数据随租约（lease）传递，消费端不得将 PQ/HLG/P3 未经转换当作 sRGB 或 linear scRGB |
| 业务状态、应用配置、区域与窗格**内容** | 应用 | 框架不持有业务路由或领域数据 |
| 可选的窗口 / Dock 视图状态恢复 | `ViewStateStore` | 应用指定稳定的 scope / key |
| 持久化落盘（`PersistenceCoordinator`） | 宿主后台线程合并写盘 | 程序经 `RuntimeProgramContext::store()` 拿到 `ProgramStore`：`flush` 仅请求尽快落盘而不阻塞等待，`clear` 仅清理应用命名空间而不触碰 `nana.*` 框架键；关闭时宿主执行至多 2 s 的有界最终 flush（超时记录 `persistence.flush_timed_out`） |
| 保留树、样式、未滚动布局、命中、焦点、IME、无障碍 | `UiWorld`（`nana-ui-runtime`） | 唯一 UI 树权威 |
| Motion IR（曲线、时序、分类、求值、`PresentationStore`、`MotionDescriptor` 表） | `nana-ui-core::motion` | 内置控件动效、L3 过渡与 CSS 动效共用同一条时间线 |
| 绘制图与合成层 | `UiScene`（`nana-ui-scene`） | `CompositorLayer` 持有呈现与 `CompositorMotionBinding`；`SceneWgpuPainter` 上传描述符存储缓冲与时间 uniform |
| 系统材质与标题栏原生 chrome | `nana-window` | 失败返回实际生效结果或显式回退 |
| Workspace 区域尺寸与折叠过渡 | `WorkspaceModel`（`nana-ui-core`） | `WorkspaceController` 仅做指针与时钟转换 |
| Dock 停靠树 | Runtime `DockWorkspace` | `nana_ui::dock::*` 仅是宿主适配器 |

## 输入入口与宿主内容边界

Rust 声明式视图（`nana_ui::runtime` 构建器与 `view!`）和 `.vue` 方言（构建期由 `nana-ui-sfc` 编译成 Rust）通向同一份 Style Model（Tokens + Semantics + Layout）和同一棵 `UiWorld`，而非两个独立运行时：

- **单一组件注册 ABI**：内置控件与扩展控件共用同一份 `ComponentRegistry` 与 `register_component`。`ComponentSupport` 不是实例化 ABI。
- **`BrowserView` 与 `WebSurface` 边界**：
  - `BrowserView` 是显式的宿主原生子视图例外：Runtime 节点仍拥有布局、无障碍、可见性与生命周期锚点，宿主负责创建并管理原生 `WKWebView` 子视图（当前仅支持 macOS，Windows 与 Linux 返回明确的 `Unsupported`，不创建占位浏览器）。它不等同于 `GpuTextureView`，不进 Runtime/Scene 离屏截图；遇到非矩形裁剪、透明度/滤镜组或被后续 Runtime 内容遮挡时，由宿主自动隐藏。
  - 无头 `WebSurface` 是宿主内容生产者：页面不进入任何窗口视图树，捕获帧经应用的 sink 离开框架（平台实现在 `nana-window`，支持 macOS `WKWebView` 与 Windows WebView2，`nana-ui` 按请求对账实例）。
  - 盒模型对照工具 `tools/css-parity-webview` 独立于 workspace 之外，严禁链接进任何产品 crate。

## Cargo Feature 与编译边界

`nana-ui` 默认不开启任何 feature（`default = []`）：打开桌面窗口必须显式启用 `hosted`（隐含 `gpu` 并仅拉起当前平台主图形后端）；需要全部跨平台标准后端时加 `wgpu-backends`，或直接启用 `full`。它绝不会创建第二套宿主或 GPU 所有者。

控件族 feature（`calendar`、`charts`、`controls`、`graph-canvas`、`image-viewer`、`rich-text`）同步控制 `nana-ui-runtime` 与 `nana-ui-scene` 的条件编译，并决定 `ComponentSupport::compiled`。未启用的控件族及其专属依赖不参与编译，`nana_ui::runtime` 仅导出当前构建实际编入的控件（完整对照表见 [应用 API](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/application-api.md)）。`component-gallery` 是独立的示例 crate，其分类导航与演示状态不属于 `nana-ui` 的公共 API。
