# 实时画面

着色器、预览视口、离屏纹理和按钮一样，是树上的一块内容：有位置、会被裁切、点得到。不是盖在界面上的一层，也不是抠出来的洞。

先把一扇普通窗口跑通，见 [开始](start.md)，再把画面挂上去。心智模型见 [框架如何运行](how-it-works.md)。

第一次接入：把画面画到可采样纹理，树上挂 `GpuTextureView`。Vue 用 `NanaGpu` / `<nana-gpu>`，Live2D 多层、预览视口都是这条。

```bash
cargo run -p nana-ui --example hosted-gpu-demo --features hosted,bundled-fonts
```

`examples/runtime-host-fixture` 同结构。现场直写见 [GpuView](#gpuview)。

## 挂上去

`run_runtime` 创建**一份** Adapter / Device / Queue（`HostedGpuContext`）。附加窗口只新建 Surface，共享同一份设备。已经有外部事件循环时，用更低层的 `HostedGpuContext`；不要再 `request_device`。

树上挂 `GpuTextureView::new("preview")`，`host_textures()` 登记同一 slot，`prepare_window_frame` 用**同一** Device / Queue 更新。

```rust
// 树上：和 Button 一样占布局
let preview = cx.build(document_id, |ui| {
    ui.child("preview", GpuTextureView::new("preview"))
})?;

// initialize / rebuild_gpu：用宿主 Device 建纹理，登记 slot
self.textures.register(
    "preview",
    host_texture,
    width,
    height,
    HostTextureAlphaMode::Opaque,
);

fn host_textures(&self, _id: WindowId) -> Option<HostTextureRegistry> {
    Some(self.textures.clone())
}

fn prepare_window_frame(&mut self, id: WindowId, context: &RuntimeProgramContext<Self::Message>) {
    // 用 context.gpu().device() / queue() 画这一帧
    // 换 view 时升 generation：view.replace_view(generation)
}

fn window_frame_presented(&mut self, _id: WindowId, _context: &RuntimeProgramContext<Self::Message>) {
    // present 之后才能释放上一帧的纹理
}

fn rebuild_gpu(&mut self, context: &RuntimeProgramContext<Self::Message>) {
    // 设备丢失后程序实例还在，只把依赖旧 Device 的资源重新绑上
}
```

`HostTexture` 用稳定 slot 和 generation 包住可采样纹理。宿主替换或改尺寸时加 generation，NanaUI 只重建绑定，不拆布局。`revision` 的高 32 位是 generation、低 32 位是内容 version（`pack_gpu_revision`）。

`HostTexture::instance_identity()` 区分使用相同公开 id 的不同 handle，克隆共享该身份。
自接事件循环使用 URL 图片时，给 `SceneWgpuPainter::set_image_waker` 安装唤醒回调，
完成通知后请求重绘。内建宿主已接入；离屏工具可用 `has_pending_images()` 等待资源后再次绘制。
HTTP 获取与解码在后台进行，纹理上传仍使用宿主 Device／Queue。
Quad 与 HostTexture 蒙版共用缓存实现；每个缓存最多同时获取 4 个 HTTP 资源，
闲置纹理最多保留 256 项／64 MiB，120 个绘制帧未使用后释放，当前帧工作集按需保留。
栅格图片解码限制为 4096 像素边长和 64 MiB 分配预算，SVG 沿用 2048 像素边长上限。

合成顺序就是文档顺序：`"nana.host-texture"` 在主 pass 里、在这个节点该出现的位置采样，不攒到帧尾。多层就是相邻的几张 `GpuTextureView`。不要绕过界面树去直写窗口 Surface。

## GpuView

没有中间纹理、必须写进当前 UI pass 时才用。`gpu-view-demo` 是演示。

`GpuView::new(slot_id)` 投影 `CustomRenderNode`，renderer 键是 `"gpu-view"`。

- `scene_gpu_renderers()` 返回 `None` 或空 registry：未注册的 renderer 会报错。
- 示例自行注册演示 painter；应用显式登记自己的 `SceneGpuRenderer`。

`GpuViewMode::Inline` 复用当前 dest pass；`Standalone` 在同一 encoder / 目标上另开 pass。Renderer 不得 `request_device`，也不得 submit 宿主正在用的 encoder。

### 批绘制

`SceneGpuRenderer` 有两个带默认实现的方法，不实现就是原来的逐节点路径：

```rust
fn batch_capacity(&self) -> usize { 1 }
fn draw_batch_in_pass(&self, nodes: &[SceneGpuBatchNode<'_>], pass, context) -> usize { 0 }
```

`batch_capacity() > 1` 时，painter 把 **display list 上连续**的同 renderer 实例、无 `dedicated_pass`、bounds 非空的节点作为一段交给你；返回值是这段前缀里你实际编码了几个，返回 0 就退回 `draw_in_pass`。你可以只吃掉前缀——例如只处理共享同一 clip 的那几个。

这**不是**把 GPU 内容攒到帧尾。run 是 display list 的连续切片：中间任何一条 Quad / Text / Icon / HostTexture / backdrop / 合成组边界都终止它，shader 节点不可能跨过一个 Button。document order 与不批处理时逐比特相同，变的只是 draw 次数。

内置的 `DefaultGpuViewRenderer` 是实例化的参考实现：一条 instance-step 顶点缓冲，N 个相邻同 renderer 节点一次 draw，不需要任何可选 device feature。**上限**：合并的是同一条管线的 run；每个节点一个不同 shader 时，下限就是每种管线一次 draw。

多节点共用一个 shader 时，用**一个** `slot_id`、**一个** `revision`，把差异放进 `params`——`params` 不参与 resource 冲突判定，也不使 frame plan 失效。对一部分节点 bump `version` 而对其余不 bump 会让整帧被拒（见下面「不要做的」）。需要各自独立 revision 时给每个节点一个自己的 slot；渲染图按 renderer 而不是按 resource 建 pass，所以这不再随节点数增加 pass。

`SceneGpuRenderContext` 带 `dest_size`（目标的物理像素尺寸），Standalone 路径的 viewport / scissor 与主 pass 一致。

节点上的 `palette` 和 `seed` 走 `CustomRenderNode::params`，槽位见 `gpu_view_params`。Runtime 只搬运这串数，语义由 renderer 键定义；换 renderer 就换一套自己的槽位约定。

```bash
cargo run -p nana-ui --example gpu-view-demo --features hosted,bundled-fonts
```

## 媒体槽（Canvas / video / iframe）

这些不是浏览器。可见输出仍然只走 Runtime → UiScene → `SceneWgpuPainter`。

默认 GPU 接入是 **字符串 slot** 的 `GpuTextureView` / `"nana.host-texture"`。`GpuView::new(slot_id: u64)` 只给 `"gpu-view"` 直写 pass，**不是** `HostTextureRegistry` 的键。`HostTexture::id` 是 painter 缓存键，同样不是 slot。

| 节点 | 槽位合同 | L1 行为 |
| --- | --- | --- |
| `<nana-gpu>` / `data-nana-gpu` | `"nana.host-texture"` + 宿主登记的 slot 名 | `GpuTextureView` |
| `<nana-gpu-view>` | `"gpu-view"` + 十进制 `slot_id` | `GpuView`。宿主必须显式注册对应 painter |
| `<canvas data-nana-canvas="{id}">` | `"nana.host-texture"` + `canvas:{id}` | 2D 像素来自 `nana-ui-web-api`（tiny-skia），hosted 路径由 `CanvasGpuBridge` dirty upload。`getContext("2d")` 只在 web-api shim 里存在，不是 Chromium 2D |
| `getContext("webgpu")` | `"nana.host-texture"` + `webgpu-canvas:{id}` | 同一套 HostTexture，不是第二套 Device |
| `<video>` / `nana-video` + `data-nana-video="{id}"` | `"nana.host-texture"` + `video:{id}` | Runtime `Video`。宿主推帧。有槽时不画 `poster` |
| `<video poster>`（无槽） | 无 CustomRenderNode；`poster` 走 `content_image` URL | 只显示 poster。不解码、不播 |
| `<iframe>` | 无 | 显式 skip（`skipped_replaced = iframe`），不加载 `src`。不是应用内浏览器 |
| `WebView`（未实现） | `"nana.host-texture"` + `webview:{id}` | 拟议控件 `nana.webview`。像素仍走 HostTexture；引擎、URL、权限在应用侧。见 [应用内浏览器](#应用内浏览器) |

没有 `data-nana-canvas` / `data-nana-gpu` 的 `<canvas>` 是空盒子（`skipped_replaced = canvas`），不会把 `src` 或 pixmap 写进 `content_image` 假装成 2D 位图。无槽且无 poster 的 `<video>` 同样是空盒子（`skipped_replaced = video`）。

`GraphCanvas` 默认画 Scene Quad / Stroke。`"graph-canvas"` 自定义 renderer 不会自动投影；未登记时整帧拒绝。

## 应用内浏览器

合同草案，不是现成控件。没有 `WebView` 类型、没有 `browser` feature，Gallery 不得摆假浏览。`tools/css-parity-webview`（workspace 外）只对照盒模型，不得链进 `nana-ui`。

落地后仍是树上的一块内容，类比 `Video`：Runtime 管布局 / 裁剪 / 命中，宿主管引擎和帧。Vue tag 拟议 `webview`（`nana.webview`）。`<iframe>` 继续 skip，不要改成会加载。

| 名字 | 是什么 |
| --- | --- |
| `WebView`（拟议） | Runtime 控件，声明 URL，投影 `CustomRenderNode`。不是 `GpuTextureView` 别名 |
| 槽 `webview:{id}` | 引擎画面登记到 `HostTextureRegistry` 的键。不是 HWND / NSView |
| `"nana.host-texture"` | 与 `Video` 同一个 Scene renderer，按文档顺序采样 |

URL、白名单、Cookie、引擎选型归**应用**（默认拒绝，localhost 不自动放行）。窗口句柄归宿主；普通控件拿不到。像素在 `prepare_window_frame` 更新同一 slot。禁止 present 后再盖原生 WebView，禁止第二套 Device。

拟议事件：`WebViewNavigated`、`WebViewTitleChanged`、`WebViewFailed`。后退 / 地址栏用现有控件拼，不要让 `WebView` 自绘浏览器 chrome。控件落地前用系统浏览器或应用自己的引擎，不要在树上叠一层原生 WebView。

## 按图离屏

离屏必须按 Scene 图、在采样**之前**编码时，仍挂 `GpuTextureView`，再实现
`SceneResourceProducer`。`encode_scene` 使用宿主在获取 Surface 后建立的 encoder；
生产与 UI 绘制合并提交，成功后才调用 `PreparedSceneResources::submitted`。
编码或绘制失败时整份 encoder 丢弃，生产者不能自行提交或提前报告完成。
标准宿主同时将已获取但未呈现的目标标记为需要 Surface 恢复，下一次获取使用原有
Device / Queue 重建该目标，避免 DX12 的帧延迟等待信号在丢帧后阻止后续获取。
低层 `HostedGpuContext` 消费者应先释放帧的 view 和 encoder，再调用 `discard_frame`
或 `discard_surface_frame`；这些方法不会提交 GPU 工作或请求重绘，重试需求由应用决定。
`hosted-gpu-demo` 展示了这条路径。

Rust 可以保存 `registry.slot("preview")` 返回的 `TextureSlot`。
内容已更新时调用 `invalidate()`，替换纹理时调用 `replace()`；通知只唤醒引用该
slot 的目标。不要为纹理内容更新改写 Runtime 节点。

多目标宿主使用 `SceneWgpuPainter::paint_target(RenderTargetId, ...)`，为每个目标保存
准备好的绘制批次、投影、可写 GPU 缓冲、文字 atlas/renderers 和纹理绑定。着色器、
管线、字体系统及文字整形缓存仍在同一 Device 上共享。目标关闭时调用 `remove_target`；
普通 `paint` 对应单独的缺省目标状态，不能借它复用多个窗口的目标缓存。
`HostedGpuResources::generation()` 标识宿主 Device 代次：克隆上下文不改变代次，
重建设备会改变代次。生产者应随新上下文重建资源，并通过宿主 encoder 编码；
提交成功后的通知才确认该帧生产完成。

自定义 renderer 默认每帧重新准备；显式实现 `preparation_version` 后才允许复用。
版本变化或 renderer 实例替换会重新准备，实际 `render` 仍在需要呈现的帧执行。
**这一条对整棵树收费**：只要有一个未实现该方法的自定义 renderer 在树上，painter 就把整帧
判为不可缓存，重建全部 quad / 文字 / 图标的绘制命令。内置的 `DefaultGpuViewRenderer`
按 `(revision, params)` 实现了它；自己写 renderer 时请照做。

`SceneGpuPrepareContext` 带 `dest_size`（目标物理像素尺寸）。尺寸变化必然使 painter 的
预备批次失效，所以准备阶段看到的就是这批命令实际编码时的尺寸。

## 渲染图的形状

`frame_graph` 按 **renderer** 建 preparation pass（`prepare:{renderer}`），并把 document order 上**连续且同 renderer** 的 Custom 图元并进一个 `custom:{renderer}` pass。pass 数因此对节点数是常数，`FramePlan.operations` 逐字节不变。

`CompiledRenderGraph` 是公开类型：pass 的数量与 label 会随之变化，后端扩展不要把它们当稳定值。多个 renderer 同时存在时，`FramePlan.preparations` 的顺序是「先 renderer 名、再 resource label」；生产者写的是互不相同的外部资源，所以这个顺序没有语义约束。

## 不要做的

- 为界面另开一套 Device / Queue
- 把画面读回 CPU、编码成图片再贴回去
- 在 UI 画完之后再往 Surface 上盖一层实时画面
- 把 GPU 内容攒到帧尾一次性画，打乱和按钮的前后关系
- 同一资源在一帧里提交互相冲突的 revision（整帧会失败，不会挑一个用）
- 为 Android 另写一套 renderer，或把实验 NativeActivity 宿主当成产品 GPU 路径。该宿主仍把 UiScene 交给 `SceneWgpuPainter`，不调用桌面的 `run_runtime`，也不是当前产品目标（见 [Android](android.md)）
- 把 `GpuTextureView` 或 `<iframe>` 当成能加载的浏览器
- 在 UI 画完之后把原生 WebView 盖在窗口上，或让控件拿 HWND / NSView 去挂引擎
