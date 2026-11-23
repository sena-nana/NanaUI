# Live GPU Content (GPU Content Integration)

In NanaUI, live GPU content (shader output, preview viewports, video streams, offscreen render textures) is a first-class node in the UI tree. It takes layout size, is clipped by its parent's rounded corners and scroll regions, takes part in pointer hit testing, and composites with the surrounding buttons, cards and overlays strictly in **document order**. It is neither a "hole" cut out of the window nor a HUD layer drawn last on top of the UI.

The host process holds exactly one `GpuContext` (WGPU is the only official graphics backend), and each frame's `FrameContext` also belongs to the host. `SceneWgpuPainter` records drawing commands directly into the host's frame. It creates no second `Device` / `Queue` and never reads the image back to the CPU. For the complete technical contract, see the [Live GPU content reference](../reference/gpu.md).

- **Precompiled shader archives**: on Apple (macOS 14+ / iOS 17+, `MSL 3.1`) built-in shaders load offline precompiled zlib-compressed MSL (`stages.msl.zz`, `0 B` resident `naga` IR), and other platforms load entry-pruned compacted `NAGA1` IR; runtime WGSL compilation for custom user `MaterialProgram` shaders is strictly opt-in behind `runtime-wgsl`.
- **Wide-gamut / HDR & rasterizer-ordered storage textures**: `SceneWgpuPainter` composites in premultiplied linear scRGB and upgrades the SDR working target to `Rgba16Float` via `SurfacePresentationPolicy` only when out-of-sRGB colors (OKLCH / Display P3) or `Rgba16Float` `HostTexture`s appear (pure 8-bit SDR stays zero-overhead). NanaUI's `third_party/wgpu` fork also adds `TEXTURE_STORAGE_RASTERIZER_ORDER` across DX12 ROV, Vulkan fragment pixel interlock, and Metal `raster_order_group` for single-pass read-modify-write compositors.

::: warning
Never call `request_device` again in an application or plugin. Multiple windows, custom renderers (`wgpu-interop`) and cross-thread frame exchange (`FrameExchange`) all reuse the host's single `GpuContext`.
:::

## Default path: from `HostTexture` to a `GpuTextureView` node

More than 90% of cases take this default path: first draw the image into a sampleable texture (`GpuTexture`), then mount it into the UI tree as a `GpuTextureView` (template tag `<Texture>`).

:::api

```rust view
// On the tree: takes layout like Button, is clipped, and is clickable
let preview = cx
    .mount_view_root(document_id, || view! {
        <Texture resource="preview" />
    })?
    .root::<GpuTextureView>();
```

```rust rust
// On the tree: takes layout like Button, is clipped, and is clickable
let preview = cx
    .mount_view_root(document_id, || widget(GpuTextureView::new("preview")))?
    .root::<GpuTextureView>();
```

:::

The whole data flow is four parts working together:

```text
GpuTextureView::new("preview")   ──► node on the UI tree (takes part in layout, clipping, hit testing)
host_textures()                  ──► registers a slot under the same string key "preview"
HostTexture                      ──► wraps a GpuTexture with a generation and a version
prepare_window_frame             ──► writes pixels or replaces the texture before each frame's flush
```

- **Up-front argument validation**: `GpuContext::create_texture` and `write_texture` validate the device generation, usage, size limits and row byte alignment before calling into the backend. Errors come back as a structured `GpuError`; they never trigger a WGPU validation panic.
- **Updating content vs. replacing the texture**: you can keep a `TextureSlot` through `registry.slot("preview")`. When the content of the same texture changes, call `invalidate()`. When the texture size or the object itself changes, call `replace()` (which increments `generation`). NanaUI only rebuilds the texture binding and wakes the windows that reference the slot; **no Runtime node has to change or be rebuilt**.
- **Sampling in document order and device-generation protection**: textures are sampled in place in the main render pass in the order their nodes appear (renderer key `"nana.host-texture"`). Multiple layers of live GPU content just mean mounting several `GpuTextureView`s side by side. If the device is lost and rebuilt while some `HostTexture` still holds the old `device_generation()`, the painter rejects the whole frame with `ScenePaintError::StaleHostTexture` and never sends a stale handle into the new device.
- **Reading the actual painted size on demand**: `HostTextureRegistry::painted_extent(slot)` returns, read-only, the largest physical pixel size any consuming node of that slot needed in the most recent complete paint (with `ContentFit`, node transforms and DPI scaling already applied), so the producer can prepare a texture at the real resolution.

### Offscreen 3D viewports

3D scenes and game viewports follow the same default path: in `prepare_window_frame`, open `context.gpu().begin_producer_frame(...)` (which does not consume presentation frame slots), record a 4x MSAA `Rgba16Float` + `Depth32Float` offscreen pass via `wgpu-interop`, resolve into a `GpuTexture::from_wgpu` texture sized from `painted_extent` (falling back to `content_geometry().physical_size` on frame 0), publish via `TextureSlot::replace` / `invalidate`, and rebuild pipelines and targets in `rebuild_gpu` on `DeviceGeneration` changes. See the `viewport-3d-demo` example, or the optional `nana-ui-viewport3d` crate for a ready-made orbit camera and glTF preview viewport.

## The no-intermediate-texture path: `GpuView` and `SceneGpuRenderer`

Use `GpuView` only when **there is no intermediate texture and the drawing commands must be recorded directly into the current UI render pass**. `GpuView::new(slot_id)` projects a `CustomRenderNode` with the renderer key `"gpu-view"` (note that `slot_id` here is a `u64`, not the string key of `HostTextureRegistry`).

The application must explicitly register the matching `SceneGpuRenderer` implementation through `scene_gpu_renderers()`:

- `GpuViewMode::Inline` reuses the render pass the current target has open. `Standalone` opens a separate pass on the same target in the same frame.
- When several `GpuView` nodes of the same kind are adjacent in document order, the renderer can merge them into one instanced draw through `draw_batch_in_pass`. But as soon as any ordinary Quad, text, icon, HostTexture or composition-group boundary sits between them, the batch is cut there. So **batching only reduces the number of draw calls; it never changes document occlusion order**.

## Cross-thread latest-frame exchange: `FrameExchange` and `FrameBinding`

When model rendering, video decoding or compositing runs on a separate worker thread, use `FrameExchange` from `nana-frame-exchange` to hand completed frames to the UI window, and bind them to a host texture slot on the window side with `FrameBinding` (the producer only depends on `nana-frame-exchange`, not on `nana-ui`):

- **GPU-internal copy, lock-free on both sides**: `copy_from` performs one `copy_texture_to_texture` on the same `GpuContext` (no CPU readback). When the texture pool is full it returns `PoolFull` and drops the current produced frame. The UI side always takes only the latest completed frame, and neither side blocks waiting on the other.
- **Lease lifecycle**: the window calls `binding.prepare(...)` in `prepare_window_frame` to swap in the new frame. The replaced old frame stays held until `window_frame_presented` fires, and only then returns to the pool, so the image being presented is never overwritten. After the device is rebuilt, rebuild `FrameExchange` and `FrameBinding` in `rebuild_gpu` from the new `GpuContext`.

## Slot mapping for media and canvas nodes

| Node / interface | Underlying slot key | Behavior |
| --- | --- | --- |
| `<Texture>` / `<nana-gpu>` | `"nana.host-texture"` + the registered slot name | Standard `GpuTextureView` host texture node |
| `<nana-gpu-view>` | `"gpu-view"` + decimal `slot_id` | `GpuView` direct-recording node (the host must register the matching `SceneGpuRenderer`) |
| `<video data-nana-video="{id}">` | `video:{id}` | Host pushes frames to show video (with a slot, `poster` is not drawn; without a slot only the static `poster` shows) |
| `<canvas>`, `<iframe>` | None | Not supported. Use `<Texture>` for GPU content and `BrowserView` for web pages |
| `BrowserView` | Runtime layout anchor + host-native child view | See the next section, "The `BrowserView` native child view exception" |

## The `BrowserView` native child view exception

`BrowserView` is the only host exception in the framework that may embed a native web child view inside a window. The Runtime node still owns layout placement, accessibility, visibility and lifecycle, while the host creates and manages the child view on the native window (currently only macOS `WKWebView`; Windows and Linux explicitly return `Unsupported` and do not create a placeholder browser).

The native child view is not a `GpuTextureView` and does not appear in offscreen screenshots. When the node meets a non-rectangular clip, an opacity/filter composition group, or is occluded by a Runtime control later in order, the host automatically hides the native child view so it never punches through menus and dialogs above it.

## Read next

<div class="next-steps">
  <a class="next-step" href="/architecture/text">
    <p class="next-step-link">Text</p>
    <p class="next-step-caption">Font selection, shaping and text layout retained on the tree.</p>
  </a>
  <a class="next-step" href="/reference/gpu">
    <p class="next-step-link">Live GPU content reference contract</p>
    <p class="next-step-caption">The full text on HDR parameters, the frame upload ring, cross-thread exchange and headless WebSurface.</p>
  </a>
  <a class="next-step" href="/architecture/frame">
    <p class="next-step-link">A frame</p>
    <p class="next-step-caption">The full timing of textures prepared before flush and released after present.</p>
  </a>
</div>
