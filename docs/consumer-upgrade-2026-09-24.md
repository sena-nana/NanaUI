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
- `TextEditOrigin` gains `Cut` and `Step` (a numeric field's arrow key or spinner; a run of steps merges into one undo step). Exhaustive matches add both arms.
- `AppContext::focused_text_editor` also reports a focused `NumberInput` (its draft). `FocusedTextEditor::is_numeric()` identifies it. A host that routes editor keys itself steps the field on plain ArrowUp/ArrowDown (`step_focused_number_input`) and commits it on Enter (`commit_focused_number_input`) instead of moving the caret or submitting. Enter with Shift or Alt commits too, and Alt+ArrowUp/Down move the caret as in any single-line field. It consumes the arrows even when nothing moves, so they never reach an enclosing table or tree. An Enter that commits nothing is not consumed, so a dialog or form can still confirm on it. `RuntimeInputAdapter` already does this.
- New `AppContext::focused_text_editor_composing`: a plain editor (TextArea, TextInput, NumberInput) is focused with an IME composition in progress. While it holds, `RuntimeInputAdapter` consumes the editor's caret-navigation keys (arrows, Home/End, PageUp/PageDown), so a composing field inside a table, tree or select no longer loses focus to that navigation. Composite search surfaces (command palette, search dropdown, context menu) keep their list navigation while composing.
- `AppContext::replace_focused_text` (typing) refuses text carrying the control characters hosts report with command keys (`"\r"` for Enter, `"\u{1b}"` for Escape, `"\u{8}"` for Backspace) and returns `false`; Tab and newline still type, and paste keeps its text as it is. Before, an Enter or Escape a NumberInput did not use landed in its draft, through `RuntimeInputAdapter` or any host that forwards key text.
- `NumberSteppers::contains(x, y)` hits either spinner half whether or not it is enabled.
- `NumberFieldSpec` grids are displayable by construction. The grid starts at the minimum rounded up to the precision and moves by the new `display_step()`: the step, but never finer than one unit of the precision (a step of 0.05 at one decimal place moves by 0.1, and a NumberInput publishes that step). `snap` rounds onto that grid and clamps to the last point inside the maximum (10.3 on a whole-number grid holds at 10), so snapping is idempotent, every value the display can show inside the bounds is reachable, and what `format` shows `parse` reads back. Before, `snap(11)` returned 10.3 and `snap(10.3)` returned 10, a minimum off the precision drifted, and a step finer than the precision froze partway. `step_by`, `can_increment` and `can_decrement` move and ask on the same grid. Bounds are read as the field displays them: a bound within its float noise of a displayable number is that number (an f32 0.7 is 0.69999998, as a Vue binding's bounds are; the field stores 0.7), otherwise the nearest displayable number inside it. New `NumberFieldSpec::within_bounds` applies the same tolerance.
- A `NumberInput` builder chain (`new(v).range(..).step(..).precision(..)`) normalizes the requested value against the finished rules, not against each half-built step: `new(0.0).range(0.1, 5.0).step(0.1).precision(1)` starts at 0.1.
- `nana_ui_core::NumberFieldSpec::parse_unsnapped(text)` reads the number a draft spells before bounds or the grid apply; `parse` is it plus `snap`.

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
- **NumberInput editing:** the draft is a full editor.
  - Undo and redo reach it. Committing (Enter or blur) and reverting (Escape) are each their own undo step; a commit also ends a run of steps when the draft already read the number. A run of steps (a held arrow key, repeated spinner presses) is one. `set_number_value` and accessibility `SetValue` clear the history whenever they change the field, even when the draft already read the new number. Undo and redo restore the draft and the committed number the field held at that point, which the history records with the text (emitting `NumberChanged` when the number moves). Undoing a step takes the number back even when the draft was empty; undoing or redoing typing never commits the typed number. Only edits that change the text are undo steps: a commit whose draft already read the number, or a run of steps that ends on the text it started from, rides on the edit before it (redo returns to the number it left), and still ends the redo branch and the typing run. With no edit before it (a fresh or cleared history) it is an undo step of its own.
  - Steps, spinner presses, Enter (`commit_number_input`), Escape and accessibility `SetValue` do nothing while an IME composition is in progress. Moving focus away cancels the composition first and then commits the draft beneath it; the composition owns the draft until it commits or cancels.
  - `set_number_value` with the number the field already holds changes nothing: a controlled field echoing `NumberChanged` back keeps the user's draft and undo history. Before, it rewrote the draft whenever the text differed.
  - A step starts from the typed draft when it parses (`21` then ArrowUp gives `22`). Before, it started from the committed value and discarded the draft. A draft that does not parse still steps from the committed value. The field's accessible `numeric_value` is this same number (the parsed draft clamped to the bounds, or the committed value), so the spinner's enabled halves, a spinner press and the arrow keys agree. A step that cannot move away from that number (including one a bound off the grid snaps back) changes nothing, even when the committed value differs. The published `numeric_minimum` / `numeric_maximum` are the reachable bounds on the field's grid (a maximum of 10 on a grid of 3 publishes 9).
  - Left/Right, word deletes, Shift+ArrowUp/Down selection and pointer caret placement work in it. A press on the spinner places no caret, including on a half that cannot move.
  - A read-only field's text can be selected and copied, and its spinner halves are drawn inert (`increment_enabled` / `decrement_enabled` false) for any read-only numeric field, since no step moves it.
  - The draft never takes control characters: a newline or tab typed, pasted or committed by an IME is dropped and the rest kept (a pasted `12\r\n` inserts `12`), and a whole-value edit such as find and replace that would add one is refused.
  - A hover card set to preserve editor focus keeps a focused NumberInput editing, as it does a TextInput.
  - Accessibility `Click` focuses it, `SetSelection` selects in the draft, and `SetValue` commits a number within the field's bounds, snapped to its grid, shows it in place of any draft (emitting `NumberChanged` when it moves), and reports success. A number outside the bounds (beyond a millionth of a step of float noise, so a client's `0.2 + 0.1` still reaches a maximum of 0.3), or text that is not a number, is refused and changes nothing, the pending draft included. Before, all three returned `false`.
  - `NumberChanged` is emitted only when the number moves. Before, `set_number_value` and a step emitted it whenever the draft was rewritten, even with the same number.
- **Android:** the IME buffer is the session's committed text with the preedit in place of the selection it stands for (`display_text()`, not the masked or folded text the editor draws); before, the preedit was inserted next to the focus.
  - The selection and composing region cross to GameTextInput in UTF-16 code units, the Java side's indices. Before, UTF-8 byte offsets were passed through as they were, which put the IME's composing region and cursor in the wrong place in any non-ASCII text.
- **Folding:** pressing Right across a collapsed fold leaves a caret, not a selection covering the hidden lines.
- **Undo after undo:** typing after an undo or redo starts a new step instead of merging into the step the undo stepped back onto.
- **Application value writes clear undo:** writing an editor's text from outside the edit path (`update_component`, `set_component`, a keyed `mount`, or committing `SetTextInput`) now clears that editor's undo/redo when the bytes change. Before, the journal survived, and Ctrl+Z after loading another document restored the previous one; `clear_text_history` is no longer needed for that. Writing back the value the editor reported, or the same text, keeps the journal.
  - A handler registered with `on` for the editor's own `TextChanged` that rewrites the editor during delivery (uppercasing, filtering) is part of how that editor takes input: the step records the text the handler left, and stays undoable. A step that ends up changing nothing is not kept. Whether a draft cleared on send stays undoable is the application's call: call `clear_text_history` when sending if it should not.
  - This includes writes an application makes on the user's behalf, such as clearing a field after send or a formatter's write-back. To keep such an edit undoable, use the new `AppContext::edit_text_area(entity, range, text)` / `edit_text_input(…)`, which replaces `range` with `text` along the path the user's own edits take, as a step of its own that never merges with the typing around it. Like the user's own edits, it is refused on a read-only or disabled editor or while an IME composition is in progress, a `TextInput`'s length limit applies, an atom the range reaches into is replaced whole, and every cursor, the user's own included, moves through the edit rather than to it, except that a caret right where text is inserted goes past it, as typing leaves it (a completion at the caret). It is not typing into a snippet: linked placeholders do not mirror it, and an active snippet session is remapped through it (or ends) as for any value change. An edit past a `TextInput`'s length limit is refused whole, never cut to fit. Replacing text with the same text is no edit and emits nothing; a range outside the text or off a grapheme boundary is an error.
  - Only `TextInput` and `TextArea` keep an undo journal. `NumberInput`, `SearchDropdown`, `CommandPalette` and `ContextMenu` inputs used to record steps that `undo_focused_text` could never take, so `can_undo_text` reported an undo that did nothing; they now record none. Their state is more than their text (a committed number, a filtered list), which a restored text would not bring back.
  - A `ColorField`'s hex text is rewritten only when it names a different color (or none), so any spelling of the current color the user typed survives reopening the picker, and so does its undo.
- **Cut and copy over an atom:** a selection that cuts into an atom copies the whole atom, which is what a cut deletes.
  - A bare primary caret inside an atom is not widened by a cut: the cut deletes only what it copied, and the atom stays. Other edits at such a caret, including `replace_text_area_selection(area, "")`, still replace the atom.
  - Every cursor is widened over the atoms it cuts into, not only the primary: a further selection that reaches into a chip replaces, cuts or copies it whole. Before, it left half of the chip behind.
- **Removing an overlay:** focus goes back to the overlay's restore target only when it left with the overlay (it was on the overlay, anything inside it, or an overlay opened from inside it and hosted elsewhere). If the user had already moved focus to another node, such as an editor mid-composition, focus and composition stay where they are. Before, the host took focus back and the editor's preedit was cancelled. A document with no focus still gets the opener back.
- **Inlays and word movement:** WordRight that stops inside an inlay label, from its anchor or from before it, lands where it would over the bare text instead of counting the label's words or stepping one grapheme past the anchor. A step that would land in a collapsed fold's hidden lines crosses the fold.
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
