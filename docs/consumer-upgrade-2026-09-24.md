# Consumer upgrade notes (2026-09-24)

This round lands Issue #182: nana-text's `EditSession` is now the only storage for an editor's committed text, selections and IME composition. For the division of responsibilities, see "Editor storage: EditSession (#182)" in [Text engine](text-engine.md).

## API changes

### Text values are shared (`TextValue`)

- New type `nana_ui_runtime::TextValue`, an alias of `nana_text::SharedText`.
  - A text buffer behind an `Arc`, plus an optional `TextStamp`. Cloning it only bumps a reference count.
  - It dereferences to `&str`, and compares directly with `&str` / `String`.
  - Build one with `.into()` from `String` / `&str` / `Arc<str>`. Wrapping an `Arc<str>` copies nothing.
  - To get a `String`, call `.to_string()` / `.into_string()`.
- These fields change from `String` / `Arc<str>` to `TextValue`:

  | Field | Was |
  | --- | --- |
  | `TextContent.value` | `String` |
  | `TextInputState.value` | `String` |
  | `TextInputPresentation.display_value` | `String` |
  | `ComponentTextRegion.content` | `Arc<str>` |
  | `AccessibilityNode.value` | `Option<Arc<str>>` |
  | `TextChanged.value` | `String` |
  | the scene's `ScenePrimitiveKind::Text.content` | `String` |

  - Construction sites add `.into()`: `TextContent { value: label.into() }`.
  - Where a `String` is needed, change `.value.clone()` to `.value.to_string()`.
  - `TextInputState::new` / `replace_value` now take `impl Into<TextValue>`.
- `nana_text::SharedText::replace_range` is public. Each call draws a fresh stamp. It edits in place only when this value alone holds an owned buffer; a shared buffer, or one built from an `Arc<str>`, is copied once. A reversed range panics, whichever buffer backs the value.

### Reading editor state from the world

- `UiWorld::text_input(id)` returns `Option<TextInputView<'_>>`.
  - It borrows the session and copies nothing.
  - Fields: `value: &str`, `selection`, `additional_selections: &[TextSelection]`.
  - Methods: `selections()`, `value_shared()`, `to_state()`, `session()`.
  - It compares directly with `TextInputState`.
  - `.cloned()` becomes `.map(|input| input.to_state())`.
  - Code that needs a `String` changes `.value.clone()` to `.value.to_owned()`.
- `UiWorld::ime(id)` returns `Option<ImeView<'_>>`, with `text: &str` and `selection`.
  - Use `to_composition()` to get an `ImeComposition`, and `ImeComposition::view()` for the reverse.
  - They compare directly with each other.
- `UiWorld::focused_text_input` / `AppContext::focused_text_input` now return `(id, TextInputView<'_>)`.
- `TextSelection` is now an alias of `nana_text::EditSelection`. Fields, constructors and `ordered` / `is_valid_for` are unchanged; `EditSelection` also adds `range` / `is_collapsed`.
- `ExtractedNode.ime` / `ExtractedNode.text_input` are removed and replaced by `editable: bool`. The scene only ever checked whether they were present.
- `TextEditOrigin` gains `Cut`.

### nana-text

- `EditSession` supports multiple selections:
  - `with_selections`, `selections`, `additional_selections`, `set_selections`, `set_primary_selection`, `add_selections`, `collapse_selections`;
  - multi-cursor `insert` / `delete` / `move_caret` / `cut`;
  - `selected_texts`.
- Batch edits: `splice(edits, primary, additional)` and `assign(&SharedText, selections)`, a minimal-diff replacement.
- `EditorGeometry::sync_stamped`, `editable::collapse_edge`, `editable::normalize_selections` / `remap_offset` / `remap_selection`, and `editable::diff::changed_range`.
- `EditSession::assign` while composing: a change the text leaves ambiguous (`"abab"` → `"ab"` lost either `"ab"`) is placed clear of the composition where it can be, so the composition survives, and the primary selection moves with the composition in `assign` and `splice` alike. This is `nana-text` behavior for direct session users; Runtime's value writes re-place the preedit over the selection they write, as before.
- `EditState` gains a public `additional: Vec<EditSelection>` field; code that builds it with a struct literal or destructures it exhaustively must add it.
- `TextWorkCounters` gains `editor_text_bytes_compared`.

## Behavior changes

- **Left/Right with a selection:** without Shift, Left/Right collapse the selection onto its edge on that side of the screen instead of stepping one grapheme from the focus. In right-to-left text the logical end is on the left. Fields that cannot be probed (masked, empty) collapse onto the logical edge.
- **Empty preedit:** the empty preedit a platform reports between keystrokes is no longer treated as a composition.
  - Line numbers, extra cursors, inlays, completions and hover all stay visible.
  - It still blocks form submit until the commit or cancel arrives, as before.
- **Selection sets:** the world and components normalize selections the same way. Overlapping or touching selections merge, and ends inside a grapheme cluster snap back to the cluster boundary. Before, an extra cursor could sit inside an emoji ZWJ sequence.
  - A selection that covers the one it merges with keeps its own direction and affinity (the primary's, when both are the same span). Two carets that meet at a soft-wrapped line end stay there, and Shift+Home from two cursors keeps its focus on the line start. Before, every merge produced a forward selection with downstream affinity.
- **Undo:**
  - A cut is its own step, and typing right after it no longer merges into it.
  - Inserting a snippet is one step, and the single-line length limit now applies to it.
- **NumberInput IME:** committed and surrounding-deleted text goes through the component. Before, the next keystroke overwrote it.
- **Android:** the IME buffer is the session's committed text with the preedit in place of the selection it stands for (`display_text()`, not the masked or folded text the editor draws); before, the preedit was inserted next to the focus.
  - The selection and composing region cross to GameTextInput in UTF-16 code units, the Java side's indices. Before, UTF-8 byte offsets were passed through as they were, which put the IME's composing region and cursor in the wrong place in any non-ASCII text.
- **Folding:** pressing Right across a collapsed fold leaves a caret, not a selection covering the hidden lines.
- **Undo after undo:** typing after an undo or redo starts a new step instead of merging into the step the undo stepped back onto.
- **Cut and copy over an atom:** a selection that cuts into an atom copies the whole atom, which is what a cut deletes.
- **Accessibility:** a secure (password) field without a label no longer falls back to its text for its accessible name.

## Issue #183: GPU backend contract isolation

WGPU stays the only backend, but it is no longer the extension contract. The new crate `nana-gpu` (re-exported by `nana-ui`) owns it: `GpuContext` is the device, `FrameContext` one frame's encoder, `GpuTexture` / `GpuRenderTarget` textures that know their device. Raw WGPU objects are behind the new `wgpu-interop` feature. See [GPU contract and the wgpu escape hatch](gpu.md#gpu-合同与-wgpu-逃生口).

### API changes

| Was | Now |
| --- | --- |
| `HostedGpuResources` (`RuntimeProgramContext::gpu()`) | `GpuContext`: `generation() -> DeviceGeneration`, `capabilities()`, `is_lost()` / `lost_report()`, `create_texture`, `write_texture`, `begin_frame`. Raw adapter/device/queue: `gpu.wgpu()` (wgpu-interop) |
| `generation() -> u64` everywhere (context, `FrameToken`, `FrameInbox`) | `DeviceGeneration` (`.get()` for the number) |
| `HostedGpuResources::submit_lock()` | Gone. `FrameContext::submit`, `write_texture` and `FrameExchange::copy_from` hold the guard themselves; raw submits hold `gpu.wgpu().lock_submission()` |
| `HostedGpuResources::from_existing(adapter, Arc<Device>, Arc<Queue>)` | `GpuContext::from_wgpu(adapter, device, queue)` (wgpu-interop) |
| `HostedGpuShared::from_device(instance, adapter, device, queue)` | `HostedGpuShared::from_device(instance, GpuContext)` (wgpu-interop); `resources()` → `gpu()`; `adapter()` / `adapter_info()` → `gpu().capabilities()` or `gpu().wgpu()` |
| `HostedDeviceLost { reason: String, .. }` | `GpuDeviceLost { reason: GpuLossReason, message }` |
| `HostTexture::from_wgpu(id, generation, TextureView)` / `replace_view(view)` | `HostTexture::new(id, generation, &GpuTexture)` / `replace_texture(&GpuTexture)`; new `device_generation()` |
| `FrameExchange::new(generation, Arc<Device>, Arc<Queue>, capacity, epoch, notify)` | `FrameExchange::new(&GpuContext, capacity, epoch, notify)` |
| `FrameExchange::copy_from(&wgpu::Texture, epoch)` | `copy_from(&GpuTexture, epoch)`, or `copy_from_wgpu(&wgpu::Texture, epoch)` (wgpu-interop). `CopyOutcome` gains `DeviceMismatch` |
| `FrameLease::view()` / `format() -> wgpu::TextureFormat` | `texture() -> &GpuTexture` / `format() -> GpuTextureFormat` |
| `FrameBinding::new(&Device, generation, slot, alpha)` | `FrameBinding::new(&GpuContext, slot, alpha)` |
| `SceneWgpuPainter::new(&Device, &Queue, wgpu::TextureFormat)`, `format()` | `SceneWgpuPainter::new(&GpuContext, GpuTextureFormat)`, `format() -> GpuTextureFormat`, new `gpu()` |
| `paint` / `paint_target(.., &mut CommandEncoder, &TextureView, ..)` | `paint` / `paint_target(.., &mut FrameContext, &GpuRenderTarget, ..)` |
| `record_submit(Duration)` | `record_submit(&GpuSubmission)` |
| `SceneGpuPrepareContext { device, queue, target_format: wgpu::TextureFormat, .. }` and the pass / batch contexts | `{ gpu: &GpuContext, target_format: GpuTextureFormat, .. }` |
| `SceneGpuRenderContext { encoder, target, .. }` | `with_pass(label, \|pass\| ..)`; raw `wgpu_encoder()` / `wgpu_target()` (wgpu-interop) |
| `draw_in_pass` / `draw_batch_in_pass(.., &mut wgpu::RenderPass, ..)` | `(.., &mut ScenePass, ..)`: `set_scissor`, `set_viewport`, `restore_viewport`, `dest_size`; raw `wgpu()` (wgpu-interop) |
| `SceneResourceEncodeContext { device, queue, encoder }` | `{ gpu, .. }` with `frame()`; the encoder is `frame().wgpu_encoder()` (wgpu-interop) |
| `SceneResourceProducer::submitted(node, &Device, SubmissionIndex)`, `PreparedSceneResources::submitted(&Device, SubmissionIndex)` | `submitted(node, &GpuSubmission)`, `submitted(&GpuSubmission)` |
| `encode_scene(scene, &Device, &Queue, &mut CommandEncoder)` | `encode_scene(scene, &mut FrameContext)` |
| `DefaultGpuViewRenderer::with_host` / `with_host_palette`, `default_scene_gpu_renderers_with_host` | Removed: renderers draw on the painter's device. Use `new` / `with_palette` / `default_scene_gpu_renderers` |
| `RuntimeProgramContext::surface_alpha_mode()`, `ResolvedWindowPresentation::alpha_mode()`, `HostedGpuSurface` / `HostedGpuContext::alpha_mode()` → `wgpu::CompositeAlphaMode` | `SurfaceAlphaMode` (same variant names) |
| `HostedGpuSurface` / `HostedGpuContext::format()` → `wgpu::TextureFormat` | `GpuTextureFormat` |
| `HostedGpuError::SurfaceFormatChanged { expected: wgpu::TextureFormat }` | `expected: GpuTextureFormat` |
| `HostedGpuContext::new` / `new_with_surface_mode` / `recreate` / `acquire_frame` / `discard_frame`, `HostedGpuShared::acquire_surface_frame` / `present` / `discard_surface_frame`, `HostedSurfaceFrame` | Unchanged, but public only with wgpu-interop |
| `nana_ui::wgpu` | Only with wgpu-interop |
| `OffscreenSnapshots { device, queue }` (devtools) | `OffscreenSnapshots { gpu: GpuContext }` |

`ScenePaintError` gains `DeviceMismatch`, `StaleHostTexture` and `TargetInFlight`; exhaustive matches need the arms.

### Who needs `wgpu-interop`

Turn it on (`nana-ui/wgpu-interop`) only where you touch WGPU objects: a host that brings its own device (`GpuContext::from_wgpu`, `HostedGpuShared::from_device`), a renderer or producer that records its own pipelines, tooling that reads back. Uploading CPU pixels (`create_texture` + `write_texture`), handing `GpuTexture` frames to `FrameExchange`, and registering `HostTexture` slots do not need it. The framework's own crates (nana-ui, nana-frame-exchange, nana-ui-vue with its JS WebGPU facade, nana-ui-devtools with snapshot readback) do not turn it on, so a Vue app does not get it by accident. In this repository `hosted-gpu-demo`, `embedded-window-lifecycle`, `native-content-probe`, the `text_device_recreation` / `text_lost_device` tests, `examples/runtime-host-fixture`, `examples/vue-hosted-acceptance`, the `nana-js-v8` WebGPU test, the Gallery benchmark and the Android host enable it.

The submission guard is not reentrant: while holding `lock_submission()`, do not call `FrameContext::submit`, `GpuContext::write_texture` or `FrameExchange::copy_from`, which take it themselves.

### Behavior changes

- **Dropping a painted frame is safe.** Before, a `paint` / `paint_target` that returned `Ok` required the encoder to be submitted, or the painter kept drawing from GPU state that never landed. Now dropping the `FrameContext` rolls the painted targets back and their next paint rebuilds them. Painting a target again while an earlier frame that painted it is neither submitted nor dropped returns `TargetInFlight`.
- **Resources from a replaced device are refused.** A `HostTexture` still sampling a texture from a replaced device fails the frame with `StaleHostTexture` instead of reaching WGPU validation; frames and targets from another device fail with `DeviceMismatch`. Rebuild device resources in `rebuild_gpu`, as before.
- **Off-thread copies no longer race surface reconfiguration.** `FrameExchange::copy_from` submitted without the guard documented for off-thread submits, and could hit `GpuWaitTimeout` while a window resized. It now holds it.
- **Renderer caches follow the device.** `DefaultGpuViewRenderer` keyed its pipeline by format only: a registry kept across a device replacement drew with the old device's pipeline, and windows of different formats rebuilt it on every alternation. It now keys by device generation and format.
- **Embedded loss is recorded on the context.** `EmbeddedRuntime::notify_device_lost` marks the `GpuContext` lost, so producer threads can query `is_lost()`.
- **A device lost with no window open recovers with the next window.** Before, the loss was consumed with nothing to rebuild from and the next window kept presenting on the lost device.
- **Diagnostics:** `framework::gpu` appends counter `FRAMES_DISCARDED` (`gpu.frames_discarded`, metric id 11) and warn event `RETAINED_FRAME_DISCARDED` (`gpu.retained_frame_discarded`, event id 6, field `target`, once per painter). The `gpu.submit` histogram now measures finish plus submit only, without producer `submitted` callbacks.

### Checked by the boundary script

`python3 scripts/check-engine-boundary.py` fails when a public signature, field, re-export, type alias, `use wgpu::..` alias used in a public item, type header (generic defaults, `where`), enum payload, trait method or associated type, or trait impl on a public type of nana-gpu, nana-frame-exchange, nana-ui, nana-ui-vue or nana-ui-devtools names `wgpu` outside `cfg(feature = "wgpu-interop")` (`not(..)` / `any(..)` do not count), and when anything but those crates' own sources uses `nana_gpu::__framework`.

`nana_ui_devtools::offscreen::FORMAT` is now a `GpuTextureFormat`, and `offscreen::readback(&GpuContext, &GpuTexture, Size)` replaces `readback(&Device, &Queue, CommandEncoder, &Texture, Size)`. `nana_frame_exchange` re-exports `GpuContext`, `GpuTexture`, `GpuTextureFormat` and `DeviceGeneration`.
