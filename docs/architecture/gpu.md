# 实时画面（GPU 内容集成）

在 NanaUI 中，实时画面（着色器输出、预览视口、视频流、离屏渲染纹理）是界面树上的一等节点：它占据布局尺寸、受父级圆角和滚动区域裁切、参与指针命中测试，并且严格按**文档顺序（document order）**与周围的按钮、卡片、浮层合成——既不是窗口上抠出的“洞”，也不是最后盖在界面顶上的 HUD 层。

宿主进程只持有一份 `GpuContext`（WGPU 是唯一正式图形后端），每一帧的 `FrameContext` 也归宿主所有；`SceneWgpuPainter` 直接往宿主的帧里录制绘制命令，不创建第二套 `Device` / `Queue`，也不把画面读回 CPU。完整技术合同见 [实时画面参考](../reference/gpu.md)。

::: warning
切勿在应用或插件里额外调用 `request_device`。多窗口、自定义渲染器（`wgpu-interop`）以及跨线程帧交换（`FrameExchange`）全部复用宿主这一份 `GpuContext`。
:::

## 默认路径：从 `HostTexture` 到 `GpuTextureView` 节点

90% 以上的场景都走这条默认路径：先把画面画到一张可采样纹理（`GpuTexture`）上，再作为 `GpuTextureView`（模板标签 `<Texture>`）挂进界面树。

:::api

```rust view
// 树上：和 Button 一样占据布局、受裁剪、可点击
let preview = cx
    .mount_view_root(document_id, || view! {
        <Texture resource="preview" />
    })?
    .root::<GpuTextureView>();
```

```rust rust
// 树上：和 Button 一样占据布局、受裁剪、可点击
let preview = cx
    .mount_view_root(document_id, || widget(GpuTextureView::new("preview")))?
    .root::<GpuTextureView>();
```

:::

整条数据流由四部分配合完成：

```text
GpuTextureView::new("preview")   ──► 界面树上的节点（参与布局、裁剪、命中）
host_textures()                  ──► 用同一个字符串键 "preview" 登记槽位
HostTexture                      ──► 包装带代际（generation）与版本（version）的 GpuTexture
prepare_window_frame             ──► 在每帧 flush 前写入像素或替换纹理
```

- **前置参数校验**：`GpuContext::create_texture` 与 `write_texture` 在调用底层后端前就会校验设备代际、usage、尺寸上限与行字节对齐，出错直接返回结构化的 `GpuError`，绝不触发 WGPU 校验 panic。
- **更新内容 vs 替换纹理**：可以通过 `registry.slot("preview")` 保留一份 `TextureSlot`。当同一张纹理的内容更新时调用 `invalidate()`；当纹理尺寸或对象本身更换时调用 `replace()`（递增 `generation`）——NanaUI 只会重建纹理绑定并唤醒引用了该槽位的窗口，**无需改动或重建 Runtime 节点**。
- **按文档顺序采样与设备代际防护**：纹理在主渲染通道中按节点出现的顺序就地采样（渲染器键为 `"nana.host-texture"`），多层实时画面只需并排挂载多张 `GpuTextureView`。若设备发生丢失重建而某张 `HostTexture` 仍停留在旧的 `device_generation()`，painter 会以 `ScenePaintError::StaleHostTexture` 拒绝整帧，绝不把过期句柄送进新设备。
- **按需读取实际绘制尺寸**：`HostTextureRegistry::painted_extent(slot)` 只读返回该槽位在最近一次完整绘制中各消费节点所需的最大物理像素尺寸（已计入 `ContentFit`、节点变换与 DPI 缩放），供生产端按真实清晰度准备纹理。

## 无中间纹理路径：`GpuView` 与 `SceneGpuRenderer`

只有在**没有中间纹理、必须把绘制命令直接录进当前 UI 渲染通道**时，才使用 `GpuView`。`GpuView::new(slot_id)` 投影出渲染器键为 `"gpu-view"` 的 `CustomRenderNode`（注意此处的 `slot_id` 是 `u64`，不是 `HostTextureRegistry` 的字符串键）。

应用需通过 `scene_gpu_renderers()` 显式注册对应的 `SceneGpuRenderer` 实现：

- `GpuViewMode::Inline` 直接复用当前目标正在打开的渲染通道（pass）；`Standalone` 则在同一帧、同一目标上另开一个独立的 pass。
- 当连续多个同类型 `GpuView` 节点在文档顺序上紧邻时，渲染器可通过 `draw_batch_in_pass` 将它们合并为一次实例化绘制；但一旦中间穿插了任何普通 Quad、文本、图标、HostTexture 或合成组边界，批次就会在此截断——因此**合批只减少 draw call 次数，绝不改变文档遮挡顺序**。

## 跨线程最新帧交换：`FrameExchange` 与 `FrameBinding`

当模型渲染、视频解码或合成跑在独立工作线程时，使用 `nana-frame-exchange` 的 `FrameExchange` 将完成的帧交给 UI 窗口，并在窗口侧用 `FrameBinding` 绑定到宿主纹理槽（生产端只需依赖 `nana-frame-exchange`，无需依赖 `nana-ui`）：

- **GPU 内部拷贝、双方无锁**：`copy_from` 在同一 `GpuContext` 上执行一次 `copy_texture_to_texture`（不回读 CPU）；纹理池满时直接返回 `PoolFull` 丢弃当前生产帧，UI 侧永远只取最新完成的帧，双方互不阻塞等待。
- **租约（Lease）生命周期**：窗口在 `prepare_window_frame` 中调用 `binding.prepare(...)` 换上新帧，被替换的旧帧会一直保留到 `window_frame_presented` 触发后才归还池槽，确保正在呈现的画面绝不被覆写。设备重建后，需在 `rebuild_gpu` 中基于新的 `GpuContext` 重建 `FrameExchange` 与 `FrameBinding`。

## 各类媒体与画布节点的槽位映射

| 节点 / 接口 | 底层槽位键 | 行为说明 |
| --- | --- | --- |
| `<Texture>` / `<nana-gpu>` | `"nana.host-texture"` + 登记的槽位名 | 标准 `GpuTextureView` 宿主纹理节点 |
| `<nana-gpu-view>` | `"gpu-view"` + 十进制 `slot_id` | `GpuView` 直写节点（宿主需注册对应 `SceneGpuRenderer`） |
| `<video data-nana-video="{id}">` | `video:{id}` | 宿主推帧显示视频（有槽位时不画 `poster`；无槽位仅显示静态 `poster`） |
| `<canvas>`、`<iframe>` | 无 | 不支持。GPU 画面用 `<Texture>`，网页用 `BrowserView` |
| `BrowserView` | Runtime 布局锚点 + 宿主原生子视图 | 见下节「BrowserView 原生子视图例外」 |

## `BrowserView` 原生子视图例外

`BrowserView` 是框架中唯一允许在窗口内嵌入原生网页子视图的宿主例外：Runtime 节点仍然负责布局占位、无障碍、可见性与生命周期，而宿主在原生窗口上创建并管理子视图（当前仅支持 macOS `WKWebView`，Windows 与 Linux 明确返回 `Unsupported`，不会创建占位浏览器）。

原生子视图不属于 `GpuTextureView`，不会进入离屏截图；当节点遇到非矩形裁剪、透明度/滤镜合成组或被排在后面的 Runtime 控件遮挡时，宿主会自动隐藏原生子视图，防止它穿透遮挡上层菜单与对话框。

## 接着读

<div class="next-steps">
  <a class="next-step" href="/architecture/text">
    <p class="next-step-link">文本</p>
    <p class="next-step-caption">字体选择、整形与保留在树上的文本布局。</p>
  </a>
  <a class="next-step" href="/reference/gpu">
    <p class="next-step-link">实时画面参考合同</p>
    <p class="next-step-caption">HDR 参数、帧上传环、跨线程交换与无头 WebSurface 全文。</p>
  </a>
  <a class="next-step" href="/architecture/frame">
    <p class="next-step-link">一帧</p>
    <p class="next-step-caption">纹理在 flush 前准备、在 present 后释放的完整时序。</p>
  </a>
</div>
