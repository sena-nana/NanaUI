# 实时画面

实时画面是树上的一块内容。它有位置，会被裁切，可以被点。不是盖在界面上的一层，也不是抠出来的洞。合成顺序就是文档顺序。

它仍走同一条路径：应用状态在 `UiWorld`，flush 成 `UiScene`，`SceneWgpuPainter` 画进宿主 Surface。宿主只有一份 `GpuContext`。这一帧的 `FrameContext` 也是宿主的。painter 往里面录，不另开 Device，也不把像素读回 CPU。

WGPU 是唯一正式后端。`wgpu-interop` 交出的是合同背后同一份对象，不会创建第二套设备。普通路径只用 `nana-gpu` 的类型。全文在 [实时画面](../reference/gpu.md)。

::: warning
不要再 `request_device`。附加窗口、逃生口和跨线程换帧，用的都是宿主这一份 `GpuContext`。
:::

## 从纹理到节点

先把画面画到可采样纹理，再挂到树上。Vue 用 `NanaGpu` / `<nana-gpu>`。默认就是这条。

:::api

```rust view
// 树上：和 Button 一样占布局
let preview = cx
    .mount_view_root(document_id, || view! {
        <Texture resource="preview" />
    })?
    .root::<GpuTextureView>();
```

```rust rust
// 树上：和 Button 一样占布局
let preview = cx
    .mount_view_root(document_id, || widget(GpuTextureView::new("preview")))?
    .root::<GpuTextureView>();
```

:::

`GpuTextureView::new("preview")` 占一块布局，和别的控件一样。`host_textures()` 用同一个字符串登记这个 slot。`prepare_window_frame` 里用 `context.gpu()` 更新纹理。

```text
GpuTextureView::new("preview")     树上的节点
host_textures()                    登记同一个 slot
HostTexture                        包住可采样的 GpuTexture
prepare_window_frame               这一帧的像素或换纹理
```

`create_texture` / `write_texture` 在交给后端前就校验设备代次、usage、尺寸和字节长度。出错返回 `GpuError`，不会进到 WGPU 的校验 panic。CPU 像素用 `write_texture` 写入。

`HostTexture` 用稳定 slot 和 generation 包住这张 `GpuTexture`。你替换或改尺寸时加上 generation。NanaUI 只重建绑定，不拆布局。换掉纹理用 `replace_texture`。同一张纹理的内容变了，用 `HostTexture::invalidate()`。

你也可以留下 `registry.slot("preview")` 返回的 `TextureSlot`。内容更新调用 `invalidate()`，替换纹理调用 `replace()`。通知只唤醒引用这个 slot 的目标。两条路都不要改 Runtime 节点。

采样发生在主 pass 里、这个节点该出现的位置。renderer 键是 `"nana.host-texture"`。多层就是相邻的几张 `GpuTextureView`。不要绕过树去直写窗口 Surface。

`HostTexture::device_generation()` 是纹理所在的设备。它和 painter 的设备不同时（设备换过、却没有在 `rebuild_gpu` 里重建），整帧以 `ScenePaintError::StaleHostTexture` 拒绝。不会把旧设备的纹理交给后端。

丢失只记在 `GpuContext::is_lost()`。这个状态是粘性的。替换设备就是换一个新的 `GpuContext`。`rebuild_gpu` 里只在 `context.gpu()` 上重建依赖旧设备的资源。程序实例还在。

`HostTextureRegistry::painted_extent(slot)` 返回这个 slot 当前的绘制需求：最近一次全新绘制里，各节点设备像素尺寸逐边取最大。它是只读的。登记多大的纹理仍由你决定。还没有节点画到它时返回 `None`。

## 没有中间纹理

必须写进当前界面 pass、没有中间纹理时，才用 `GpuView`。`GpuView::new(slot_id)` 投影 `CustomRenderNode`，renderer 键是 `"gpu-view"`。这里的 `slot_id` 是 `u64`，不是 `HostTextureRegistry` 的键。`HostTexture::id` 也不是这个 slot。

你要显式登记自己的 `SceneGpuRenderer`。`scene_gpu_renderers()` 返回 `None` 或空表时，未注册的 renderer 会报错。Renderer 不得 `request_device`，也不得提交宿主正在录的那一帧。

`GpuViewMode::Inline` 复用当前目标 pass。`Standalone` 在同一帧、同一目标上另开 pass。

可以批，但批的是文档顺序上连续的一段。中间任何一条 Quad、文字、图标、HostTexture 或合成组边界都会把这段切开。这不是把 GPU 内容攒到帧尾。和逐个画相比，文档顺序不变，变的只是 draw 次数。

## 另一个线程上的画面

模型、解码或合成若在别的线程，用 `FrameExchange` 把完成的帧交给窗口，用 `FrameBinding` 绑到上面的 slot。两者都在宿主那一个 `GpuContext` 上。生产端不必依赖 `nana-ui`。

这是 GPU 里的一次复制，不是零拷贝，也不回读 CPU。源纹理要在同一设备上，并且能被复制。池满时 `copy_from` 返回 `PoolFull`，丢掉这一帧，不等。界面侧只取最新一帧。

`prepare` 换帧之后，旧帧留到 `presented` 才释放。`window_frame_presented` 之前，不要丢掉仍可能被采样的帧。不要在界面线程上等生产端或 GPU 完成。

设备重建之后，用新的 `GpuContext` 重建 exchange 和 binding。代次不同的 inbox 不会被绑上。

## 这些节点用哪一个槽

| 节点 | 槽 | 结果 |
| --- | --- | --- |
| `<nana-gpu>` | `"nana.host-texture"` + 你登记的名字 | `GpuTextureView` |
| `<nana-gpu-view>` | `"gpu-view"` + 十进制 `slot_id` | `GpuView`。你要登记对应的 painter |
| `<canvas data-nana-canvas>` | `canvas:{id}` | 同一套 HostTexture |
| `getContext("webgpu")` | `webgpu-canvas:{id}` | 同一套 HostTexture，不是第二套 Device |
| `<video data-nana-video>` | `video:{id}` | 有槽时不画 poster |
| `<iframe>` | 无 | 显式跳过，不加载 `src` |
| `BrowserView` | Runtime 锚点，外加宿主的原生子视图 | 见下一节 |

没有这些标记的 `<canvas>` 是空盒子。没有槽、也没有 poster 的 `<video>` 同样是空盒子。有 poster、但没有槽时，只显示 poster，不解码。

## BrowserView

`BrowserView` 是宿主上的原生内容例外。Runtime 节点仍拥有布局、可访问性、可见性和生命周期。宿主创建并管理原生子视图。它不是 `GpuTextureView`，不复制 `UiWorld`，也不创建第二个 GPU 设备。

当前正式实现只有 macOS，后端是 `WKWebView`。Windows 和 Linux 返回明确的 `Unsupported`，不会创建占位浏览器。窗口、文件对话框和别的原生内容能力不受这条边界影响。

原生子视图不进入 Runtime / Scene 的离屏截图。非矩形裁剪、透明度或滤镜组，以及被后面的 Runtime 内容盖住时，由宿主隐藏。不要在界面画完之后再把原生子视图盖回窗口上。

## 不要做的

- 为界面另开一套 Device / Queue
- 把画面读回 CPU，编码成图片再贴回去
- 在界面画完之后往 Surface 上盖一层实时画面
- 把 GPU 内容攒到帧尾，打乱它和旁边控件的前后关系
- 同一资源在一帧里提交互相冲突的 revision。整帧会失败，不会挑一个用
- 把 `GpuTextureView` 或 `<iframe>` 当成会加载 `src` 的节点

## 接着读

<div class="next-steps">
  <a class="next-step" href="/architecture/text">
    <p class="next-step-link">文本</p>
    <p class="next-step-caption">测量、整形和留在树上的文本布局。</p>
  </a>
  <a class="next-step" href="/reference/gpu">
    <p class="next-step-link">实时画面合同</p>
    <p class="next-step-caption">换帧、直写 pass 和离屏生产的全文。</p>
  </a>
  <a class="next-step" href="/architecture/frame">
    <p class="next-step-link">一帧</p>
    <p class="next-step-caption">纹理在 flush 之前准备，present 之后才能释放。</p>
  </a>
</div>
