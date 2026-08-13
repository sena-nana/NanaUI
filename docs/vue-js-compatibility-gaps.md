# NanaUI Vue+JS 兼容层能力缺口

## 0. 当前复查结果（2026-08-13）

本文第 3、4 节保留实施前基线与目标；实际完成状态和剩余缺口以本节为准。业务接口仍由应用
定义，未进入 NanaUI。

| 能力 | 状态 | 已落地边界 |
|---|---|---|
| `NUI-ARCH-01` | 已关闭 | `VueHostedRuntime` 只持有一个 JS engine，并让全部窗口共享业务 JS、localStorage、Canvas、WebGPU 与 HostTexture。发布程序在应用构建中选择 `V8Engine`；框架保留泛型 engine 仅作为测试注入点，不会产生第二个发布运行时。 |
| `NUI-NATIVE-COMPONENT-01` | 已关闭 | 注册、结构化 props、事件、slot、命令、稳定节点状态和卸载协议已接通；渲染错误会可靠重试并进入 Vue 本地 `error` 与全局 `Nana.components.onError`。真实 Vue SFC 验收已覆盖 props、事件、slot、命令、状态保持、卸载与重挂载，Windows 窗口已验证 Vue/Iced 混合显示。 |
| `NUI-GPU-01` | 已关闭 | `<nana-gpu>` 已绑定真实 `HostTexture`，复用宿主 Device/Queue，支持 generation、版本重绘、圆角/布局裁剪、父级透明度、仿射变换和输入命中。 |
| `NUI-CANVAS-01` | 已关闭（明确子集） | Canvas2D 使用稳定 WGPU texture，并按实际脏矩形上传；变换后的填充脏区已正确换算，复杂描边保守上传整面。未承诺的完整浏览器 Canvas2D 不计为缺口。 |
| `NUI-WEBGPU-01` | 已关闭（明确子集） | `navigator.gpu` 复用宿主 WGPU；承诺的 buffer/texture/binding/shader/render/compute/queue/canvas 子集已接通，设备恢复会使旧 generation 失效。正常提交由稳定事件循环 deadline 非阻塞推进；未承诺的 feature、format、descriptor 和异步 API 明确抛出 `NotSupportedError`，不伪造成功。 |
| `NUI-BRIDGE-01` | 已关闭 | Promise 调用、事件、取消/超时、ArrayBuffer/TypedArray/DataView、BigInt 句柄、上下文清理与稳定错误均已接入 V8。Fetch 请求和响应正文直接使用二进制 HostValue，不再经过 JSON 数组或 Base64；Abort 会取消排队请求、停止 DNS 等待并关闭已连接 socket，可主动打断响应头或正文读取。 |
| `NUI-RESOURCE-01` | 已关闭 | Vue `<img src>` 支持异步加载/解码、换源取消、load/error、PNG/JPEG/GIF/WebP 与 SVG；资源节点遵守 CSS 尺寸并只应用一次父级透明度。Vue 子树卸载释放自有 Image/Canvas；Blob URL 以引用计数持有资源且每次创建返回独立 URL。V8 HostResource 对象带 guaranteed weak finalizer，同时保留 `close()`/`Nana.resources.release()` 确定释放。 |
| `NUI-INPUT-01` | 代码完成/待硬件 | 混合树优先分发、`preventDefault`、局部/屏幕坐标、多键 mouse buttons、pointer capture、enter/leave、touch cancel 已接通；Windows `WM_POINTER` 已提供 pen、pressure、tangentialPressure、tilt、twist、主指针和可靠 cancel，并作为标准 JS PointerEvent 顶层字段暴露。外部文件拖放按最终命中节点产生 `dragenter`/`dragover`/`dragleave`/`drop`，同一次系统多选拖放在事件批次结束时聚合为一个 FileList-like `dataTransfer`。验收小游戏已直接使用 pointer capture 和笔字段驱动状态，仍需真实笔设备验收。 |
| `NUI-IME-01` | 代码完成/待实机 | winit 桌面 IME 启停、候选区域、preedit/commit、多窗口焦点路由和去重已接通；中文候选框仍需 Windows 实机验收。 |
| `NUI-WINDOW-01` | 代码完成/待透明验收 | `Nana.windows` 可创建窗口并控制 bounds、DPI、全屏、最小/最大化、置顶；晚创建窗口继承同一 WebGPU/Canvas GPU runtime、应用样式表和 Host API，关闭时释放窗口级状态。真实 Windows 双窗口已验证 Vue 内容、Canvas、WebGPU 和 Iced 同时显示；Windows 原生 owner/disable 路径已验证模态期间父窗口禁用且关闭后恢复。验收窗口跨三个真实显示器移动时，HWND DPI 从 96 切换为 120，800×560 逻辑窗口相应变为 1000×700 物理像素。仍待透明窗口 Alpha 验收。 |
| `NUI-COMPOSE-01` | 代码完成/待透明验收 | 普通控件、Canvas、WebGPU、HostTexture 与原生组件进入同一 Iced/WGPU 顺序、裁剪和命中树；完整常用 2D 仿射变换同步作用于绘制、overlay、命中、局部坐标及 Iced Operation/AccessKit 几何，父级 opacity（含 overlay）由复用离屏纹理整体合成。Windows 窗口已验证 Vue/Iced/Canvas/WebGPU 交错显示，像素测试已覆盖旋转和重叠透明度；仅剩桌面透明窗口 Alpha 的实际合成证据。 |
| `NUI-DIAG-01` | 已关闭 | V8 Inspector/CDP、异常/Promise rejection/Vue handler、隐私化 Host trace、资源/窗口/帧/Device Lost 诊断。详见 `vue-js-diagnostics.md`。 |

### 仍需关闭的 NanaUI 缺口

1. **P0：目标硬件上的 Windows 发布验收。** `vue-hosted-acceptance` 已同时提供纯 Vue+JS
   模式和注册 Iced 组件模式，覆盖完整窗口、弹层、输入、图片、Canvas 动画、WebGPU、Vue
   slot 与辅助 Vue 窗口。当前 Windows 实机已验证双窗口实际绘制、单 V8 辅助 document 挂载、
   Canvas rAF、WebGPU completion、SVG `<img>` 的正确尺寸绘制、Iced 混合显示，以及模态窗口
   对父窗口的原生禁用/恢复，以及三个显示器间 96→120 DPI 切换；仍缺中文 IME 候选框交互、
   真实笔输入和透明窗口 Alpha 的人工验收证据。

代码层 P1 已关闭：外部文件拖放现在进入 Vue 事件树；仿射包装通过 Operation proxy 转换
容器、滚动、焦点、文本输入、文本和自定义节点的报告边界，同时保持控件状态与原布局关联。
系统多文件拖放利用 winit 同一原生事件周期的 `AboutToWait` 作为批次边界，保序去重后只向
Vue 发出一个 `drop`，不依赖猜测性的毫秒延时。

不列为 NanaUI 缺口：Electron IPC 名称、账号/角色/对话/存档、服务端协议、业务音视频、
Live2D 模型与小游戏规则；它们应通过应用 Host API 或注册原生组件暴露给 Vue。

兼容边界：NanaUI 不是完整浏览器。普通 Vue 节点支持 translate、非等比/负 scale、rotate、
skew 和 matrix 的常用 2D 仿射组合，默认以节点中心为 transform origin，并同步绘制、overlay、
逆变换输入命中与 `getBoundingClientRect`；自定义 `transform-origin` 尚不属于 CSS 子集，可用
显式平移矩阵或包装节点表达。WebGPU 暴露迁移 3D 内容所需的明确子集，而不是 Chromium 的
完整 GPUWeb 实现；`mapAsync`、shader compilation info 和异步 pipeline 编译等未承诺能力会
明确拒绝，不计为缺口；不提供 WebGL。

文件拖放中的文件是包含 `name`、绝对 `path`、`size`、`type` 和 `lastModified` 的 NanaUI
描述符，并提供 `dataTransfer.files/items.item()` 兼容访问；读取内容和权限仍通过应用 Host API，
不伪装成已实现完整浏览器 `File`/沙箱文件系统。

桌面 IME 候选框位置、真实笔输入和透明窗口仍需在对应硬件环境做交互验收；静态与
行为测试只证明事件、状态和宿主调用链已经接通。rAF 使用注册时固定的约 16 ms deadline，
避免事件循环重复查询时永远错过回调或退化为无上限忙循环。

Fetch 使用每请求可取消的 DNS/transport 链；取消会关闭登记的 TCP socket，从而中断 TLS/HTTP
等待。该实现依赖 ureq 的 unversioned transport 接口，因此 workspace 将 ureq 精确锁定到
`3.4.0`，升级时必须重新验证 connector、resolver、代理和取消行为。

### Windows 人工验收入口

```powershell
cargo run -p vue-hosted-acceptance --locked -- --input-probe
cargo run -p vue-hosted-acceptance --locked -- --hybrid --windows
cargo run -p vue-hosted-acceptance --locked -- --alpha-probe
```

`--input-probe` 仅让真实 Vue 输入框在挂载后获得焦点，不注入、伪造或代替 IME 事件；
验收者需要实际选择中文输入法，确认预编辑、候选框位置和最终文本。`--hybrid --windows`
启动注册 Iced 组件与透明模态辅助窗口，用于检查 Vue/Iced 交错合成和桌面 Alpha；
`--alpha-probe` 则以透明主窗口启动，并在标准错误输出中报告 WGPU 从当前原生 surface
能力选中的真实 alpha mode。该输出能验证 NanaUI 透明渲染链，但不替代桌面合成器的视觉验收。
当前 Windows/WGPU 实际运行输出为 `PreMultiplied`，证明发布路径没有退化为
`Opaque`；尚缺的只是不受当前桌面捕获策略干扰的底层桌面像素合成证据。
真实笔验收必须使用物理笔在 Canvas 上交互，不以合成 pointer 事件代替硬件证据。

## 1. 目标架构

NanaUI Vue+JS 兼容层的目标不是复刻浏览器，而是提供可承载完整 Vue+JS 应用的原生运行与显示基础：

- 使用单个 V8 isolate/context 执行业务 JS、Vue UI 和小游戏逻辑。
- 只使用 Vue+JS 与 NanaUI 内置接口即可编写完整产品界面、弹窗、窗口内容和小游戏。
- Rust 可控制 UI 或提供高性能 Iced 组件；供 Vue 使用时必须注册为稳定 Vue 组件或 JS 接口。
- 不依赖 Electron、Chromium 或 WebView。
- 允许 Vue UI 与 WGPU、Live2D、视频等原生内容混合显示。
- 原生能力必须在 JS+Vue 层暴露为稳定、通用的兼容接口。
- 提供常用 Canvas2D 兼容能力。
- 不实现 WebGL；3D 内容使用基于现有 wgpu 的 WebGPU 风格 JS API。

该目标适用于 `nana-ui-vue`、`nanavue-runtime`、`nana-ui-web-api` 等兼容层 crate，
不改变 `nana-ui` 核心作为通用 Iced/WGPU 组件和绘制基础的职责。

## 2. 缺口判定边界

满足以下条件的能力才属于 NanaUI 缺口：

- 与具体产品业务无关。
- 可以被多个 Vue+JS 应用复用。
- 属于 JS 运行环境、Vue 渲染、窗口、输入、图形或资源基础设施。
- 即使底层使用更优的原生实现，仍需要向 JS 暴露兼容接口。

具体业务状态机、产品数据结构、第三方服务和厂商 SDK 不属于 NanaUI 缺口。

## 3. 当前已有基础

NanaUI 当前已经具备：

- 基于 rusty_v8 的 JS 执行环境。
- Vue custom renderer 与 NanaUI/Iced 组件树。
- 基础布局、样式和控件能力。
- 基础窗口管理能力。
- WGPU Device/Queue 和 Surface 的宿主管理。
- `HostTexture`、`GpuTextureView` 等宿主纹理基础。
- 定时器、Fetch 和有限的 Web API。
- JS 到 Rust 的 Host API 调用机制。

这些能力可以继续作为兼容层基础，但尚不足以完整替代 Electron 中的 Vue+DOM+Canvas
运行环境。

## 4. NanaUI 能力缺口

### NUI-ARCH-01：Vue 产品层权威边界

**实施前状态**

现有架构仍倾向于由 NanaUI/Rust 组件定义产品界面，Vue 被描述为可选状态或命令层。

**目标能力**

- 产品 UI 树、交互状态和页面组合由 Vue+JS 定义。
- NanaUI 组件只能提供通用控件与基础能力。
- Rust 层不得包含具体产品页面、业务弹窗或小游戏规则。
- Vue custom renderer 必须成为正式、稳定的产品 UI 接口。

**关闭条件**

可以只通过 Vue 代码创建完整窗口、页面、弹窗和混合 GPU 内容，不需要新增产品专用
Rust 组件。

### NUI-NATIVE-COMPONENT-01：Rust/Iced 组件的 Vue 扩展协议

**已实现能力**

- `NativeComponentRegistry` 在应用启动时注册描述符与工厂，不扩展固定 `WidgetKind`。
- 名称规范化为 `nana-{name}`；重复注册、未知组件、非法 props 均返回结构化错误。
- props 以 `HostValue` 增量传递；组件声明事件和命令白名单。
- Vue 默认 slot 子树先构建为 Iced `Element`，再交给原生组件组合。
- 原生事件通过 `BridgeEvent::Native` 进入 Vue 标准 `@event` 分发。
- `Nana.components.call(element, command, args)` 返回 Promise；未知命令拒绝。
- 组件和 Vue 节点共用父级约束、裁剪、透明度、变换、层级、焦点与命中树，不创建悬浮窗口。
- 实例状态以稳定节点 ID 关联；节点卸载、窗口关闭及运行时销毁时调用统一 unmount。
- 组件 panic/渲染错误被隔离为可诊断错误，不能退出渲染循环。

**边界**

原生组件为启动时编译注册，不支持运行时加载动态库。业务组件的模型、协议和状态机由
应用实现；NanaUI 只负责通用注册、props、事件、slot、命令、布局和生命周期协议。

### NUI-GPU-01：真实 GPU 内容节点

**实施前状态**

`<nana-gpu>` 只保存 GPU slot 元数据，最终仍显示占位内容，没有绑定真实
`HostTexture`。

**目标接口**

```vue
<nana-gpu
  :source="textureHandle"
  fit="contain"
  :interactive="true"
/>
```

纹理句柄至少包含：

```ts
interface NanaTextureHandle {
  id: bigint
  generation: number
  version: number
  width: number
  height: number
  alphaMode: "premultiplied" | "opaque"
}
```

需要支持：

- 与 Vue 节点一致的尺寸、变换、裁剪、透明度和层级。
- DPI 缩放。
- 预乘 Alpha 混合。
- 纹理内容更新触发重绘。
- Device/Surface 重建后的 generation 失效。
- 同一 WGPU Device/Queue 内的零拷贝或低拷贝组合。
- 输入事件按照最终布局位置命中。

**关闭条件**

Vue 页面可以显示真实宿主纹理，并与普通 Vue 控件正确交叠、裁剪和交互。

### NUI-CANVAS-01：Canvas2D 兼容层

**实施前状态**

NanaUI 没有 `HTMLCanvasElement` 或 `CanvasRenderingContext2D` 实现。

**目标接口**

在 JS 中提供常用 Canvas2D API：

```ts
const canvas = document.createElement("canvas")
const ctx = canvas.getContext("2d")
```

必须覆盖：

- `width`、`height` 和 DPI 行为。
- `clearRect`、`fillRect`、`strokeRect`。
- `beginPath`、`closePath`、`moveTo`、`lineTo`。
- `rect`、`arc`、`ellipse`、`quadraticCurveTo`。
- `fill`、`stroke`、`clip`。
- `fillText`、`strokeText`、`measureText`。
- 线宽、线帽、线连接和虚线。
- 纯色、线性渐变、径向渐变和图案。
- `save`、`restore`、`translate`、`rotate`、`scale`、`transform`。
- `globalAlpha` 和常用 `globalCompositeOperation`，包括 `destination-out`。
- `drawImage`，支持图像、Canvas、ImageBitmap 和媒体帧。
- `createImageData`、`getImageData`、`putImageData`。
- `toBlob` 和 `toDataURL`。
- Canvas 对应的指针事件和 pointer capture。

Canvas 应作为独立兼容服务实现，不把任意 CPU raster 绘制重新引入 NanaUI 通用
组件树。需要同步像素访问时，可维护 CPU 可寻址的 Canvas 表面并向 GPU 上传脏区，
不依赖产品呈现纹理的 GPU readback。

**关闭条件**

现有依赖常用 Canvas2D 的 Vue 组件和小游戏可以在不修改调用方式或只做少量兼容调整
的情况下运行。

### NUI-WEBGPU-01：WebGPU 风格 JS API

**实施前状态**

NanaUI 已使用 wgpu，但没有向 JS 暴露 WebGPU API。

**目标接口**

通过 `navigator.gpu` 暴露 WebGPU 风格接口：

```ts
const adapter = await navigator.gpu.requestAdapter()
const device = await adapter.requestDevice()
const context = canvas.getContext("webgpu")
```

至少支持：

- Adapter 和 Device 获取。
- `GPUBuffer`、`GPUTexture`、`GPUTextureView`。
- ShaderModule。
- BindGroup 和 BindGroupLayout。
- RenderPipeline 和 ComputePipeline。
- CommandEncoder、RenderPass、ComputePass。
- Queue 写入与提交。
- Canvas context 配置和呈现。
- 资源销毁、错误传播和 Device Lost。
- 与宿主纹理节点互操作。

JS WebGPU 必须复用 NanaUI 已有的 WGPU 实例、Adapter、Device 和 Queue，不创建第二套
图形设备。不提供 WebGL 模拟层。

**关闭条件**

Vue+JS 可以直接创建和更新 GPU 内容，并通过 Canvas 或 `<nana-gpu>` 显示到 NanaUI
窗口。

### NUI-BRIDGE-01：异步与二进制 Host Bridge

**实施前状态**

现有 Host API 主要是同步调用，并通过 JSON 传递值，无法高效传输图像、音频和 GPU
资源。

**目标能力**

- Promise 异步调用。
- Host 主动向 JS 发布事件。
- 请求取消和超时。
- `ArrayBuffer`、TypedArray 和 DataView。
- 二进制数据所有权和生命周期管理。
- 大型资源句柄，避免反复 JSON 序列化或内存复制。
- Rust 错误转换为稳定的 JS Error。
- V8 context 销毁时自动释放关联资源。

建议通用形式：

```ts
Nana.host.invoke(method, params, options): Promise<unknown>
Nana.host.on(event, listener): () => void
Nana.resources.release(handle): void
```

**关闭条件**

图像、音频、文件和媒体帧可以通过二进制对象或资源句柄传输，不需要 Base64 或大型
JSON 数组。

### NUI-RESOURCE-01：图像与媒体资源对象

**实施前状态**

缺少 Canvas 和 Vue 组件可共同使用的浏览器式资源对象。

**目标能力**

提供：

- `Image`。
- `ImageBitmap`。
- `ImageData`。
- `Blob`。
- Object URL。
- `createImageBitmap`。
- 二进制图片解码。
- 外部纹理和媒体帧句柄。
- 资源加载、失败、取消和释放事件。
- Canvas、Vue 图片组件和 GPU 节点之间的资源互操作。

媒体采集、视频解码或 RTC 引擎本身不属于 NanaUI；NanaUI 只负责接收通用媒体帧并将其
暴露给 JS、Canvas 和 GPU 节点。

**关闭条件**

同一图像或媒体帧可以在 Vue 图片组件、Canvas2D 和 GPU 节点之间复用，不需要产品
专用转换代码。

### NUI-INPUT-01：完整输入事件模型

**实施前状态**

当前事件桥只覆盖简化的点击、滚轮和键盘事件，缺少完整指针语义。

**目标能力**

支持：

- `pointerdown`、`pointermove`、`pointerup`、`pointercancel`。
- `mousedown`、`mousemove`、`mouseup`、`click`。
- 本地坐标、窗口坐标和屏幕坐标。
- `button`、`buttons`、压力和指针类型。
- Ctrl、Shift、Alt、Meta 等修饰键。
- `wheel` 的像素和行滚动语义。
- `setPointerCapture`、`releasePointerCapture`。
- Pointer enter、leave、over、out。
- 键盘按下、抬起和重复。
- 焦点、失焦、Tab 顺序。
- Canvas、GPU 节点和普通 Vue 控件统一命中测试。
- 窗口失焦时自动清理捕获和按键状态。

**关闭条件**

拖动、绘图、缩放、游戏操作和复杂控件交互不需要绕过 Vue 事件系统调用产品专用
Rust 接口。

### NUI-IME-01：桌面输入法

**实施前状态**

桌面 IME 能力当前被标记为不支持。

**目标能力**

- 中文、日文等组合输入。
- `compositionstart`、`compositionupdate`、`compositionend`。
- 候选窗口位置跟随输入光标。
- Vue 输入框、文本区域和自定义编辑器共享一致语义。
- 多窗口焦点切换时正确启停 IME。
- 高 DPI 和窗口缩放下候选位置正确。

**关闭条件**

Vue 输入组件可正常使用中日韩输入法，候选框位置和最终文本均正确。

### NUI-WINDOW-01：单 V8 多窗口 Vue 根

**实施前状态**

Rust 窗口基础存在，但 VueHost 当前主要对应单文档和单视口。

**目标接口**

```ts
const windowHandle = await Nana.windows.create(options)
windowHandle.mount(AppComponent, props)
```

需要支持：

- 单个 V8 isolate/context。
- 多个原生窗口。
- 每个窗口独立 Vue 根和组件树。
- 独立尺寸、DPI、焦点、输入和重绘状态。
- 窗口间共享 JS 模块、业务状态和资源。
- 模态窗口、无边框窗口、透明窗口和普通窗口。
- 窗口关闭时卸载 Vue 根并释放关联资源。
- 不为每个窗口创建新的 V8 isolate。

**关闭条件**

Vue+JS 可以创建和管理全部应用窗口，Rust 只执行通用窗口操作。

### NUI-COMPOSE-01：统一混合图层

**实施前状态**

Vue 控件、Canvas、宿主纹理和未来 WebGPU 内容尚未形成统一的组合规则。

**目标能力**

所有显示节点共享：

- 布局尺寸和坐标系。
- `transform`。
- `opacity`。
- 矩形和圆角裁剪。
- 层级和覆盖顺序。
- 滚动容器裁剪。
- DPI 缩放。
- 脏区和重绘调度。
- `requestAnimationFrame`。
- 窗口遮挡、最小化和恢复后的调度。
- 透明窗口的预乘 Alpha 规则。

混合内容不得通过额外子窗口拼接，以避免层级、输入和透明度不一致。

**关闭条件**

普通 Vue 控件可以正确覆盖在 Canvas、WebGPU、Live2D 或视频节点上，滚动、动画、裁剪
和输入行为一致。

### NUI-DIAG-01：JS 与渲染诊断

**实施前状态**

缺少替代 Electron DevTools 的完整诊断能力。

**目标能力**

- V8 inspector 接入。
- JS 异常和调用栈。
- 未处理 Promise rejection。
- Vue warning 和 error handler。
- Host API 调用跟踪。
- 活跃资源句柄统计。
- 窗口、Canvas 和 GPU 资源生命周期诊断。
- 帧调度、绘制耗时和丢帧信息。
- Device Lost、纹理失效和渲染错误报告。

诊断能力只向开发工具暴露，不在产品 UI 中显示技术说明。

**关闭条件**

无需 Chromium DevTools，也能定位 JS、Vue、Host Bridge 和 GPU 生命周期问题。

## 5. 不属于 NanaUI 的能力

以下内容由使用 NanaUI 的应用负责，不计入 NanaUI 缺口：

- Logic Core 业务状态机和业务 dispatch。
- 存档格式、业务数据库和业务持久化。
- 日记、知识库、UGC、提示词和账号数据。
- TTS、STT、RTC、分析、登录、AI 服务及厂商 SDK。
- 产品文件目录和资源包格式。
- Live2D 动作、表情、激活事务和业务命令协议。
- 小游戏规则、关卡、存档和资产。
- Electron IPC 名称及其兼容适配器。
- Android WebView 桥接和应用侧平台同步。
- 任意外部网页、CSR 页面和第三方网页脚本执行。
- 浏览器完整 DOM、CSSOM、WebGL 和网页兼容引擎。

应用可以通过通用 Host API 接入上述服务，但具体方法、数据结构和实现不应进入 NanaUI
核心。

## 6. 实现优先级

### P0：替代 Electron 的阻塞项

1. `NUI-ARCH-01` Vue 产品层权威边界。
2. `NUI-BRIDGE-01` 异步与二进制 Host Bridge。
3. `NUI-INPUT-01` 完整输入事件。
4. `NUI-IME-01` 桌面输入法。
5. `NUI-WINDOW-01` 单 V8 多窗口 Vue 根。
6. `NUI-GPU-01` 真实 GPU 内容节点。
7. `NUI-COMPOSE-01` 统一混合图层。
8. `NUI-CANVAS-01` Canvas2D 兼容层。
9. `NUI-RESOURCE-01` 图像与媒体资源对象。

### P1：GPU 内容迁移与开发效率

1. `NUI-WEBGPU-01` WebGPU 风格 JS API。
2. `NUI-DIAG-01` JS 与渲染诊断。

## 7. 总体验收标准

NanaUI Vue+JS 兼容层只有在满足以下条件后，才具备替代 Electron 的基础能力：

- 完整产品 UI 可以只由 Vue+JS 编写。
- 单个 V8 context 同时运行业务 JS、Vue UI 和多个窗口。
- Canvas2D 小游戏和绘制组件可以运行。
- 3D 内容可以使用 WebGPU 风格接口重写，不依赖 WebGL。
- Live2D、视频等原生内容可以作为通用 GPU 节点嵌入 Vue。
- Vue 控件与 Canvas、WebGPU、宿主纹理之间的层级、裁剪和输入一致。
- 中日韩输入法正常工作。
- 图像、音频和媒体帧不通过大型 JSON 或 Base64 搬运。
- WGPU 实例、Device 和 Queue 仍由 NanaUI 宿主统一管理。
- Device 重建后，旧 HostTexture 和 JS GPU 资源能够可靠失效。
- NanaUI 核心中不存在具体产品页面、状态机、小游戏规则或厂商服务实现。
