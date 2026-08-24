# 实时画面

着色器、预览视口、离屏纹理和按钮一样，是树上的一块内容：有位置、会被裁切、点得到。不是盖在界面上的一层，也不是抠出来的洞。

先把一扇普通窗口跑通，见 [开始](start.md)，再把画面挂上去。心智模型见 [框架如何运行](how-it-works.md)。

```bash
cargo run -p nana-ui --example gpu-view-demo --features hosted,bundled-fonts
cargo run -p nana-ui --example hosted-gpu-demo --features hosted,bundled-fonts
```

`gpu-view-demo` 把 `GpuView` 当节点，用默认 painter 画进当前 pass。`hosted-gpu-demo` 和 `examples/runtime-host-fixture` 是应用先画到纹理、再作为 `GpuTextureView` 被 UI 采样。

## 选哪条接入

| 内容怎么来 | 树上挂 | 宿主职责 |
| --- | --- | --- |
| 已经（或每帧都会）画到一张可采样纹理 | `GpuTextureView::new("preview")` | `HostTextureRegistry::register`；在 `prepare_window_frame` 里用**同一** Device/Queue 更新纹理 |
| 可以直接写进 UI 当前的 render pass | `GpuView` | 默认有演示 painter。产品实现 `SceneGpuRenderer`，经 `scene_gpu_renderers()` 交给这一窗 |
| 必须先离屏画完，UI 再采样（多层内容） | 纹理节点 + `SceneResourceProducer` | producer 在 Scene 采样**之前**编码；仍是同一对 Device/Queue |

多层实时画面就是相邻的几张 `GpuTextureView`，由应用映射。不要绕过界面树去直写窗口 Surface。

## 宿主纹理

`run_runtime` 创建**一份** Adapter / Device / Queue（`HostedGpuContext`）。附加窗口只新建 Surface，共享同一份设备。已经有外部事件循环时，用更低层的 `HostedGpuContext`；不要再 `request_device`。

最小闭环（结构同 `runtime-host-fixture`）：

```rust
// 树上：和 Button 一样占布局
let preview = cx.create_component(document_id, GpuTextureView::new("preview"))?;

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

合成顺序就是文档顺序：`"nana.host-texture"` 在主 pass 里、在这个节点该出现的位置采样，不攒到帧尾。

## GpuView

`GpuView::new(slot_id)` 投影 `CustomRenderNode`，renderer 键是 `"gpu-view"`。

- `scene_gpu_renderers()` 返回 `None`：宿主装上默认演示 painter。
- 返回 `Some(空 registry)`：`"gpu-view"` **画不出来**。要自定义就登记自己的 `SceneGpuRenderer`，不要交空表。

`GpuViewMode::Inline` 复用当前 dest pass；`Standalone` 在同一 encoder / 目标上另开 pass。Renderer 不得 `request_device`，也不得 submit 宿主正在用的 encoder。

## 不要做的

- 为界面另开一套 Device / Queue
- 把画面读回 CPU、编码成图片再贴回去
- 在 UI 画完之后再往 Surface 上盖一层实时画面
- 把 GPU 内容攒到帧尾一次性画，打乱和按钮的前后关系
- 同一资源在一帧里提交互相冲突的 revision（整帧会失败，不会挑一个用）
