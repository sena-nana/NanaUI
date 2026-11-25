# Application API

This page is for looking up entry points. If you are writing your first application, start with [Quick start](../guide/quick-start.md) and [How the framework runs](how-it-works.md). rustdoc is authoritative for signatures. This page does not copy every type.

## What to depend on

| Consumer | crate / package | Entry points |
| --- | --- | --- |
| New desktop UI | `nana-ui` (feature `hosted`) | `nana_ui::runtime`, `ApplicationState`, `RuntimeApplication`, `NanaApplication::builder` / `run_runtime` |
| Window settings / input types | Usually re-exported through `nana-ui`; use `nana-ui-platform` directly when needed (the input contract itself lives in `nana-ui-input`) | `WindowDescriptor`, `WindowHandle`, `CanonicalInputEvent`, `InputPayload`, `InputEndpoint`, `HostServices` |

Do not depend on `nana-ui-devtools` or `nana-css-parity` directly to draw product UI. The former is headless debugging. The latter is CSS parity testing.

### Input: one route

For the long-term conformance matrix and reproducible commands, see [Issue 243 input conformance](input-conformance.md).

All input has exactly one shape: `CanonicalInputEvent` (`nana-ui-input`, re-exported through `nana-ui-platform` and
`nana-ui`). Native windows, devtools, Android, and tests each lower native events into
it once, at their own boundary. After that there is one route:

```text
Host lowering → InputSequencer stamps (source / device / generation / sequence / timestamp)
        → InputEndpoint (bounded; adjacent pointer moves and wheels coalesce)
        → AppContext::drain_input / route_input (Runtime, document bound to the source)
        → RoutedEvent → host → RuntimeProgram::input_event(RoutedInput)
```

- **Payloads**: `Pointer`, `PointerEnter` / `PointerLeave`, `Wheel`, `Key` (the physical key is the W3C
  `code`, such as `KeyA`; the logical key is the name after layout resolution), `Text(CommittedText)`, `Composition`,
  `Focus`, and device and source connect/disconnect. A key press and the text it types are two events. The text carries the key's
  sequence: when a control handles the key (shortcut, focus traversal, form submit, terminal already sent bytes), the
  route drops that text. It is not inserted again. IME composition goes only through `Composition`. It is never derived from key names. The application key policy
  (`AppContext::on_key`) and keymap (`KeymapLayer::resolve_key`) read this same
  `KeyInput`: the physical key stays in `physical`, shortcuts match on `logical`, and the four modifier bits are copied verbatim into
  the stored `KeyModifiers`. This is the only public keyboard event. `FileDrag(FileDragInput)`
  is a file dragged in from outside the window (hover, drop, cancel). The route hands it to registered drop targets,
  emitting `FileDropEvent` as well as `DragDropEvent` carrying an external reference-only `DataTransfer` (`text/uri-list`,
  without eagerly reading file contents).
  `RoutedInput::pointer_hit` is the target it is over or dropped on. A program that wants to accept files on the whole window
  reads the paths of `FileDragKind::Drop` in `input_event`.
- **Internal typed drag sessions (`DragSource` -> `DropTarget`)**: nodes declare an internal drag source via
  `AppContext::set_drag_source` / `El::drag_source` (supporting `DropKind::Custom` and `DropEffects` for `Copy` / `Move` / `Link`),
  and drop targets declare accepted `DropKind`, `DataFormat`, and actions via `set_drop_target` / `El::drop_target`. Once pointer travel
  crosses the slop threshold, `DragSource` produces a `TransferOrigin::Internal` `DataTransfer` (which may carry a process-local
  `InternalPayloadToken`-verified typed object without leaking raw pointers/OS handles), emitting `DragSourceEvent` on the source
  and `DragDropEvent::{Entered, Moved, Left, Dropped}` on the deepest accepting target. `Escape`, pointer cancel, window blur/close,
  or source node destruction cleanly aborts the session and invalidates lazy providers.
- **Binding**: `AppContext::bind_input_source(source, generation, document)` returns
  `Result`. A generation that goes backward, or a different document for the same generation, is rejected (`InputBindError`).
  `unbind_input_source` first runs any pointer that is still pressed or captured through the same cancel as an explicit `PointerCancel`,
  then revokes capture and hover. A window that is closed and reopened uses a newer generation. Late events from the old one are not accepted.
- **Routing state belongs to the context**: each source's pointer identity, ordering, disconnect flag, cursor, and text input slot
  live in the `AppContext` it is bound to. Two windows never share a pointer, IME owner, or cursor. An event's time comes from
  its own timestamp (the Runtime animation clock domain). Tooltip delays, hover probing, and multi-click detection all use that time.
- **Focus**: when a window loses focus, only that source's pressed and captured pointers and the file drag it is hovering are canceled. The document's focused control is unchanged. On regaining
  focus, the text input state is handed to the host once more. The document focus is cleared only when the source disconnects and no other focused source
  drives the same document.
- **Hit testing**: an uncaptured pointer event runs exactly one hit query (counted by `UiWorld::hit_test_queries`). Overlay
  routing, hover, diagnostic hover, handle probing, cursor, and `RoutedInput::pointer_hit` share that one result.
  Captured pointer events run no hit query. Steady pointer movement (including movement that changes hover every time) does not allocate.
- **Drain**: `drain_input` takes events out of the endpoint and then routes them. No event gets stuck at the head of the queue. Native hosts drain
  state transitions (press, release, key, text, composition, focus) immediately. Pointer moves and wheel drain at the end of the current
  event-loop iteration or before the next redraw. The program therefore sees one coalesced move per iteration.
- **Outcome**: `InputRouteOutcome` reports `handled`, `prevent_default`, `pointer_hit`, and
  `invalidated_work`. `handled` and `prevent_default` are independent: a blocking overlay can prevent the host's
  default behavior without meaning a control handled the event. `AppContext::input_counters()` reports counts of routed, rejected, and hover-change events,
  cursor and text input updates, and dropped text. Hit queries and focus and capture changes go only into diagnostic metrics.

Host capabilities go through `HostServices` (also in `nana-ui-input`):

- **Cursor** and **text input (IME)** are "latest value" slots, not request queues: Runtime calls
  `set_cursor(CursorIcon)` and `set_text_input(Option<&TextInputContext>)` only when the value changes. When the window regains focus,
  the text input state is sent once more. `TextInputContext` carries the purpose (normal, password, terminal), the candidate window anchor (the caret,
  in logical coordinates), and a text window of at most `SurroundingText::MAX_BYTES` (4000 bytes) around the selection. However large the field is,
  only this window is copied. Password and terminal fields provide no surrounding text. Native hosts combine Runtime's cursor with the window-border resize
  cursor and the program's `WindowCursor` override before setting it on the window.
- **Clipboard & `DataTransfer`** is the only call with a return value. Alongside the text convenience methods
  (`read_clipboard` / `write_clipboard`), `HostServices` and `ClipboardHost` converge on `DataTransfer`
  (`read_clipboard_transfer` / `write_clipboard_transfer`, plus image helpers `read_clipboard_image` / `write_clipboard_image`)
  covering `text/plain`, `text/html`, `text/uri-list`, `image/png`, `image/rgba8`, `application/octet-stream`, and validated custom
  MIME formats with lazy `TransferProvider` materialization and fail-closed cancellation/size bounds. Desktop hosts
  (`nana-window::NativeClipboard`) map multi-format and lazy reads onto macOS (`NSPasteboard`), Windows (Win32 Clipboard /
  `CF_HTML` / `CF_HDROP`), and Linux (Wayland `wl-paste`/`wl-copy` or X11 `xclip`, reporting `Unsupported` when unavailable),
  opening the system clipboard only once per process. It happens synchronously at the point in the key chain where the copy/cut/paste
  shortcut originally sat: the application's key policy (`on_key`) and the terminal see the key first. The host does not wait on a busy backend. It returns `HostServiceError::Busy`.
  When the clipboard refuses a write, cut does not delete text.
  `clipboard_change_count()` reports the clipboard's change sequence number: every write from any process advances it; reads do not.
  macOS reads `NSPasteboard.changeCount`, Windows reads `GetClipboardSequenceNumber` (0 means not visible).
  The default implementation, X11/Wayland, Android, and OHOS report `Unsupported`; `HeadlessHostServices` counts its own private clipboard,
  and `set_clipboard` / `set_clipboard_transfer` also count as an external write. A rich text editor records it after copying and pastes back its in-process rich text only when the number matches;
  when unsupported, it matches by text.
- `HeadlessInput` is an input source with no window (devtools, tests, offscreen test tools). It comes with
  `HeadlessHostServices`, which stores the cursor and IME state the host would have shown, and a private clipboard.
  `UnsupportedHostServices` is for hosts that do not need these capabilities.

Still out of scope, with no headless result standing in for them: authentication/replay/rate limiting of remote sources; synchronization of cross-thread endpoints
and process-wide memory quotas; a formal FFI/serialization version of the canonical payload (the Rust enum layout is not a wire format);
coordinate transforms for nested NanaUI (#251, added together with the real embedded producer from #247); and device acceptance for real Windows IME,
Android, XR, and accessibility providers.

New code imports controls from `nana_ui::runtime`. The crate-root control compatibility surface has been removed. `runtime::internal` is only for the Gallery, host adapters, and migration checks; it is hidden from docs and is not a second product API. `runtime::host` holds Scene / GPU slot types. `runtime::perf` is frame counters, not view state.

### Root-level compatibility entry point migration

| Old entry point | New entry point | Status |
| --- | --- | --- |
| `nana_ui::Button`, `Text`, `Workspace`, and other root-level controls | Removed | Use `nana_ui::runtime::{Button, Text, Workspace, ...}` |
| `nana_ui::Textarea` | Removed | Use `nana_ui::runtime::TextArea` |
| `nana_ui::components::*` | Compatibility surface (hidden from docs) | New code uses `nana_ui::runtime::*`; the old path is only for migration and catalog checks |
| `AppContext::world_mut()` / `compat_world_mut()` | Removed | Use `commit_mutations` and typed `AppContext` methods |
| `nana_ui::dock::*` | `nana_ui::runtime::{Dock, DockWorkspace, ...}` + host adapter | The Dock tree belongs only to Runtime |

The `world_mut` and `Textarea` compatibility entry points have been removed. New capabilities may only go into `runtime`
or an explicit host compatibility module.

### Capability authority

Each item has exactly one owner and one recommended call. Action/Menu conversion and dispatch (`ActionRegistry` + `Keymap` + `MenuActionMap` + `AppContext`) are settled, Theme is accepted under #108 / #110, and the migration guide for the recommended style under #291.

| Capability | Owner and recommended call | Retained entry point | Removal condition |
| --- | --- | --- | --- |
| Component update | `update_component` / `set_component` / `create_component` | `create_view` (`doc(hidden)` host primitive; does not run the component projection) | Host adapters no longer need unprojected placeholder nodes |
| CSS color | `nana-ui-css` parses strings. PaintScript accepts only semantic roles and typed RGBA | Caller-supplied color parser | Runtime never gets a second CSS parser |
| Virtual list / table / tree | `materialize_virtual_*_retained_in` | None (legacy `materialize_virtual_*` removed) | Removed; `scripts/check-api-convergence.py` rejects reintroduction |
| Query editing | `TextInputState`, through `query_text()` / `set_query()` | No second query field | The query authority stays this one |
| Virtual row identity | `each_virtual::Unit::Row { columns, keys }` | No hash identity | Row identity stays a structured key |
| Event subscription | The host's own stream | None (`Subscription<T>` removed) | Removed; `scripts/check-api-convergence.py` rejects `Subscription::new` |
| GPU surface | `HostedGpuShared::resize_surface` / `prepare_surface_frame` | `HostedGpuSurface::resize` / `prepare_frame` (`doc(hidden)` advanced) | Kept while some host still holds the surface and device separately |
| Default GPU renderer | `scene_gpu_renderers`: neither `None` nor an empty list installs the demo renderer. Return `default_scene_gpu_renderers()` explicitly when needed | No implicit `"gpu-view"` | The demo painter stays opt-in |

### Actions, shortcuts, and menu dispatch

There is exactly one `ActionDescriptor`, defined in Runtime (`nana_ui::runtime`; `nana_ui` re-exports the same type): the keymap reads `id` / `enabled` / `when`, while command palettes and menu adapters also read `label` / `category` / `keywords`. Hosts that only bind shortcuts use `ActionDescriptor::new(id)`, and actions that belong in palettes or menus use `ActionDescriptor::labeled(id, label)`.

- **ID normalization**: `ActionId::normalized()` trims surrounding whitespace and rejects empty or whitespace-only IDs across `ActionRegistry`, `KeyBinding`, `Keymap`, `MenuActionMap`, and `AppContext` registration, binding, and lookup.
- **Single registry and dynamic bindings**: `ActionRegistry` preserves registration order. Beyond static `set_enabled` / `set_label`, `bind_enabled` / `clear_enabled_binding` and `bind_label` / `clear_label_binding` compute live availability and labels from context and application state (such as dynamic undo/redo labels paired with `store_with_history`), read via `resolved_label`. `is_available`, `available`, `search`, `palette_items`, `action_menu_item`, and `MenuActionMap::sync_menu_bar` share this same evaluation.
- **Thin DTO adapters**:
  - `ActionRegistry::palette_items(query, context, keymap)` builds `nana_ui::runtime::CommandPaletteItem` entries.
  - `ActionRegistry::action_menu_item(id, context, keymap)` builds one `nana_ui::runtime::ActionMenuItem` with resolved label, `.disabled(...)`, and shortcut hint (formatted via `Keymap::display_binding`).
  - `MenuActionMap` maintains a `u32 <-> ActionId` bijection (`bind` / `unbind_menu_id` / `unbind_action` / `action_for` / `menu_id_for`), constructs a `MenuEntry` via `menu_entry`, and keeps `MenuBar` labels, enabled states, and platform shortcuts in sync via `sync_menu_bar` (using `Keymap::menu_shortcut`).
- **`AppContext` dispatch and route ordering**:
  - `bind_action_handler` / `unbind_action_handler` / `has_action_handler` register handlers (`register_action` remains for handler-level `when` predicates); `can_dispatch_registered_action` checks both registry availability and handler presence; `dispatch_registered_action` is the single adapter that validates registry availability before invoking the handler.
  - `dispatch_keymap_key` and `dispatch_routed_key` connect keyboard input to `Keymap::resolve` and `dispatch_registered_action`. `dispatch_routed_key` checks `disposition.prevent_default` first so modal overlay barriers, active IME composition, an open `AppMenuBar`, `AppContext::on_key`, and focused text editors consume their handled keys before global keymap shortcuts fire.
  - `dispatch_menu_action` (resolving numeric `menu_id` via `MenuActionMap`) and `dispatch_palette_event` (handling `Select`) also dispatch through `dispatch_registered_action`.
- **Document key context**: `AppContext::document_key_context(document, base)` augments `base` with `CONTEXT_MODAL`, `CONTEXT_TEXT_INPUT`, and `CONTEXT_IME_COMPOSING` from the document's live Runtime state.

## Cargo features

`nana-ui` has `default = []` (no features are enabled by default). Enabling `gpu` (directly or via `hosted`) pulls in only the platform's primary backend; enable individual capabilities explicitly or use `full`. See [Layout, Feature Defaults, and Runtime Migration](../guide/migration-layout-and-features.md):

| feature | Effect |
| --- | --- |
| `hosted` | `run_runtime`, winit 0.31-line (workspace git pin, not crates.io 0.31.0), AccessKit; implies `gpu` |
| `gpu` | `SceneWgpuPainter`, `HostTexture`, `GpuTextureView`, `GpuView` (platform primary backend only) |
| `wgpu-backends` | Enables all standard cross-platform WGPU backends (`dx12`, `vulkan`, `gles`, `metal`) when `gpu` is enabled; included in `full` |
| `runtime-wgsl` | Runtime WGSL shader compilation fallback (`naga` / `wgpu` WGSL); when disabled, loads precompiled pipelines only and emits `gpu.shader.build_failed` if built-in stages are missing or rejected, and `gpu.material.rejected` for custom WGSL `MaterialProgram`s |
| `bundled-fonts` | Embeds the Noto Sans SC variable font (`wght` 400–700, about 4.2 MiB) and registers it as the UI font. Without it interface text and SVG text use system fonts; see [Look](look.md#bundled-font-or-system-fonts) for what is traded |
| `components` | Aggregate of the component families below (including `qr-code`) |
| `full` | `bundled-fonts` + `components` + `hosted` + `http-fetch` + `icu` + `image-codecs-extra` + `runtime-wgsl` + `snippet-transforms` + `svg` + `syntax-highlighting` + `wgpu-backends` |
| `icu` | Opt-in ICU4X locale formatting; falls back to `PlainFormatter` when disabled and emits `resource.icu_unavailable` on first localized formatting use |
| `http-fetch` | Opt-in `nana-ui-platform/fetch` for `http(s)://` fetches; without it or a custom `FetchHost`, remote requests are skipped and emit `resource.fetch_unavailable` |
| `image-codecs-extra` | Opt-in JPEG / GIF / WebP decoding in `image`; when disabled, PNG stays available and excluded formats emit `resource.image_codec_unavailable` |
| `svg` | Opt-in SVG images: SVG documents in `url()` textures and `Image`, and a host icon wrapped with an SVG and no shapes. Icons are drawn from their own geometry and do not need it. When disabled, an SVG image is treated as unreadable with `resource.image_codec_unavailable`, and `usvg`, `resvg`, and `tiny-skia` (about 0.8 MiB) are not linked. Effective only with `gpu` |
| `snippet-transforms` | Opt-in regular-expression transforms in snippets (`${1/(.*)/${1:/upcase}/}`). When disabled, a snippet still parses, expands and links its placeholders, transforms return values unchanged with a one-shot `runtime.snippet.transform_unavailable` event, and `fancy-regex` (about 0.7 MiB) is not linked |
| `area-layout` / `calendar` / `charts` / `controls` / `diff-view` / `dock` / `graph-canvas` / `image-viewer` / `paint-script` / `property-grid` / `qr-code` / `rich-text` / `terminal` / `timeline` | Enables the matching optional component family and re-exports it (`calendar`, `charts`, `controls`, `graph-canvas`, `image-viewer`, `rich-text` forward to both `nana-ui-runtime` and `nana-ui-scene`; `area-layout`, `diff-view`, `dock`, `paint-script`, `property-grid`, `qr-code`, `terminal`, `timeline` gate runtime-only clusters, with `dock` also carrying the host adapters `nana_ui::dock` and `runtime_dock_window_update`). When disabled, the family is not compiled. Historical no-op aliases (`overlays`, `selects`, etc.) have been removed; use `components` / `full` |
| `syntax-highlighting` | The `"highlight"` presenter for `TextArea` |
| `no-default-app-icon` | The application always registers its own icon: the default icon marker is not linked, and windows without an icon keep the platform default. Enabled only by applications |
| `accesskit-tree` | Exports the TreeUpdate projection of `AccessTreeProjector` for hosts that wire their own platform adapter (such as Android `accesskit_android`); implied by `hosted` |

Cargo does not enable `calendar` just because you wrote `CalendarHeatmap`.

## Application identity, paths, and diagnostics

Process-wide settings go through `NanaApplication::builder(ApplicationIdentity)`: it resolves and publishes `ApplicationPaths` (available anywhere through `NanaApplication::paths()`). The optional `.diagnostics(DiagnosticsConfig::default())` turns on structured diagnostics. Then call `.run::<P>(WindowDescriptor)`. `run_runtime` can still be used directly; it just sets no paths and enables no diagnostics. Business code should not build `./runtime/...` or platform directories itself. Take logical locations from `ApplicationPaths` (`runtime_resources`, `data`, `config`, `cache`, `logs`, `crash`, etc.). `app_data_dir` is kept. Under the installed layout on desktop platforms it equals `ApplicationPaths::data` (in the portable build, data is at `<root>/data`). See [Diagnostics and application paths](diagnostics.md).

Packaged applications declare their identity with `nana_ui_platform::application_identity!`: it leaves an identity marker in the binary. Both the packager and the validator compare against it. With the `packaged-resources` feature enabled, calling `.resource_packs(ResourcePackOptions)` reads the package manifest and mounts `.nrpack` resource packs at `nana://res/<logical path>`. Styles, `@font-face`, and `url()` can reference them directly. In installed and portable builds, a relative `url()` with no explicit base resolves against `runtime_resources`, no longer against the CWD. See [Packaging and distribution](packaging.md).

## RuntimeProgram

Ordinary Rust applications should implement `ApplicationState` first. `RuntimeApplication<State>` manages
each window's `RuntimeDocument` and the resource registry. For a minimal runnable program, see
`crates/nana-ui/examples/application-counter.rs`.
The lower-level contract below is for embedded hosts that manage windows/documents themselves.

When controls are built from typed Rust builders, `view!`, or `.vue`, built-in controls enter `ComponentRegistry` on demand where referenced (unused built-in controls are tree-shaken out of the final binary); custom controls register through `ComponentView` and `RegisterableComponent`. See [Layout Model, Feature Defaults, and Runtime Migration](../guide/migration-layout-and-features.md#4-removed-public-apis).

`ApplicationWindow::demand` and the low-level `RuntimeProgram::frame_demand` use the same
`FrameDemand`: `OnDemand` by default, a one-shot deadline `At(Instant)`, or continuous refresh
`Continuous(NonZeroU32)`. 120 requests 120Hz. It is not a guarantee of the display refresh rate.
The host merges `UiScene::compositor_needs_tick` into that window's present cadence.
The compositor overlay does not rewrite the program's own `frame_demand`, and it does not wake unrelated windows.
Compositor tracks, per-glyph reveal / looping effects, and chart transitions present at 120Hz by default on visible windows.
To cap a particular window, a program implements `RuntimeProgram::max_frame_rate` (`ApplicationWindow::max_frame_rate`):
after it returns `Some(30)`, neither the program's own `Continuous` nor the compositor ticks the framework adds exceed 30Hz.
These animations are evaluated against absolute time, so a cap only produces fewer frames; it does not slow them down. `At` deadlines, input, and explicit redraws are not limited by it.
The default `None` means no cap.
After device/surface loss the host calls `RuntimeDocument::set_surface_generation`.
Resource updates use `TextureSlot`. For detailed changes and the current acceptance scope, see
[High refresh refactor](https://github.com/sena-nana/NanaUI/tree/docs-archive/archive/docs-notes/high-refresh-refactor.md).

The application implements this trait, then calls `run_runtime::<App>(WindowDescriptor::new("…"))`.

| Method | Responsibility |
| --- | --- |
| `initialize` | Builds the program instance; may return messages to `update` on the first frame. The moment it is called is the startup `UiReady`: build only the first screen and hand heavy work to tasks; see [Two-phase startup](startup.md) |
| `startup_takeover` | Optional; whether the document built by `initialize` takes over the Early Splash immediately (default `Immediate`); `Deferred` waits for `context.startup().take_over(ticket)` |
| `startup_changed` | Optional; startup phase changes (requested, withdrawn, handoff complete) |
| `with_document` / `with_document_mut` | Hands out the `RuntimeDocument` for a `WindowId` inside an access closure |
| `update` | Host-level messages; keep it cheap |
| `theme` | Returns the registry-resolved `Arc<CompiledTheme>`; Light/Dark are prebuilt themes, identified by `ThemeId` |
| `window_material_mode` | Optional; solid by default |
| `host_textures` | Default; slot → `HostTexture` |
| `prepare_window_frame` | Prepares textures before flush. Still called when a `FrameDemand` comes due while the window is occluded or minimized, including at zero size; no flush, no present. Producer encode runs only when the size is drawable |
| `window_frame_presented` | Releases old resources after present. Not called for hidden GPU ticks |
| `scene_gpu_renderers` | Advanced. `None` = register no custom renderer (does not implicitly install the demo `"gpu-view"`); empty list = explicitly no custom renderer. Return `default_scene_gpu_renderers()` explicitly when you need the built-in demo painter |
| `scene_resource_producers` | Advanced. Offscreen per graph; can be ignored at first |
| `bind_window` | Fills content after present |
| `rebuild_gpu` | Rebinds resources after device loss |
| `window_event` | Window lifecycle. Both the system close and the title bar close button only send `CloseRequested`; the host does not close the window itself. The window closes only when you return `WindowCommand::Close(id)` or `exit()`. The default implementation closes immediately; programs that must save first can return it later |
| Set the menu via `WindowHandle::set_menu_bar` | Native menu bar; drain selected items each frame with `take_menu_activations()`; see [Window](window.md#menu-bar) |
| `input_event` | Raw input after Runtime dispatch; the only input hook. Its `RoutedInput` argument carries `event`, `pointer_hit` (pointer and wheel only), and `disposition`; consumed events are still delivered, so application shortcuts should check `disposition.prevent_default` |
| `next_wakeup` / `wake` | Timed work unrelated to redraw |
| `host_failure` | The host has recovered from this error; ignored by default |

`RuntimeProgramContext` provides `window_id`, `geometry`, `gpu()`, `material()`, `dispatch`, `run_task`, and `startup()` (startup record and takeover requests). Native window handles do not cross this boundary.

### Programs built in stages (`Staged`)

Some programs cannot be built yet when the host starts: they read settings and open devices on a worker thread, then mount pages one per iteration, while the splash screen keeps presenting in between. In that case implement `Launch` (`type Program` is the final program) and run `run_runtime::<Staged<MyLaunch>>(..)`:

- During construction, `Staged` answers the main window's document with `Launch::document` (other windows do not open yet), answers appearance with `Launch::theme_mode` / `window_material_mode` / `window_background`, and hands messages to `Launch::update`.
- After `Launch::update` returns `LaunchUpdate::Ready(program, update)`, `Staged` hands the animation clock to the program and from then on forwards **every** hook to it unchanged; hooks added to `RuntimeProgram` in the future are forwarded too, so the application does not forward them one by one.
- Splash screen: `Staged` answers `Deferred` to the host; once the program is built and its own `startup_takeover()` is `Immediate`, `Staged` requests takeover on its behalf. If the host has not issued a ticket yet, it asks again every 50 ms; past `Launch::TAKEOVER_TIMEOUT` (default 30 s) it calls `Launch::takeover_failed` (exits by default). When the program answers `Deferred`, it takes over itself.
- The associated functions `gpu_backend_policy` / `startup_window_material_mode` come from the final program.

### Running a whole program without a window (`HeadlessRuntime`)

Offscreen acceptance has to run the real program, not a document extracted from it. `nana_ui::headless::HeadlessRuntime<P>` calls `P::initialize` with a `GpuContext` and the main window geometry, then drives the hooks in the Scene host's order; the program holds its own documents, and the host reads and writes them through `with_document` instead of moving a document into a session and back.

- `turn()` is one host iteration (returning `HeadlessTurn { work: WorkCounters, did_work: bool }`): window requests, queued messages (`update`), due `wake`s and animation frames, then one frame per visible window: `prepare_window_frame`, layout, `window_frame_presented`, and `bind_window` once there are messages (`present(id)` returns `Option<WorkCounters>` for a single frame). The main window is bound once after the first frame, which corresponds to the message that installs the program in the native host.
- `input(id, payload, text)` routes into the document through that window's own input source, then hands off to `input_event`; `accessibility_action`, `window_event`, and `resize` likewise go through hooks. `update`, `dispatch`, and `wake` let an acceptance script add a message or a wakeup.
- Windows: `WindowService::create_window` and `WindowCommand::Open` create a virtual window and call `initialize_window`; `set_visible` and `set_size` change it and send the matching events; `Close` closes it and its child windows. Other commands only go into `take_window_commands()`. File dialogs immediately answer `FileDialogCompleted` with the answer set by `set_file_dialog`, which reverts to cancel after one use.
- `take_report()` reports the merged redraw target, whether exit was requested, and how many times messages, wakes, input, and window events requested a redraw (frame callbacks are not counted; `set_quiet_messages` can exclude messages that arrive every frame).
- It draws no pixels. `nana_ui_devtools::headless::HeadlessHost` (feature `headless-host`) adds a painter and readback on top of it: `screenshot(id, clear)` first produces a frame, then draws, waits for images to finish decoding, and hands the learned image sizes back to the document; it also provides `accessibility`, `click`, `mouse`, `key`, `type_text`, and `click_node`.

Tasks from `run_task` run on the host's worker threads. When a task panics, no message is returned: the host records a `host.task_panicked` fault and the worker thread keeps processing the queue. A caller waiting on a result must catch the failure inside the task and return a failure message.

`ApplicationState` passes through the following hooks. `RuntimeApplication<State>` forwards them unchanged to `RuntimeProgram`, so applications with transparent overlay windows or that need the compositor path do not have to hand-write another `RuntimeProgram` layer:

| `ApplicationState` method | Matching `RuntimeProgram` hook |
| --- | --- |
| `gpu_backend_policy()` (associated function) | `gpu_backend_policy` |
| `startup_window_material_mode()` (associated function) | `startup_window_material_mode` |
| `window_material_mode_for(&self, id)` | `window_material_mode_for` |
| `appearance_backdrop_opacity_for(&self, id)` | `appearance_backdrop_opacity_for` |
| `input_event(&mut self, id, input, windows, cx)` | `input_event`, plus the `ApplicationWindow` of every window |
| `animation_frame(&mut self, id, frame, windows, cx)` | `animation_frame`: the host has already written this frame's samples into the document of `id`; the application reads finished or canceled animations from `frame.events` |
| `next_wakeup(&self)` / `wake(&mut self, now, windows, cx)` | `next_wakeup` / `wake` |
| `close_requested(&mut self, id, windows, cx)` | `CloseRequested` from `window_event`. Returns `WindowCommand::Close(id)` by default; if the answer does not contain it, the window stays (confirm first, minimize to tray) and can be closed later from `update` once decided |

Rich text: `nana_ui::runtime::rich` is the value vocabulary (`RichText`, `RichSpanStyle`, `RichTextStroke`, `RichTextShadow`, and the Style Model's `PaintColor`). The `RichTextView` component or `MutationQueue::set_rich_text(id, rich)` attaches it to a text node, and the cost depends on which layer changed: the shaping layer relays out text, the paint layer only repaints, and `effect` does no text work. See [RichTextView](../components/rich-text-view.md) and [Text engine](text-engine.md#rich-text-spans). For editing use [RichTextEditor](../components/rich-text-editor.md): the toolbar calls `AppContext::rich_edit(entity, RichEditCommand)`, and the document is handed back to the application through `RichTextEditorEvent::Changed`. Per-glyph effects and typewriter reveals use `AppContext::set_rich_presentation(node, effects, reveal)`; it is presentation only, does no text work, and requests frames only while playing.

`AppContext::animation_now()` is the document's animation clock: the time the compositor last used to sample animations. Presentation the application schedules itself (typewriter reveals, the start of per-glyph effects) reads it, using the same clock as the compositor instead of a wall clock that drifts.

A document's presentation time can be set, played and paused (Issue #300): `set_presentation_playback(PresentationPlayback::Paused)` holds this one document at the current frame, `seek_presentation(t)` puts layout-class samples and the compositor clock at `t` together, `step_presentation(interval)` advances by a step, and `Playing { rate }` plays at a rate. At rate 1 without pausing it advances exactly as the wall clock did; input and delays keep the wall clock. A tour that stops at a step and a headless screenshot aimed at the middle of a transition both use it. See the [Runtime and Scene contract](https://github.com/sena-nana/NanaUI/blob/main/docs/en/reference/runtime-scene.md#document-presentation-clock-issue-300).

Video, image sequences and sprite frames sample frames at the document's time (Issue #303): give a `Video` or `GpuTextureView` `media_time(Some(MediaTimeMapping::new(origin, in_point)))`, and reversing or changing speed rewrites only that mapping; the host reads `RuntimeDocument::media_sample_requests()` in `prepare_window_frame`, publishes the frame for each `media_time`, and the window binds it with `FrameBinding::prepare_media` within half a frame. `MediaTransportBar::source(Some(node))` makes the progress bar show that node's resource time, and the application writes the `Seek` on release back into the mapping. See the [Runtime and Scene contract](https://github.com/sena-nana/NanaUI/blob/main/docs/en/reference/runtime-scene.md#resources-sampled-at-presentation-time-issue-303).

`RuntimeProgramUpdate.redraw` supports `None`, `Window(id)`, `Windows(ids)`, and `All`.
Merging partial updates keeps the actual set of windows. `RuntimeRedraw::for_windows` sorts and deduplicates.
`RuntimeRedraw::merge` and `RuntimeProgramUpdate::merge` are public: `All` absorbs the other side, `None` adds no windows, and window lists are unioned; the updates' window commands are concatenated in order, and exit is requested if either side requests it. The host itself also uses them to combine the updates from one callback into one.
`FrameworkError` converts to `String`, so in callbacks with `type Error = String` you can write `?` directly on document operations.
`RuntimeRedraw` now holds a window list. It implements only `Clone`, no longer `Copy`. Exhaustive matches must handle `Windows`.

### Realtime applications (Issue #177)

`examples/realtime-fixture` is a game-shaped application built only from the hooks above: an application-owned 60 Hz fixed update, display-driven presentation, keyboard axes, and dozens of moving objects under an ordinary NanaUI overlay. It adds no framework type. It is the evidence that NanaUI supplies common mechanics rather than an implicit game runtime.

```bash
cargo run -p realtime-fixture
```

```bash
cargo run -p realtime-fixture -- --evidence --bodies 64 --present-hz 120
```

`--evidence` runs a script (run 3 s, minimize 3 s, restore 2 s, resize 2 s, exit) and prints per-phase counters.

| Concern | The application owns | NanaUI provides |
| --- | --- | --- |
| Fixed update | `FixedClock`, `World::fixed_update(dt)`, the catch-up bound | `next_wakeup` / `wake(now)` deadlines |
| Presentation | When it wants frames: `Continuous(rate)` while running and visible, `OnDemand` when paused or hidden | Frame scheduling bounded by the display, occlusion and minimization |
| Input | `MoveX` / `MoveY` from physical keys in `input_event`; held keys released on focus loss | Canonical key events, routed to the overlay first and delivered with no focused control |
| State to pixels | Publishes a flattened copy after each step; a `SceneGpuRenderer` uploads it once per painted revision and draws every object in one instanced draw | One `GpuView` anchor in layout, clipping and document order; the shared device and frame encoder |
| Resources | Loads the sprite once | `HostTexture` slots; `DeviceGeneration` for realizing again on a new device |

Rules the fixture follows, and that a realtime program should follow:

- Restart the fixed clock from the `now` the host passes to `wake`, not from `Instant::now()`. A program has no clock of its own to trust (`HeadlessRuntime` simulates one), and a resumed clock should not owe the time it slept. To restart, return an instant already past from `next_wakeup`; the host wakes the program at once.
- Return no redraw from `wake`. The continuous frame demand presents whatever was published last; a redraw per fixed step would tie UI flushes to the simulation.
- On `VisibilityChanged { hidden: true }` and on a 0 × 0 `Resized` (how Windows reports a minimized window), stop the clock and return `OnDemand`. Otherwise the host keeps calling `prepare_window_frame` for hidden ticks at the requested rate.
- Return `Some(constant)` from `preparation_version` and `None` from `content_version`: preparation is reused and the pixels stay live. `None` from `preparation_version` makes every frame of the whole tree uncacheable.

Measured on an Apple M4 with an external 1080p display, debug build, 64 bodies, `--evidence`, 2026-11-20:

| Phase | Fixed steps/s | Wakeups/s | Frames/s | Instance uploads/s | Renderer prepares | Frames that changed the `UiWorld` |
| --- | --- | --- | --- | --- | --- | --- |
| Running (3 s) | 60.0 | 59.3 | 117.7 | 59.3 | 1 | 1 (first frame) |
| Minimized (3 s) | 0 | 0.3 | 0.3 | 0.3 | 0 | 0 |
| Restored (2 s) | 42.5 | 43.5 | 85.5 | 42.5 | 0 | 0 |
| Resized (2 s) | 60.0 | 60.5 | 119.5 | 60.0 | 1 | 1 (relayout) |

Of 764 presented frames, 2 changed the `UiWorld`. The run built 1 pipeline and 1 sprite texture, prepared twice (once per target size) and dropped no fixed step. With `--present-hz 30 --bodies 100` the fixed update still runs at 60.0/s while 29.6 frames/s are presented, and instance uploads follow painted frames (29.6/s), not published revisions. The only wakeup while minimized is the script's own deadline. The restored phase falls short of 60 because macOS reports the window visible again about 0.6 s after the restore command (the restore animation); the clock then restarts without catching up.

The headless gates in `examples/realtime-fixture/src/tests.rs` check the same contract on a simulated clock: 60 fixed steps per second at 30, 60, 144 and 240 Hz presentation with no style, layout, text or render-node work; the same `UiScene` and `UiWorld` generation across two seconds of simulation; one playfield anchor whatever the body count; pause, hide and 0 × 0 resume without owed steps; and a paint on the host's device where the same `UiScene` shows moved pixels with one pipeline, one preparation and one upload per painted revision. Windows, Linux and the evidence run in a release build were not measured.

What the fixture found about the presentation bridge (input for #171 and #185):

- The existing path is enough. The overlay paints over the playfield in document order, the playfield is clipped like any node, a fixed step mutates nothing in the `UiWorld`, and the whole workload is one batched draw from one node. No new presentation primitive or Scene type was needed.
- Gap: recording the draw needs `wgpu-interop` (`ScenePass::wgpu`) and `runtime-wgsl`. `GpuContext` already creates logical buffers and resource groups, but a custom renderer has no backend-neutral pipeline object or draw call. That is the one logical capability this workload needs from #185.
- Minor gap: `GpuView` always uses the `"gpu-view"` renderer key, so an application renderer takes that key over for the window and has to skip slots that are not its own.

## Tree building

```text
RuntimeDocument::new(DocumentId)
mount_view_root(document, || {
    column().gap(12).children(
        button("…").on_cx(|_, _: &Activate, cx| cx.dispatch_program_all(Msg)),
    )
})
mount { scope.child("key", …) }          // add/remove in a dynamic region
update_component(entity, |view, _| { … }) // change a single field
set_component(entity, Button::new(…))     // replace all props, keep interaction state
```

Mounting a view is the initial whole page (one commit). After that, bindings update specific points. `mount` is keyed subtree reconciliation, not a second renderer. Click handlers should not remount everything. For details, see [L3: Building UI in Rust](l3-authoring.md).

When rebuilding a whole component from application state, use `set_component`. Do not write
`*view = New()` inside `update_component`: that also overwrites interaction state owned by the runtime,
which shows up as a menu collapsing on every refresh or a filter field's caret being reset. `set_component` goes through `ComponentView::reconcile`, and
each component decides what survives (`Select` / `Dropdown` / `SearchDropdown` keep their open state and highlight;
`SearchDropdown` also keeps the query the user has typed and its caret). If the props really change, interaction state is still reset.

Rewriting a whole list from current values on refresh is fine: when the component equals its previous value and the closure queued no mutations or events,
`update_component` / `set_component` return immediately, with no projection and no commit. The application does not need to skip rows by fingerprint itself.
When the component itself has not changed but the world state it reads during projection has, use `reproject_component(entity)` to project it again.
An `update_component` with an empty closure no longer has that effect.

Regions that already hold entities and need to keep their state when switching
can use `reconcile_children(parent_id, &[child_id, ...]) -> Result<bool, FrameworkError>`.
It publishes the parent's complete child order. Omitted subtrees are parked, not destroyed. Valid cross-parent moves and
assembly on a parked parent are supported. The same order returns `false`. Missing nodes, duplicates, cycles, and cross-document moves
fail within the same Runtime transaction, without parking other children first. It does not create components or replace semantic
slots / overlay assembly interfaces. `mount` is still for dynamic regions that construct by key and destroy absent components.

`create_component` / `append_child` / `on` remain low-level primitives.

The public identity is `StableNodeId` / `Entity<V>`. Do not depend on the internal entity encoding.

## Extension controls

| Goal | Path |
| --- | --- |
| Participate in layout, hit testing, Scene | `UiExtension` + `register_component` |
| GPU content | `GpuTextureView` + host texture; for direct writes see `GpuView` |
| Change how one node looks | Attach a `Painter` to `NodeStyle::painter` (`Card::painter` / `Stack::painter` / `Panel::painter`); see below |
| Hover, drag, wheel | `NodePointerHooks` + `ExtensionRegistrar::register_pointer` (built-in controls write it in `TypeBehavior::pointer`); see below |

Dynamic dylibs are not supported.

### Node pointer (`NodePointerHooks`)

When a control handles the pointer over its whole box itself (charts, canvases), install a set of `NodePointerHooks` for its type: `moved` is called when the pointer passes over the node or while the node holds capture; `left` is called when the pointer leaves or the node is no longer reachable; `pressed` is a primary, middle, or secondary button press (`NodePointer::button`, numbered as in the DOM: 0 primary, 1 middle, 2 secondary), and returning `true` takes capture until the same button is released; controls that only handle the primary button return `false` for other buttons; `released` is the release of the button that took this capture; when `wheel` returns `false`, the wheel scrolls the container underneath as usual. All coordinates are the node's own layout px (the top-left of the box is `0, 0`). `NodePointer` also carries the held buttons (`buttons`, DOM bits), all modifiers (`modifiers`), pointer type, pressure and tilt, and the click count `click_count` (presses of the same button within 500 ms and 4 px; a double click is 2); `NodeWheel` carries all modifiers. `Chart` uses exactly this path.

### Node painting (`Painter`)

This is the equivalent of Qt's `paintEvent`: `paint(&self, cx)` draws below the child nodes.
`paint_over_children` draws above them. Calling `cx.draw_default()` inside draws
the node's original built-in appearance (the base class `paintEvent`). Not calling it replaces it entirely. Child nodes lay out and
paint as usual. Children with a negative `z-index` are also above `paint`.

`cx` records commands; it does not draw immediately: the same node is not re-recorded while (`paint_key()`, layout size, theme generation,
interaction state, font set) are unchanged. When it only moves, even the triangulation is reused. Write colors as
`SemanticColorRole` / `SemanticColorMix`, corner radii as `RadiusTier`, and shadows as
`ElevationRole`. `cx` resolves them against the current theme. Switching themes re-records automatically.

- State: `cx.state()` gives hovered, pressed, focus-visible, disabled, and selected. A state change re-records,
  so it does not have to be folded into `paint_key`. Hover only considers whether the node itself is the hit target. When the pointer is over a child,
  the parent's `hovered` is false. This matches the built-in interactive appearance.
- Text: `cx.measure_text(&text, max_width)` measures with the host's current layout engine (when no engine is wired,
  it estimates by em), returning `PaintTextMetrics` (overall width/height, `baseline`, `ascent` / `descent`,
  Canvas `actualBoundingBox*` ink bounds, and per-line `lines` metrics). It matches what `cx.text` draws.
  `PaintText` supports wrapping, a maximum line count, line height, letter spacing (`letter_spacing`), italics, font family
  (`family`, CSS `font-family` syntax; measuring and drawing read the same value), text decorations (`decoration`:
  underline / line-through / overline with color, `Solid`/`Double`/`Dotted`/`Dashed`/`Wavy` style, and thickness),
  text stroke (`stroke`), and multi-layer text shadows (`shadows`). Both fill and stroke accept gradients.
- Text-to-path: `cx.text_path(&text, origin, max_width)` (or `PaintPath::from_text`) converts shaped glyphs into a
  `PaintPath` via `nana-text`'s `OutlineSink` (`skrifa` outline scaling), ready for fill, stroke, clip, hit-testing,
  and path boolean operations.
- Paths: `PaintPath` supports lines, quadratic / cubic Béziers, `arc`, and Canvas-semantics
  `arc_to` (rounds any corner, including concave ones). NonZero / EvenOdd. Parameters are written as scalars like Canvas
  (the last argument of `arc` is the sweep angle, the same as `QPainterPath::arcTo`,
  not Canvas's end angle). `PaintPath::from_svg` builds a path from SVG path data. Path boolean operations
  `union` / `intersect` / `difference` / `xor` interpret each input by its own fill rule and flatten curves at
  `PATH_FLATTEN_TOLERANCE` (matching Scene), returning a `PaintPath`. `contains` /
  `stroke_contains` correspond to Canvas `isPointInPath` / `isPointInStroke`. There are also
  `bounds` and `transformed`. `cx.fill_path` / `cx.stroke_path` are anti-aliased.
- Color: fills, strokes, rounded rectangles, text, and icons all accept a solid color, a `Gradient`,
  or a `PaintPattern` (`Paint::Pattern`). `Gradient` supports `linear` / `radial` (single-circle or
  `radial_two_circle` focal gradients) / `elliptical_radial` (with any local affine `transform`) /
  `conic`, any number of color stops, and `Pad` / `Repeat` / `Reflect`. Gradients are evaluated per pixel,
  and interpolation color space can be selected via
  `.color_space(GradientColorSpace::PremulSrgb | LinearScRgb | Oklab)` (`Oklab` blends in perceptually
  uniform Oklab space; `LinearScRgb` and `PremulSrgb` interpolate premultiplied in their respective spaces;
  more than 16 color stops are resampled). `PaintPattern` tiles a decoded image with optional `src_rect`,
  `PatternRepeat` (`Repeat` / `RepeatX` / `RepeatY` / `NoRepeat`), `transform`, and
  `with_smoothing(bool)` / `with_sampling(..)`.
- Strokes: `StrokeStyle` sets width, line cap, join, and miter limit (Canvas `miterLimit`).
  Defaults match Canvas: butt cap, miter join, limit 10.
  `.dash(pattern, offset)` has Canvas `setLineDash` / `lineDashOffset` semantics.
  Each segment ends with the line cap. A stroke that would split into more than ten thousand dashes is drawn solid (each segment is already under a pixel).
  Undashed solid strokes with effective screen width ≤ `1.5 px` use the GPU stroke-capsule primitive directly instead of generating a tessellated stroke polygon mesh.
- Local transforms: `translate` / `scale` / `rotate` / `concat` / `set_transform`.
  `save` / `restore` save and restore the transform, opacity, and blend mode, as in Canvas.
  `restore` also pops clips pushed after `save` (layers are still managed separately by `push_layer` / `pop_layer`).
  Paths, clips, and
  gradients follow the transform. Strokes and dashes are computed in pre-transform coordinates and then transformed as a whole (as in Canvas).
  Shadow offset and blur do not follow the transform. Text, icons, images, and the built-in appearance from `draw_default()` also
  follow the current transform.
- Opacity, blending, and masks: `set_opacity` (Canvas `globalAlpha`, applied per command),
  `set_blend(mode)`, and `push_layer(opacity, blend)` / `pop_layer`, which draw a group of commands
  into a layer and then composite it as a whole, so overlapping parts do not accumulate opacity. The blend modes cover the full CSS
  `mix-blend-mode` set (`multiply`, `screen`, `overlay`, `darken`, `lighten`,
  `color-dodge`, `color-burn`, `hard-light`, `soft-light`, `difference`,
  `exclusion`, `hue`, `saturation`, `color`, `luminosity`) plus all 11 Porter-Duff compositing operators
  (`source-over`, `source-in`, `source-out`, `source-atop`, `destination-over`,
  `destination-in`, `destination-out`, `destination-atop`, `xor`, `copy`, `lighter`).
  Clearing Porter-Duff modes (`copy`, `source-in`, `source-out`, `destination-in`, `destination-atop`)
  clear outside their drawn bounds across the target, even when the group is empty or offscreen; top-level
  custom paint that uses Porter-Duff clearing/masking is automatically isolated inside the node's own layer so outer page backgrounds are never erased.
  `cx.push_mask(MaskKind::Alpha | MaskKind::Luminance)` / `cx.pop_mask()` turns the following custom-paint commands
  into an alpha or Rec.709 linear-luminance mask applied to the isolated destination group.
- `cx.shadow(path, ..)`: a shadow along the path outline. Outer shadow or `inset` inner shadow. (Rounded) rectangle and similarity-transformed rectangle shadows route through the analytic SDF quad pass instead of mesh extrusion.
- `cx.push_clip(path)` / `cx.pop_clip()`: clips only content recorded afterward. It does not affect child nodes,
  and it does not clip shadows drawn before `push_clip`. Any path is exact: path geometry is cut on the CPU and
  keeps anti-aliasing. Text, icons, images, and the built-in appearance from `draw_default()` use GPU clipping directly when the clip path is a (rounded)
  rectangle or a polygon with at most 8 vertices. Other shapes are drawn into a layer masked by the path.
- `cx.rounded_rect` (independent radius per corner; can carry a border and shadow), `cx.text`, `cx.icon`,
  `cx.image(rect, source, fit, radii)` (`source` has the same origin rules as CSS `url()`; `fit` is
  the same as `object-fit`; loaded asynchronously), `cx.draw_image(rect, PaintImage)` (adds `src_rect` atlas sub-rect crop,
  `nine_slice(insets, fill)`, `with_smoothing(bool)`, and `with_sampling`), and
  `cx.draw_texture_region` / `cx.draw_texture_regions` (shared-atlas sub-rect batching).
  `cx.fill_path_with_image(path, rect, source, fit)` fills an arbitrary path with an image. How subsequent images are sampled is set by
  `cx.set_image_sampling(..)` or Canvas-style `cx.set_image_smoothing(bool)` (`false` selects `ImageSampling::Nearest`)
  (saved and restored with `save` / `restore`). See [Image sampling](#image-sampling) below.
- Hit testing: points where `Painter::hit_test(local, size)` returns `Some(false)` pass through to what is below.
  When `hit_painted_outline()` is true, the hit region equals the recorded content (fills, strokes including dash gaps,
  rounded rectangles, images, text boxes, the node rectangle from `draw_default()`, computed with transforms and clips).
  `DestinationOut`, `SourceIn`, `DestinationIn`, `Xor`, `Copy`, and `push_mask` scopes update hit testing accordingly.
  Shadows do not participate in hit testing, consistent with CSS `box-shadow` and Canvas `isPointInPath`.
  Content drawn after `set_opacity(0)`, or inside a layer with zero opacity, does not participate either. Strokes
  are tested by distance to the center line of at most half the line width, so line caps and joins always count as round. Hit testing reads the node's
  latest recording. It takes effect immediately after state, focus, or theme changes trigger a re-record.
- Numeric limits: paths, line widths, and shadows containing NaN / infinity are not drawn. Paths more than one million pixels from the node origin after transform
  are not drawn. Line width, blur, and spread are capped at ten thousand pixels.

Besides setting it on `NodeStyle::painter`, you can use `AppContext::set_painter(id, …)`
(`MutationQueue::set_painter`) to attach a painter to any node: it takes precedence over the painter in the style,
and a component rewriting the node style will not wash it out. This suits reskinning built-in components.

The node's transform, clipping, opacity, stacking order, and compositor animations apply to the custom-painted content as usual.
A node with a painter forms its own stacking context. The node's own `overflow` clip applies only to child
nodes. It does not clip the painted shadow.

Consumers that cannot use a Rust trait use `PaintScript` (behind Cargo feature `paint-script`, included in `components`; `PaintPath::from_svg` remains always available): a JSON command list. It covers all of `cx`'s
drawing commands (except `measure_text`, which needs to read back a result, and custom hit logic; for hit testing you can give
a path). Lengths can be written relative to the node size as `"50%"` or `"100% - 12"`. Colors are semantic
role names, theme mixes, or typed paint; when calling Runtime
`PaintScript::from_json`, pass typed paint. Paths accept SVG path strings, segment arrays, boolean path objects
`{ "op": "union" | "intersect" | "difference" | "xor", "paths": [...] }`, and `{ "textPath": { "text": "...", "origin": [x, y], ... } }`.
Gradients accept two-circle arrays (`"radial": [x0, y0, r0, x1, y1, r1]`), elliptical objects (`"radial": { "center": ..., "rx": ..., "ry": ..., "rotation": ... }`), `transform`, and `colorSpace` (`"premul-srgb"` / `"linear-scrgb"` / `"oklab"`);
`paint` also accepts `{ "pattern": { "src": "...", "srcRect": ..., "repeat": "repeat" | "repeat-x" | "repeat-y" | "no-repeat", "transform": ..., "smoothing": false } }`.
`image` accepts `srcRect`, `nineSlice`, `fill`, and `smoothing`; `textureRegions` batches atlas sub-rectangles;
`imageSmoothing`, `blend` (including Porter-Duff keywords), and `pushMask` / `popMask` (`"alpha"` / `"luminance"`) have matching commands.
The `text` command also supports `family`, `letterSpacing`, `decoration`, `stroke`, and `shadows`. Commands can be filtered by interaction state.
A misspelled field name is an error (see the docs of `PaintScript::from_json` for the format). Icons are drawn by built-in
icon name.

In unit tests, `PaintRecording::record(&painter, &theme, size, state)` records a painter once without
`UiWorld`, so you can inspect the commands it recorded.

`Panel::kind(CardKind::Flat)` / `Card::kind(CardKind::Flat)` draw no background or border.
This suits handing the frame to a painter.

## Fetch host

Network permission belongs to the host. This section covers what an embedder implements and configures.

The default policy `FetchPolicy::default()` **allows no origin at all**. Without an allowlist, everything is denied.

### FetchHost

| Method | Contract |
| --- | --- |
| `fetch` | Required. Takes a `FetchRequest` (url / method / headers / complete body) and returns a `FetchResponse` or `FetchError` |
| `fetch_cancellable` | Also takes a `FetchCancellation`. The default implementation calls `check()` first and then `fetch`; a backend that can be interrupted should observe it in the connect, redirect-following, and body-reading loops |
| `fetch_streaming` | Writes the response into a `FetchSink`. The default implementation falls back to `fetch_cancellable` and sends the whole body as one chunk, so a host that only implements `fetch` still works; it just does not stream |
| `policy` | Required. Limits, timeout, redirect count, and worker count are all read from here |

`FetchSink` has two callbacks: `head(FetchHead)` is called exactly once, before any `chunk(&[u8])`. Returning `Err` from either aborts the transfer on the spot. When the consumer has gone away (or has its own limit), there is no need to keep paying for bytes nobody wants. `FetchHead` is all the response information except the body. `FetchHead::with_body` combines it with the fully read body back into a `FetchResponse`.

The response limit comes from **the `policy()` declared by the host that executes the request**. The default fallback of `fetch_streaming` checks against it too: when exceeded it reports `ResponseTooLarge` and not a single chunk enters the sink. Otherwise, a host that only implements `fetch` would leak the limit written in its own `FetchPolicy` entirely.

### FetchPolicy

| Field | Default | Notes |
| --- | --- | --- |
| Allowed origins | Empty set = deny all | Exact **origin** comparison, not a path prefix |
| `timeout` | 30 seconds | Total budget for one request, shared across redirects, not one per hop |
| `max_request_bytes` | 16 MiB | Request body, checked before sending |
| `max_response_bytes` | 16 MiB | Response body; when streaming, counted as **cumulative** bytes, so chunking does not let a larger body through |
| `max_redirects` | 5 | Exceeding it reports `Redirect` |

Origins are registered with `allow_origin` / `with_allowed_origin`. The format is `scheme://host[:port]`. They are stored as the ASCII serialization of the URL origin: a value with a path, query, or fragment reports `InvalidRequest`. Non-http/https reports `Policy`. Allowing `https://example.com` allows every path under it, but **not** `https://api.example.com`. A subdomain is a different origin and must be registered separately. `authorize(&Url)` is the enforcement point. `allowed_origins()` reads the set back.

### What the built-in NativeFetchHost does

A blocking `ureq` implementation. It must run off the UI thread. It enforces the policy before every hop:

- A request body over `max_request_bytes` → `RequestTooLarge`. Not sent.
- The response's `Content-Length` is checked first. If over the limit, it reports `ResponseTooLarge` directly and not a single body byte enters the sink. While reading, cumulative bytes are checked again, and the stream is cut immediately when exceeded.
- Every hop is re-`authorize`d. A redirect target not on the allowlist is denied as well. On cross-origin hops `authorization` / `proxy-authorization` are stripped. 303, and 301 / 302 for POST, become GET, clearing the body and `content-length` / `content-type`.
- `set-cookie` never appears in the returned response headers.
- The timeout is a total budget for the whole request: each hop deducts the elapsed time. When it runs out, it reports `Timeout`.

Errors are classified by `FetchErrorKind`: `Policy`, `InvalidRequest`, `Network`, `Timeout`, `RequestTooLarge`, `ResponseTooLarge`, `Redirect`, `Cancelled`, `Unsupported`.

### Installing it

| Entry point | Usage |
| --- | --- |
| `shared_fetch_host(host)` | Wraps your own `FetchHost` implementation as a `SharedFetchHost` (`Arc<dyn FetchHost>`) |
| `RuntimeProgram::resource_fetch_host(id)` / `ApplicationWindow.fetch_host` | The engine resource egress (`url()` images, etc.) for that window's document. Applications fill it per window; it defaults to `None` (remote images denied) |
| `SceneWgpuPainter::set_resource_fetch_host` | Embedded hosts that own their painter: before painting a document, hand that document's host to the painter |

```rust
let policy = FetchPolicy::default().with_allowed_origin("https://api.example.com")?;
let mut window = ApplicationWindow::new();
window.fetch_host = Some(shared_fetch_host(NativeFetchHost::new(policy)));
```

A custom `FetchHost` needs at least `fetch` and `policy`. To read responses as a stream, also implement `fetch_streaming`.

### What this path explicitly does not have

- **Cookies**: `set-cookie` in responses is dropped. There is no cookie jar.
- **CORS / preflight**: there is no browser same-origin model, and no `OPTIONS` preflight is sent. The only gate is the host allowlist.
- **Cache**: there is no HTTP cache layer.
- **Streaming request bodies**: the body is already complete bytes before it reaches the host. For response-side streaming, see `FetchSink` above.

The engine's resource egress goes through this path. `url()` images, `<img src>`, `mask-image`, and `border-image` share the same `FetchHost` and the same `FetchPolicy`: the same origin allowlist, the same per-hop redirect checks, and the same stripping of authorization headers across origins. **Without a host installed, no remote image is fetched at all.** `data:`, `file:`, and relative paths inside the jail are unaffected. The host is per document, not per process: when one process mounts several times or several windows share a painter, each document's images go only through its own host. Cached results are also separated by host, so a document never gets an image allowed by another policy. Cancellation happens when the painter is dropped and when an image has gone unreferenced for 120 consecutive frames. It ends the network wait, not a decode that has already started. `@font-face` has no network transport. It accepts only `local()`, `data:`, and local files inside the jail.

## Performance work you do not write by hand

Mounting a view (`mount_view*`) collects the whole subtree into one commit. After mutations commit, Runtime schedules dirty work itself. No changes means no frame. Large lists use `materialize_virtual_*`. Swapping a GPU texture bumps the generation without rebuilding layout.

### Image sampling

`url()` images (`<img src>`, `background-image`, `cx.image`) are prepared by default at **the device pixels they are actually painted at**:
in every fresh prepare the painter records how large each image is painted (the size after `ContentFit` / `background-size`
× node transform × scale factor, the same measure as HostTexture's `painted_extent`). When the same image is used in several places or
several windows, the largest is taken. A background thread downscales the original to that size with CatmullRom in linear light with premultiplied alpha,
uploads only that one level, and samples it bilinearly. Large images shown small no longer alias. GPU memory is counted at the display size. The size follows with hysteresis:
growing resamples immediately; shrinking waits until the smaller size has been stable for 2 seconds. Changes within `max(6px, 6%)` and requests where either side is under 16px
(collapsed, mid-animation) do not trigger it. Layout and `fit` are still computed from the image's original size. Resampling does not change the space it takes.

When the same image appears at very different sizes at once, or its size keeps changing (zoom animations, zoomable previews), use a mip chain instead:

```rust
BackgroundImage::url_with_fit(url, BackgroundImageFit::Cover)
    .with_sampling(ImageSampling::Mipmap);  // CSS layer / <img> content
cx.set_image_sampling(ImageSampling::Mipmap); // subsequent cx.image calls in a Painter
```

`ImageSampling::Mipmap` keeps the decoded size, generates a full mip chain in the background, and samples trilinearly
(`MipmapFilterMode::Linear`). `ImageSampling::Nearest` (also selected via `cx.set_image_smoothing(false)`)
keeps the natural decoded resolution and samples with nearest-neighbor filtering (`FilterMode::Nearest`) without bilinear or CatmullRom resampling, suited to crisp pixel art and tile atlases. Each sampling mode for the same URL uses an independent cache entry.

Local images (files, `data:`, `nana://res/`) are still decoded synchronously on the frame the first time they appear and are first shown at the decoded size.
The resampled image or mip chain replaces it once finished in the background. Remote images are fetched, decoded, and prepared directly at the target size on a background thread.
Resampling reuses the already-fetched bytes and does not hit the network again. Hosts that run their own event loop still use `set_image_waker` /
`has_pending_images()` to wait for these background results.

An `<img>` with no width or height (`content_image`) lays out at the image's decoded size. When only one side is given, the other is derived from the aspect ratio.
Until the size arrives the box has no area, and the image still loads. After each present, the built-in host hands the new sizes to this window's document,
and the next frame relays out. Hosts that run their own event loop do the same with `SceneWgpuPainter::take_image_natural_sizes` and
`commit_image_natural_sizes` (see [Layout · Replaced content](layout.md)).

HostTexture pixels belong to the host: by default only level 0 is sampled. The host prepares the size from `painted_extent` (see
[Live GPU content](gpu.md#preparing-content-at-the-actual-painted-pixel-size)). When the host uploads its own mip chain, use
`.sampling(ImageSampling::Mipmap)` on `GpuTextureView` / `Thumbnail` / `Avatar` (`CustomRenderNode::with_sampling`) to switch to trilinear sampling.

Messages have two entry points; choose by type: `dispatch_program` **keeps only the last message per Rust type**. It suits state messages where "the later one supersedes the earlier one" (resize, theme changed, redraw requested). `dispatch_program_all` delivers all messages in dispatch order. Business messages are usually one `enum`, which is **the same type**. Using `dispatch_program` collapses two clicks in one frame into one and silently drops the first. In that case use `dispatch_program_all`. Both enter `update` on the next frame.

When a control needs to handle keys before default editing, register a policy with `AppContext::on_key` or `on_view_key`. The latter reads the current retained control value. Returning `true` means consumed. Registering again replaces the old policy. Removing the view removes the policy. Input routing calls `dispatch_focused_key` after overlay handling and before the terminal and default editing. It delivers only keys (committed text does not go through policies), and only to a focused node that is mounted in the current document and not disabled. Business policies are skipped during IME composition. A key consumed by a policy no longer inserts the text it would have typed.

When the application rewrites editor text (`update_component`, `set_component`, `mount`, or committing `SetTextInput` directly), as long as the bytes change, that editor's undo/redo is cleared with the commit (when writing `UiWorld` directly, it is cleared on next use): after loading another document, Ctrl+Z does not go back to the previous one. Writing back the value the editor itself reported, or rebuilding identical text, is not a rewrite; the history is kept. Clearing after send and formatting write-backs done from the application side (`update_component`, etc.) likewise erase the user's undo history. To make it a user-undoable step (formatting shortcuts, completion), use `edit_text_area(entity, range, text)` / `edit_text_input(…)` instead: it replaces `range` with `text` through the same path as the user's own edits and records it as a separate step, not merged with continuous typing before or after. Like user input, it is rejected when the editor is read-only, disabled, or in IME composition. The `TextInput` length limit still applies. Atoms touched by the range are replaced whole. Every caret, including the user's own, shifts with the edit and is not moved to the edit location. The only exception is a caret sitting exactly at the insertion point, which moves to after the inserted text, consistent with typing (completion at the caret). It is not typing inside a snippet placeholder; linked placeholders do not follow. An active snippet session is remapped or ended like any value change. Exceeding the `TextInput` length limit rejects the whole edit rather than truncating. Replacing with identical text is not an edit and sends no change event. A range that is out of bounds or not on grapheme boundaries returns an error. When the editor's own `TextChanged` handler (registered with `on`) rewrites the editor during event delivery (uppercasing, filtering characters), that counts as part of the editor receiving input: the step records the text after the handler's rewrite and is undoable as usual. A step that changes no text is not kept. For clearing after send, whether the sent draft can be undone back is up to the application: if it should not be, call `clear_text_history(node)` at send time. Only `TextInput` and `TextArea` keep an undo log: number fields, search dropdowns, the command palette, and context menu input fields carry state beyond the text (the committed number, filter results), and restoring only the text would put the two out of sync, so they keep no log. `can_undo_text` is always false for them.

When a retained editor is rebound to another task, file, or draft identity, call `clear_text_history(node)` even if the text is the same, to clear the original object's undo/redo. It does not change the text, selection, or an in-progress IME. Business object identity and whether rebinding is allowed are still the application's decision.

Components that handle the pointer themselves can use `UiWorld::pointer_layout_position` to convert window coordinates to layout coordinates, and `layout_pointer_position` for the reverse. Both use the current hit projection, including ancestor scrolling and perspective transforms. With no projection, a parked node, or a non-invertible transform they return `None`. The existing frame flow should refresh the layout and hit projection before input.

## Rust virtual lists and trees

When you need real scroll space and retention of offscreen edits, put the List inside a ScrollView and use
`materialize_virtual_list_retained_in` (`materialize_virtual_table_retained_in` for tables, and
`materialize_virtual_tree_retained_in` with a `VirtualTreeLayout` that contains only expanded rows for trees). The old
unpositioned `materialize_virtual_{list,table,tree}[_in]` compatibility entry points have been removed. List example
(`list` is already mounted; `items` is kept across frames):

```rust
cx.materialize_virtual_list_retained_in(
    list, &mut items, &layout,
    VirtualViewport::vertical(offset, height, overscan),
    &[], // extra business edit sessions; focus and Runtime IME are retained automatically
    |index| model.key_at(index),
    |key| model.index_of_key(key),
    |index, _key| TextInput::new(model.value_at(index)),
)?;
```

Scroll-driven windows use `sync_virtual_list_retained_in(scroll, list, items, layout, overscan, fingerprint, …)`
(trees/tables use `sync_virtual_tree_retained_in` / `sync_virtual_table_retained_in`). It reads the current `ScrollOffset` and viewport from the
ScrollView and determines the range with `window_for` (including overscan).
When the range, data fingerprint, and active focus/IME state are all unchanged, it commits no Runtime mutation and still translates.
The application computes the fingerprint from the key sequence, data version, and retained keys. Reordering keys with the same length and extent must change it.
New rows are mounted and old rows unmounted only when the range crosses a row. When focus or IME becomes invalid, automatically retained items are released immediately.
Call it in `prepare` / frame hooks. Do not tie it to `ScrollChanged` (it would miss inertia, layout clamping, and post-flush offsets).
When the data key sequence, version, or `retained_keys` change, update the fingerprint and keep calling sync. When row height or layout changes,
update `layout` first and then materialize. Covers, events, and paging stay in `on_mount` / the row diff. The framework does not fetch images.

When each row needs events bound, use `materialize_virtual_list_retained_with` (or `sync_virtual_list_retained_with`).
It takes an extra `on_mount` callback: called once **only** for rows newly created in this call (scrolling back to an already-mounted row does not call it again).
It runs after the commit, so you can call `cx.on(entity, ...)` directly. Rows released by scrolling away are released together with their handlers.

When row height is determined by content (wrapping, `line_clamp`, expansion), do not estimate character widths in the application: build the items with
`VirtualListItems::measured()` and call `sync_virtual_list_measured_with(scroll, list, items,
&mut layout, …)` instead. Row containers are no longer pinned to the height in `layout`; they lay out by content. Each sync first reads back
the real heights of mounted rows from the previous layout and writes them into `layout` (`measure_anchored`). The row at the top of the viewport stays put. When rows above it measure
taller, the scroll offset grows by the difference instead of pushing the content down. Values in `layout` are only estimates for rows not yet measured.
Newly mounted rows can only be measured after the next layout: when `items.pending_measure()` is true, request another frame.
The list does not have to be the entire scroll content: both syncs subtract the content above the list (header, padding) using the list's position relative to the ScrollView in the previous layout. Only the part of the viewport that overlaps the list counts as the window.
Entry points like "expand" appear only when the text is actually truncated: after layout, `cx.text_truncated(text)` reports whether `line_clamp`
(or the height) dropped lines. Do not guess from a character-count threshold.

Item identity and content are kept by key. The framework places a non-hit-testable container outside the component to maintain logical positions,
and manages the List's full content height. Data reordering must provide the latest inverse index. Deletion, collapse, or a key
mismatch releases the corresponding item. For offscreen navigation, first query the target offset and materialize. After layout is published, call `scroll_into_view(scroll, target, margin)`
(a target already in the viewport does not move the container; it scrolls only by the minimum distance; virtualized rows still need to be materialized before they have a layout box).
Then move focus to the specific control inside the target item. Business-persisted drafts and selection are still saved by key.
The old materialize entry points only reconcile identity. Do not mix them with the positioned entry points on the same non-empty `items`.

## Rust frozen tables

`materialize_virtual_table_retained_in(table, items, layout, viewport, frozen, retained_cells,
row_key_at, row_index_of, column_key_at, column_index_of, build_row, build_cell)` uses the same
`VirtualTableLayout` and key data model. `frozen` is ordered as `[column count, row count]`. `retained_cells`
are additional `(row key, column key)` pairs to retain. Cells holding focus or the Runtime IME are retained automatically.
The framework manages the table content size, absolute row and column positions, frozen transforms, and frozen region order. When no cell background is specified,
Surface is used. The application maintains the latest inverse index for both axes. When data changes, call `materialize_virtual_table_retained_in`.
For scrolling use `sync_virtual_table_retained_in(..., overscan, fingerprint, frozen, ...)`. When the body and frozen ranges,
fingerprint, and active focus/IME state are all unchanged, no cells are mounted/unmounted. The frozen prefix
still updates its transform from the current offset. Do not override the managed transforms.
For offscreen navigation, first `reveal_cell_with_frozen`, then materialize, publish layout, and scroll, then focus the control inside the target.
Unloading a column releases all nested views and subscriptions of its cells. Persistent selection and drafts are still saved by the application by business key.

## Non-goals

- A full browser, Tauri, or a whole-window WebView as product UI
- A second Device / Queue, or CPU readback disguised as zero-copy
- Do not give controls the window handle. Do not lay a native WebView over the Surface after the UI is drawn
- Defining new framework contracts through the crate-root control table

Opening web pages inside the application uses the host-native content exception of `runtime::BrowserView`. Currently only
macOS `WKWebView` is supported. Windows/Linux are explicitly unavailable. `nana-ui` has no separate
`browser` feature.

When you need a web page as visual material (live-stream widgets, overlays), use a headless `WebSurface`:
`RuntimeProgram::web_surface_requests` declares the pages, and frames go to the application's own sink. It is not in the
UI tree, and it is not a WebView product shell. For the contract, see the [GPU reference](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/gpu.md#无头网页画面websurface).


### Multi-document layout cache

`RetainedLayoutCache` isolates layout boxes, measurement results, isolated container positions, and resolved
padding by `DocumentId`. When the same `UiWorld` lays out several documents in turn, a full layout of one document does not clear the others.
Hosts that call `RuntimeLayoutEngine` independently call
`RetainedLayoutCache::remove_document(document)` after a document closes to release that document's cache. Laying out an empty document also releases the cache.
`AppContext` releases the layout cache automatically when it removes, parks, or detaches the last document root.
Parking still keeps control state. The layout cache is rebuilt after reinsertion.


Retained measurement results are invalidated per node: when content or style affects layout, all historical
constraints of that node and its real dependent ancestors are invalidated, so switching back to an old viewport does not restore stale sizes. Each node retains at most two sets of constraints across frames. Exceeding the capacity only causes
remeasurement. The measurement cache within a single layout is not subject to this limit. After deleting a node, a low-level host can call
`RetainedLayoutCache::remove_node(document, id)` to release the entry immediately. AppContext's deletion
transaction does this automatically. Other nodes' caches are unaffected.


### Scroll metrics after partial layout

A partial layout publishes scroll metrics only for the nodes actually relaid out and their ancestor ScrollViews. Duplicate targets are merged, and
document order is kept. Other documents, parked nodes, and unrelated subtrees do not take part in target selection. A full layout still performs a complete publish.
`restore_scroll_anchor` adds the container to the next layout's work, ensuring the anchor can be restored after measurement even with no style change.
The content extent of affected scroll containers is computed by UiWorld's lazy layout bounds index. The first query builds the subtree index.
Ordinary geometry write-back updates only the maximum bounds of the changed node and its ancestors. Structural changes rebuild the child aggregate of the corresponding parent.
The index does not include painted shadows, filters, or scroll offsets. Deleting a node releases its entry at the same time. The extent counts only up to the nearest clip box: an inner scroll container
contributes only its own box, and an `overflow: hidden` box contributes only its own box on the axes it clips (see
`UiWorld::overflow_contained_axes`). When a box starts or stops clipping, the container above remeasures accordingly.

### Workspace borrowing an existing control as a region

`WorkspaceRegionSlot::new` denotes a structural region managed by Workspace. It uses Generic semantics and does not receive pointer input itself.
To use an existing control directly as the region surface, use `WorkspaceRegionSlot::borrowed`: Workspace only projects region layout and surface style,
and the control keeps its current input and accessibility semantics. Reassembly does not restore old label, disabled, or focusable snapshots.
`DesktopShell` selects this contract automatically for a directly borrowed ScrollView. Ordinary structural regions keep the original behavior.
After creating a Workspace, call `AppContext::assemble_workspace` to establish the region tracks before building layout and the hit index.

### Accessibility node coordinates

`AccessibilityNode::bounds` is relative to the parent node's space: `(x, y)` is the origin of the node's own space within the parent's space (within the viewport for the root node, or when the parent has no bounds), and `(width, height)` is the node's box in its own space, starting at the origin.
`AccessibilityNode::transform` is the scale, rotation, skew, or flip from the node's space to the parent's space (`PaintTransform`, CSS `matrix(a, b, c, d, e, f)`), applied before the translation `(x, y)`: node to parent is `translate(x, y) · transform` (`AccessibilityNode::to_parent`).
Two kinds of nodes set it; for those, the size in `bounds` is the node's own layout box, and the children's bounds are unscaled local offsets inside it:
boxes whose own `transform` style is more than a translation (a positive scale emits `diag(a, d)`, with the translation folded into the `bounds` offset; rotation, skew, and flip emit their own 2×3);
and boxes that come from the viewport (page root, `fixed`, `sticky`, escaped absolute positioning) that have been rotated or scaled on the 2D chain, or that sit in such a space: the translation part still comes from the viewport, and the linear part is handed to descendants.
On a chain with only translation and scrolling it is always `None`, and a `transform` style that is a pure translation (factor 1) is also `None`. Nodes in a 3D context emit no perspective; `bounds` is the bounding box of the projected corners.
Virtual children (menu rows, tree rows, list box options) are likewise relative to their own parents, and inside a rotated control they are upright boxes in the control's space. `clip` is still in viewport coordinates: it is the screen region left by the clip chain.
AccessKit hosts set `to_parent()` as the node's `transform` (in physical pixels the translation part is multiplied by the scale factor) and the size as the local `bounds`, so when a row grows taller, nodes below that merely translate with their parent do not enter the delta or the native update; the same holds under rotated or scaled ancestors: when an ancestor changes its rotation angle or scale factor, only it and the viewport-derived boxes below it enter the delta.
When you hold the relative tree and need a node's position in the viewport (hit testing, automation, tests), use `AccessibilityPlacements::new(&nodes)`: `transform(id)` is the accumulated transform from the node's space to the viewport, `corners(id)` is the box's four corners, `viewport_bounds(id)` is the bounding box, and `contains(id, x, y)` tests by transform whether a point is in the box.
To take screen coordinates directly from the world (automation, tests, debug export), use `UiWorld::project_accessibility_in_viewport` or `project_accessibility_nodes_in_viewport`; they give viewport bounding boxes, and `transform` is always `None`.
The native tree can only be built by `project_accessibility` and `project_accessibility_delta`. On mobile, `MobileRuntime::accessibility_nodes` gives the relative tree (TalkBack via AccessKit), and `accessibility_nodes_in_viewport` gives viewport bounding boxes (used by the flat VoiceOver snapshot on iOS).

### Accessibility semantics in hidden hierarchies

`UiWorld::project_accessibility` and the incremental projection keep the structural ancestors needed to connect visible controls.
A `visibility:hidden` container emits a neutral Generic node. Its own label, value, description, original role, and
interaction state are not exposed. A child with explicit `visibility:visible` keeps its full parent chain and semantics.
Hidden leaf nodes and subtrees that produce no layout box are not projected. Once visible again, semantics from the current business state are restored.

Embedded hosts with `accesskit-tree` enabled can use `AccessTreeProjector` to consume full nodes or an
`AccessibilityDelta`. A hidden structural container maps to a GenericContainer with no click/focus actions.
Children keep their actions and focus. The Workspace/Dock persistence format is unaffected by this projection change.

The native hosted adapter additionally holds a stable Window accessibility root. Mounting, replacing, or clearing the document,
and focus entering an ordinary control, never replace the window provider's root identity. Runtime nodes still keep their own
role and stable ID. When no control is focused, native focus falls back to the window root rather than borrowing a hidden structural container.
This window wrapper does not change the embedded projection contract of `AccessTreeProjector`.

Native adapters share the current retained projection. On platform activation or reactivation they build a full snapshot of the published state on demand. Updates and activation
share the same committed state. The projection lock is released before entering native event callbacks. `AccessTreeProjector::new`
likewise only establishes retained state. An embedded host that needs the first full tree calls `full_update()` explicitly.
Pure label, value, focus, and bounds updates reuse the root and the text run index. Focus is maintained incrementally by stable ID.
Changes to structure or text input roles still go through the necessary rechecks.
The accessible content of static `Text` maps to AccessKit `Label.value`, so the native UIA Name property
contains the text. Ordinary controls still use label and value separately. An edit control's name and input value are never mixed.

The hosted host stages accessibility changes after layout converges and publishes them after a successful present, before the application's `presented` / window
binding callbacks. A batch of changes made while nothing is presented keeps its delta. When several batches accumulate, it only marks that, after recovery,
a full snapshot of the current document is needed, so intermediate deletions or remounts do not distort the stitched batches. Idle retries do not overwrite
existing changes. Each window keeps its pending state independently. Closing a window does not modify the main window's queue instead.

After an encoding failure, a self-driven surface's `HostedGpuContext` consumer (`wgpu-interop`) first drops this frame's
`FrameContext` and the views that reference the Surface frame, then calls `discard_frame` (main window) or
`discard_surface_frame` (auxiliary window). The next acquire rebuilds only the affected Surface and keeps using the existing
`GpuContext`. The failed frame is not submitted, and producers are not told the submit succeeded. Targets the painter drew are rolled back automatically
when the frame is discarded. The standard Runtime host handles this path automatically.

The accessibility adapters of newly created main and auxiliary windows initially provide only the stable Window root and do not read the document in advance.
Content is published only after the first successful present. Even if the application has already called `flush` before mounting, unpresented controls do not
enter the native tree early. A cold start has no base tree, so the first batch of partial deltas needs one current document snapshot added. A valid explicit
full snapshot can be used directly. Partial updates after the first publish continue under the delta contract.

`RuntimeProgramContext<Message>` can be cloned directly and moved into background tasks. It only requires `Message: Send`,
not that the message implement `Clone`. Clones share the host GPU resources, message entry point, and task queue. Background tasks use
`dispatch` to wake the host and submit messages. Business state is still handled by the application's update callback.

## Window service

`RuntimeProgramContext::windows()` provides a thread-safe `WindowService`. `window()` provides the current window's `WindowHandle`. Window descriptions are unified as `WindowDescriptor`. A window that shows a single picture uses `content_aspect_ratio` to lock the client area's aspect ratio; the framework handles edge dragging, minimum size, and ratio changes; see [Content aspect ratio](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/window.md#内容宽高比). `ApplicationState::build` runs before each window is published. If building fails, the window is not shown. Window events can be observed in `ApplicationState::window_event`. See [Window](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/window.md#多窗口) for completion semantics, the embedded interface, and migration notes.

### Claiming windows by kind (`WindowRegistry`)

Programs that implement `RuntimeProgram` directly and hold the documents of several windows themselves use `WindowRegistry<K>` to remember which kind of window each `WindowId` is. `K` is the application's own `Copy` type, usually an enum (main window, tool card, onboarding card, ...):

- `WindowRegistry::new(primary)`: the main window is created by the host and bound from the start.
- Before creating a window, `request(key)` gets a tag, which goes into `WindowDescriptor::tag`; the registry generates the tag, and the application neither writes nor parses the tag string. A new request for the same key replaces an older one not yet claimed; `withdraw(key)` withdraws it.
- In `initialize_window`, `claim(id, context)` binds the host-assigned id to the key; for a window not requested by this registry it returns `None`, in which case return an error so the host rolls back.
- Other hooks take the kind with `key(id)` and then `match`; `window(key)` looks up the reverse, and when several windows of the same kind coexist (the old one still closing, the new one already open) it returns the newest.
- In `window_event`, `route_event(id, &event)` takes the kind, and `Closed` also unbinds; in `discard_window`, call `release(id)`. A window stays bound until it closes, so after the application lets go, its last events still reach code that knows it.
- Hosts without a window service (offscreen, tests) bind directly with `bind(id, key)`.

When a window closes, `RuntimeApplication` first removes the window document, then calls `ApplicationState::window_closed` to clean up application state, and finally calls `window_event(Closed)`. Observers of the close notification therefore see the cleaned-up state. Rollback after a creation failure still calls the cleanup hook, but sends no success or close events.

`TextArea::auto_grow(min_rows, max_rows)` provides a multi-line input that grows and shrinks with soft wrapping; `character_counter(limit)` shows the committed character count, and `submit_on_enter(true)` selects Enter to submit / Shift+Enter for a newline. Default behavior is unchanged. L3's `.trailing_action(view)` reserves a real action slot of small-control size in the bottom-right corner of the box, and `.model(signal)` and `.on_submit(...)` can be used together; for imperative assembly see `AppContext::assemble_text_area`. When the minimum row count of `auto_grow` is 1, the counter and action first share a compact single line with short text; only once the actually displayed text (including IME) exceeds that line's available width or has an explicit line break does it expand into a full-width editing area with a bottom action bar. Row count counts only text lines. IME confirmation does not send `TextSubmitted`.
