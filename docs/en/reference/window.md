# Window

The custom-drawn window buttons you see on Windows use the NanaUI icon button look: 28×28, control corner radius, vertically centered. Buttons are 2px apart. The button group has 6px on each side. Ordinary buttons use the regular hover/pressed colors. The close button uses the danger color. Each button's actual area handles the window action. The surrounding space does not expand into a system-style block button. The native red/yellow/green buttons on macOS keep platform behavior.

NanaUI draws desktop windows for you. Title bars, icons, system materials, and multiple windows follow desktop software conventions, not browser conventions.

## WindowShadow (Issue #215)

`WindowDescriptor::shadow` is platform-neutral desktop shadow semantics. The default is `WindowShadow::Auto`. It only describes top-level window decoration. It does not add padding to the client area. It does not change the coordinates of `WindowId`, input, IME, accessibility, or capture. `None` turns it off explicitly. `Custom` can select `WindowShape` or the explicitly expensive `ContentAlpha`.

WindowShadow and the UiScene `DropShadow` are two separate authorities: the former is decided by the Window/Platform layer. The latter is a visual effect inside the framebuffer. Input, drag, and resize regions are never used as a source of the shadow shape.

Who draws the shadow is decided in one place (the intent from `nana_ui::window_shadow` → applied by `nana_window::shadow::WindowShadowState` → result written back to `ResolvedWindowPresentation::shadow()`):

| Request | Opaque / material window | Transparent window |
| --- | --- | --- |
| `Auto` | The platform's own window shadow (`Native`) | companion, following the window's visible body |
| `Custom(WindowShape)` | companion (platform shadows do not accept custom styles) | companion |
| `Custom(ContentAlpha)` | `ContentAlphaUnavailable` | Same as left |
| `None` | Off | Off |

- **Visible body**: the first large (≥ 1/4 of the viewport) filled background quad the window draws. For a transparent window, the root card and its rounded corners. If none is found, the whole client area. It is recomputed only after the scene's projection revision, the window size, or visibility changes. If the body, size, and visibility are unchanged after recomputing, only the revision is updated and the companion is not repositioned. The Windows platform also skips unchanged positioning requests; main-window moves are still synchronized by the subclass. Input, drag, and resize regions never participate.
- A **companion** is a platform-private auxiliary window that cannot be hit, does not activate, and does not appear in the taskbar. It is always directly below the main window. The main window's frame, client area, `WindowId`, and native handle do not change. A companion is not a NanaUI window. It receives no events. A move only moves it (on macOS the window server moves child windows along. Zero work). A resize only updates geometry. Only a style or scale-factor change re-rasterizes (`WindowShadowWork` counts each separately).
  - macOS: a borderless child window `orderedBelow` the main window. One `CALayer` whose `shadowPath` is the body's rounded rectangle. An even-odd `CAShapeLayer` masks out the inside of the body. A translucent card does not show its own shadow through it. The shadow is drawn by the render server.
  - Windows: `WS_POPUP` + `WS_EX_TOOLWINDOW | NOACTIVATE | NOREDIRECTIONBITMAP | LAYERED | TRANSPARENT`. `WM_NCHITTEST` returns `HTTRANSPARENT`. DirectComposition nine-slice (the CPU rasterizes only one rounded shadow tile. Edges are stretched by scale transforms). The main-window subclass places it directly below the main window in `WM_WINDOWPOSCHANGED`. Hidden on minimize / maximize / hide. Destroyed with the main window.
  - Windows windows with a DWM frame use the DWM shadow (`Native` is reported only when `DWMWA_NCRENDERING_ENABLED` is observed). Requesting `None` for such a window reports `Native` + `DisableUnsupported`. The frame policy is not changed behind your back. A borderless `Auto` window has no DWM shadow. The default companion takes its place.
  - Linux and others: `Unsupported` / `CompositorManaged`. This tells the application explicitly that there is no observable shadow here.
- `WindowCommand::SetShadow { id, shadow }` changes the shadow after the window opens. The application reads `presentation().shadow()` for the actual result. Before the window is shown it is `Pending`.
- Transparent windows get a shadow by default: an application whose card draws its own UiScene `DropShadow` should set `WindowShadow::None`. Otherwise there are two shadows. Click-through overlays should also set `None`.

**Verification status**: on macOS, the companion's geometry and ordering were confirmed on a real window (body + 21pt margin, `orderedBelow`, no updates on static frames). Visible rendering still awaits visual confirmation with the display awake. On Windows only cross-compilation and clippy for `x86_64-pc-windows-gnu` were done. The DComp nine-slice, the combination of `LAYERED` + `NOREDIRECTIONBITMAP`, and cross-process click-through are **not verified on real Windows hardware**.

`run_runtime(WindowDescriptor::new("Title"))` creates the main window and the single GPU context, and starts the event loop. `WindowDescriptor` is `nana_ui_platform::WindowDescriptor`.

## Title bar

The default is a custom-drawn title bar: content on the left, title in the middle, window buttons on the right. Dragging empty space moves the window. Buttons receive the pointer first. They are not dragged away.

A title bar overlaid on the stage still uses `AppTitleBar`. Text and actions go in the `leading / center / trailing / controls` layout slots, and `assemble_app_title_bar` is called. `transparent(true)` only removes the bar background. Content and hit testing remain. Whole-window transparency is not required. Over video, use `over_media(true)`: a media scrim gradient sits behind the bar, and the title and window buttons use the theme's light media foreground, which does not darken under a light theme. `drag_enabled(false)` stops empty space and text in the bar from starting a window drag, and cancels a gesture that was pressed but not completed on later input. Hiding or unmounting the title bar also cancels it. The defaults are `false` and `true` respectively. When keeping business entry points in fullscreen, set `drag_enabled(false)` and `show_window_controls(false)`. Do not add separate top-edge coordinate drag logic. The semantic entry supports `transparent` and `drag-enabled`. When the title is empty and there is no center content, no middle placeholder is kept. The right side reserves space by content width. The left side uses the remaining width and can shrink. With a title or center content, the left and right stay symmetric. When things do not fit, they give way in this order: the middle column moves first (when the right content is wider than an equal share, the middle column yields to the left. Its width does not change). Then it narrows (content is elided. It can shrink to zero). The right column (the application's buttons and the window buttons) always keeps its content width, so the window buttons are never clipped. The left column is squeezed before the middle column.

The system window buttons are the title bar's `controls` component, `AppTitleBarControls`. Layout is decided by the component node. Both platforms look the same from outside:

- macOS (`native_controls`): transparent title bar + full-size content. NanaUI draws a 36px title bar. `controls` is an empty 78px-wide placeholder at the start of the leading column. Red/yellow/green are still the native system buttons (hover icons, the green-button menu, graying out when unfocused, and accessibility are unchanged). After layout each frame, the host centers the native button cluster on the placeholder's layout box, and replays this immediately after show/hide commands, native style rewrites, and fullscreen mode changes. So when the title bar shifts or insets (for example to avoid the window's rounded corners), red/yellow/green move with it. The move applies to the whole system title-bar view that holds the buttons: setting the buttons' own frame has no effect because AppKit constraints override it, and moving only the button group makes it leave its parent view's bounds. It still draws but receives no clicks. No moves happen while fading or hidden (`alphaValue < 1` or `isHidden`). AppKit is animating the buttons then. Reading their position would skew the whole cluster. When shown again they are re-placed at full opacity. The placeholder receives the pointer. No window drag starts on it. The placeholder is not limited to the title bar's controls slot: `AppTitleBarControls` is an ordinary component. Mounted anywhere in a window's document, it decides where that window's native buttons go (one set of buttons per window; the first in document order wins). The position only has to be inside the window. The buttons' own size and spacing do not change. When the window is created, `prepare_client_chrome` first centers them vertically by the title bar height. Once layout exists, the placeholder takes over.
- Windows / Linux: system decorations are turned off. `AppTitleBarControls` draws minimize, maximize, and close, at the end of the trailing column, flush with the title bar's right edge.

Applications that reveal chrome on pointer movement use `WindowCommand::SetNativeWindowControlsVisible { id, visible, duration }` to fade the native window buttons with the chrome: macOS animates the red/yellow/green opacity over `duration`. When hiding, the buttons leave painting and hit testing after the animation ends. A show received midway takes over from the current value. The host remembers the hidden state and hides them again after native styles are rewritten, for example on a fullscreen change. On Windows / Linux the buttons are an `AppTitleBar` control. The command is a no-op. The application shows and hides them with the title bar. Fading out only changes opacity. The placeholder layout stays put. Only `show_window_controls(false)` hides the placeholder and gives up the left-side space.

Custom chrome can resize from the outermost 8px of the window's client area (four edges and four corners). System caption, maximized, fullscreen, and `resizable: false` are left to the platform frame or disabled. No second set of hit areas is stacked on top.

A window without a custom title bar sets `WindowDescriptor::system_caption(true)`. This keeps a borderless Windows window from losing its close button.

Native window move/resize on Windows keeps the system title bar, Aero Snap, and system frame behavior. `LiveSizeMove`
installs a window timer through `install_with_wake` during the operation to wake the host event source; the Scene host
runs normal frame-demand scheduling in the proxy wake, so other windows and hidden GPU outputs keep updating. The timer
stops when the operation ends, the hook is removed, or the window is destroyed, so idle windows get no extra redraw demand
from it. The callback may only signal a wake. It must not call into the host inline; embedded hosts should forward the
proxy wake to `EmbeddedRuntime::wake`. A custom title bar starts the native drag through a queued post, so the current
input callback returns first and the application callback is not occupied for the whole drag. `host_managed_drag(true)`
remains an optional alternative behavior. It is not a required switch for keeping rendering alive.

Close / minimize / maximize are window actions. Controls emit semantics (`WindowChromeAction`). The Scene host executes them. Close, like the system close, only sends `CloseRequested` to the program. The program decides whether to close. Ordinary controls never get the window handle. CSS `-webkit-app-region` / `app-region` is not a drag contract: writing `drag` on any box does not make it a caption.

`WindowChromeState` is bound to an explicit `WindowId`. A single window can use the default entry (which only accepts the first window it receives). With multiple windows, wait for `WindowService::create_window` to return a handle, then call `for_window` with its `id()` to build one per window. After a window closes, the state does not automatically take over another window.

### Windows client-drawn title bar contract

Chrome on Windows is decided by two groups of inputs. Read them together: **the descriptor decides how the HWND is created. The live material decides what DWM draws**.

`WindowDescriptor::transparent` is a lifetime contract, not current state: `window_surface_effect` unconditionally returns `Transparent` for a window where it is true. The material can never switch back to solid. A host that wants to offer users a "solid background" switch must keep it `false` and report the live material through `RuntimeProgram::window_material_mode_for`. Chrome follows the effective material (`ResolvedWindowPresentation::chrome`).

#### At creation (descriptor only; implemented in `windows_scene_chrome`)

| Setting | winit `decorations` | `WS_EX_NOREDIRECTIONBITMAP` | Initial corners |
| --- | --- | --- | --- |
| `system_caption: true` | On (system title bar and resizing) | On only if this window uses composition | System default |
| `system_caption: false` and `transparent: false` | Off; `AppTitleBar` draws Minimize / Maximize / Close | On only if this window uses composition | Rounded |
| `system_caption: false` and `transparent: true` | Off | On only if this window uses composition | Not rounded |

`undecorated_shadow` is always `false`: that winit path insets the client area's top edge by 1px, which the title bar cannot cover.

### Presentation path: ordinary window vs DirectComposition

Where a window's alpha comes from depends on what the surface negotiated. That is decided by the **presentation path**, not by the material. Three combinations measured:

| Path | Backend | Negotiated alpha mode | Source of transparency |
| --- | --- | --- | --- |
| HWND swapchain | Vulkan | `PreMultiplied` | The swapchain itself |
| HWND swapchain | DX12 | **`Opaque`** | Only the glass from `DwmExtendFrameIntoClientArea` |
| DirectComposition visual | DX12 | `PreMultiplied` | The swapchain itself |

The DX12 HWND swapchain is hard-coded to report only `Opaque` (`wgpu-hal` `dx12/adapter.rs`). So **window transparency on the ordinary path actually depends implicitly on Vulkan**.

A surface that is not `Opaque` stores pixels in the last step the way the compositor reads them. DWM and Core Animation both treat 8-bit surfaces as premultiplied `enc(color) × α` and blend in encoded space. Metal's `PostMultiplied` is the same. Blending is still done in linear light internally. Only the blit that copies the whole frame onto the surface stores `enc(color) × α` instead (`AlphaEncoding::Gamma`). Otherwise translucent pixels are shown too bright. Feathering, shadows, and antialiased edges over the desktop show hard edges and banding. Opaque pixels are byte-identical under both encodings. Offscreen snapshots default to `Linear`.

#### Two dimensions: process backend vs per-window target

These are two questions. Do not merge them into one:

| Dimension | Owner | Decided by |
| --- | --- | --- |
| GPU backend / adapter / device | **The process**; one shared by all windows | `RuntimeProgram::gpu_backend_policy()` |
| Surface target (ordinary window surface vs composition visual) | **Each window** | `WindowDescriptor::surface` |
| Color space and dynamic range (SDR / Display-P3 / HDR scRGB / PQ / HLG) | **Each window** | `WindowDescriptor::presentation_policy` (`SurfacePresentationPolicy`) |

Transparent client areas have an explicit alpha-depth boundary with 10-bit HDR swapchain formats: `Rgb10a2Unorm` (the standard 10-bit format for `HdrPq` / `HdrHlg`) carries only 2 alpha bits and cannot represent smooth translucent edges, so transparent windows reject it with `SurfaceProfileFallback::TransparencyIncompatible` and fall back to full-alpha `Rgba16Float` extended-linear HDR or SDR (while `Rgba16Float` + `Bt2100Pq` / `Bt2100Hlg` remains valid when advertised). See [Live GPU Content and GPU Integration Contract · HDR display info and color space contract](gpu.md#3-hdr-display-info-and-color-space-contract-issue-279--283).

`GpuBackendPolicy::COMPOSITION_CAPABLE` only **makes the composition path available** (on Windows it narrows the backend to DX12, because the DirectComposition visual is a DX12 surface target). It does not put any window on that path. This narrowing is **unrelated** to whether the first window wants composition: otherwise opening an ordinary Settings window first would silently make the whole process lose composition capability, and the main window opened later could never get it. A window asks for it itself with `WindowSurfacePreference`:

- `Auto` (default): composition is used only when the process asked for composition capability and this window wants a transparent client area. Otherwise an ordinary window surface. So a transparent main window uses composition, while Settings / Dialog / Popup / floating panels in the same process are still ordinary windows.
- `NativeWindow`: always an ordinary window surface.
- `Composition`: explicitly asks for a composition visual. If it cannot get one, it falls back and reports, rather than failing to open the window.
- `RequireComposition`: a composition visual or no window. This is the **only** level that turns composition being unavailable into a startup/window-open failure. It is for hosts whose content is wrong on the ordinary path. Every other window should use `Composition`: fall back, report, and keep the application alive.

The composition target (DirectComposition visual) exists only on Windows. The ordinary window surfaces on macOS and Linux are already composited by the system (on macOS it is `CAMetalLayer`). There is no second path to ask for. So both levels are satisfied directly by an ordinary window there, do not count as a fallback, and never cause a window to be refused. Whether a transparent client area holds is decided as usual by the alpha mode the surface negotiated (with `Opaque`, `Transparent` falls back to `Solid` and is reported).

One window wanting composition does not mean all windows do. The #215 shadow companion can therefore use `Composition` on its own, without touching the application's other windows. Mixing both targets on one shared GPU device is normal.

#### Availability is decided before a window opens; the window itself is provisional

`WS_EX_NOREDIRECTIONBITMAP` is a creation-time flag: winit derives it from its own `NO_BACK_BUFFER`. `apply_diff` overwrites `GWL_EXSTYLE` as a whole. It cannot be set afterwards. So "can this machine use composition" must be answered before the first window. `GpuBootstrap` does this: a standalone DX12-only instance enumerates adapters (no surface; `WGPU_BACKEND` still takes precedence), **and that instance is kept for reuse by the real device request**. Probing and device selection are the same bootstrap. DX12 is not initialized twice, and the two cannot contradict each other. An embedded host's device already exists. There is no room to narrow, and no instance is created. Composition is only possible if the host is already on DX12.

But "there is a DX12 adapter" only proves the backend exists. The real composition path must still pass: composition device → `CreateTargetForHwnd` → `CreateVisual` → wgpu `CompositionVisual` surface → `PreMultiplied` negotiation. All of these happen **after** the HWND has already been created with `WS_EX_NOREDIRECTIONBITMAP`. So a composition window is **provisional**: if any step fails, its composition objects are released and the window is dropped (winit posts the destroy message). An ordinary window is created in its place. That HWND is never reused as an ordinary HWND surface. The reason is concrete. Without a redirection bitmap there is nothing for `DwmExtendFrameIntoClientArea` to blend, and that is the only per-pixel alpha source for a DX12 HWND swapchain. If it were kept, this window could never be transparent on either path. The window is created hidden. It never appears on screen.

After the device changes (device loss recovery), "can this process still use composition" is decided again from the new device's backend. The old answer is not reused: the new device may land on a different backend, and windows are created based on this answer.

This fallback applies to **every window**, not just the first: wherever a composition target is requested, it is provisional there. A composition failure for an auxiliary window likewise drops that HWND and reopens it as an ordinary window, rather than `OpenFailed`. At the same time, the process's availability is set to unavailable. What this window could not build, the next one cannot build either. There is no need for every window to first waste an HWND that is bound to fail.

The ordinary path is the last one. Only its failure is a real startup failure. A composition failure does not stop the application from starting, unless the window declared `RequireComposition`. The result is visible to upper layers: `ResolvedWindowPresentation::requested_target()` / `surface_target()` / `target_fallback()` (`SurfaceTargetFallback::{BackendUnavailable, TargetUnavailable}`), read from `RuntimeProgramContext::presentation()`. The acceptance probe uses `NANA_FORCE_COMPOSITION_FAILURE=1` to make the composition step fail artificially and walk through this fallback.

`WS_EX_NOREDIRECTIONBITMAP` belongs to the presentation path, not the material: DirectComposition draws the visual **on top of** the redirection bitmap. A composition window must not have that bitmap, or the opaque surface underneath shows through. Windows on the ordinary path always keep it. Their swapchain is very likely presenting into it. NRB + HWND + `Opaque` was measured to work in all four cases: creation, steady-state frames, `ResizeBuffers`, and a round trip through zero size. So keeping it is not a compromise.

Glass (`DwmExtendFrameIntoClientArea`) is applied only to transparent windows that **still have a redirection bitmap**: the DX12 HWND swapchain can only negotiate `Opaque`, and per-pixel alpha relies entirely on this glass. A composition window is created with `WS_EX_NOREDIRECTIONBITMAP`, and its pixels come from the DirectComposition visual that already negotiated `PreMultiplied`. Adding glass would only make DWM blend a surface that does not exist. When switching from Mica / Acrylic, the previously applied glass must also be withdrawn (margin `0`). The material layer reads that creation-time ex-style bit on the HWND. No separate API is added. Mica / Acrylic are themselves the DWM frame and still unconditionally `ExtendFrame(-1)`.

### One presentation, one chrome authority

`ResolvedWindowPresentation` is the **only** authority for what each window "is presenting". `crates/nana-ui/src/presentation.rs`:

```text
requested_material   Business request. Only decides what to ask the platform and surface for.
effective_material   What was actually obtained after surface negotiation, with MaterialFallback.
alpha_mode           The surface's answer.
surface_target       The presentation path this window actually uses (+ requested_target / target_fallback).
chrome               Native chrome policy derived from effective. None for system_caption windows.
```

`chrome` is decided in `resolve` together with `effective`: it is impossible to get an effective material without also getting the matching chrome. So the state "rendering thinks `Solid`, native chrome thinks `Transparent`" cannot be represented.

For that specific bug: a `Transparent` request + DX12 HWND surface → negotiates `Opaque` → effective falls back to `Solid(NativeMaterialUnavailable)` → chrome becomes the opaque policy at the same time, and the DWM glass already applied for this request is withdrawn (`needs_material_reset()`). Every later maximize / minimize / DPI change / style restore replays from `host.presentation.chrome()`. No second place re-derives it from requested. So it cannot fall back into transparent chrome.

The window is created hidden. Chrome is written only after the surface answers. The window is shown only after that. So the user never sees an intermediate state.

#### At runtime (effective `MaterialEffect`; implemented in `NativeChromePolicy` / `apply_native_chrome`)

| Effective material | Frame style bits | Corners | `DWMWA_BORDER_COLOR` | `DWMWA_NCRENDERING_POLICY` |
| --- | --- | --- | --- | --- |
| `Solid` | Kept | `DWMWCP_ROUND` | `COLOR_DEFAULT` | `ENABLED` |
| `Mica` / `Acrylic` / `Vibrancy` | Kept | `DWMWCP_ROUND` | `COLOR_DEFAULT` | `ENABLED` |
| `Transparent` (`StripFrameStyles`) | **Stripped** `WS_CAPTION \| WS_THICKFRAME \| WS_SYSMENU` | `DWMWCP_DONOTROUND` | `COLOR_NONE` | `ENABLED` |
| `Transparent` (`SuppressNonClientRendering`) | Kept | `DWMWCP_DONOTROUND` | `COLOR_NONE` | **`DISABLED`** |
| `system_caption: true` | The host touches nothing | | | |

The criterion is "is the effective material `Transparent`", not `wants_transparent_surface()`: Mica and Acrylic also make the latter true, but their background **is** DWM's non-client rendering. Rounded corners and the border must stay.

### Non-client suppression strategy: two paths, default decided by measurement

A transparent client area needs DWM to stop drawing the non-client area underneath. There are two paths with different costs (`NonClientRenderingStrategy`):

- `StripFrameStyles` (**current default**): removes the frame style bits. Measured to work. The cost is losing Aero Snap (Win+arrow keys, dragging to a screen edge to split), the Windows 11 Snap Layouts hover menu on the maximize button, the Alt+Space system menu, and the system maximize/minimize animations. The custom 8px resize (`LiveFrameResize`), custom window drag (`LiveFrameMove` / `WM_NCLBUTTONDOWN`), and the custom caption buttons do not depend on them.
- `SuppressNonClientRendering`: keeps the frame style bits and uses `DWMWA_NCRENDERING_POLICY = DWMNCRP_DISABLED` instead. If it holds, all the system capabilities above are kept. This is the default a general GUI framework would prefer.

**Which one is right is a measured conclusion, not a derived one.** `DwmSetWindowAttribute` returns `S_OK` whether or not it had the intended effect. So the code has no "detect failure and fall back" branch. It is a switch with a recorded default. In the first acceptance round `DWMWA_NCRENDERING_POLICY` never took effect. But at that time the main window **was not yet on the composition path**. That conclusion cannot be carried over to now.

Real-hardware comparison (Windows, the same machine, run twice):

```bash
cargo run --release -p nana-ui --features "hosted bundled-fonts wgpu-interop" --example native-content-probe -- --hold
NANA_WINDOWS_NC_STRATEGY=suppress cargo run --release -p nana-ui --features "hosted bundled-fonts wgpu-interop" --example native-content-probe -- --hold
```

Both times, read the three lines `PRESENTATION` / `PRESENTATION_TARGET` / `NC_STRATEGY` to confirm that this run really tests the target strategy. Then record each item: whether the transparent client area shows the caption / caption buttons / drop shadow through, Aero Snap, Snap Layouts hover, Alt+Space, taskbar and Aero Peek, maximize / minimize animations, resize, DPI change, maximize / restore round trip. Only if `suppress` fails on "transparency correctness" does `StripFrameStyles` stay the default. Otherwise change the default to `suppress` and write this table back into this section.

#### winit writes the frame style bits back

winit's borderless windows do not remove the frame style bits. `to_window_styles()` unconditionally writes `WS_CAPTION | WS_SYSMENU | WS_BORDER` for non-POPUP top-level windows, then adds `WS_SIZEBOX | WS_MAXIMIZEBOX | WS_MINIMIZEBOX` depending on resizable and so on (`MARKER_DECORATIONS` only strips the caption for `WS_CHILD`). It relies on `WM_NCCALCSIZE` to make the client area fill the window rectangle. So DWM keeps rendering a non-client area for this HWND, drawn **beneath** the client area: an opaque client area covers it. A transparent client area lets it show through. That is where the system caption buttons and drop shadow on transparent windows come from.

`apply_diff` overwrites `GWL_STYLE` as a whole whenever any `WindowFlags` bit changes. **This includes inside its own WndProc** (on `WM_DPICHANGED` when the size changes). The host cannot see that path. So stripping style bits cannot rely on replaying after every host call. It can only be blocked at the message: `install_style_guard` installs a `WM_STYLECHANGING` subclass.

**This subclass does not monopolize the message chain.** NanaUI is a framework that lives on other people's windows: native extensions, accessibility shims, and embedding hosts may all have their own subclasses on the same HWND. So the guard **first** passes the message down with `DefSubclassProc`, **then** clears the masked bits from `styleNew`. Placing this after forwarding makes it the final vote over every stage below it (winit's proc is there). A subclass installed after it runs before it and gets control back after it, so it can still write the bits back. This is an inherent property of Win32 subclass ordering, and the price of not monopolizing the message. Reinstalling the guard at the next style write makes it take effect again. Opaque windows use an empty mask to make it a pass-through. It does not need to be uninstalled. It calls `RemoveWindowSubclass` itself on `WM_NCDESTROY`. Calling `SetWindowSubclass` again with the same `(window, proc, id)` triple swaps the mask in place. It does not stack a second subclass. The host installs it on every chrome replay. The chain always has only one link. `crates/nana-window/src/chrome.rs` has a real-HWND fixture that verifies another subclass still receives `WM_STYLECHANGING` while NanaUI's invariant is still the last vote.

`arm_frameless_guard` only installs the guard and does not write styles, so it sends no window messages. It can be called in `can_create_surfaces`. Showing the window is itself an `apply_diff`: it rewrites the whole style (the guard clears the frame bits) and immediately sends its own `SetWindowPos(SWP_FRAMECHANGED)`. So the window has no frame from the first frame. The host does not need to send a frame change itself in the surface callback. `allow_caption_change` only decides "can styles be written directly now". It does not decide whether to strip.

### Three kinds of native changes; a move is only a move

Only paths that really make winit rewrite `GWL_STYLE` / `GWL_EXSTYLE` do chrome reconciliation. The rule is winit's: `apply_diff` overwrites styles as a whole only when some `WindowFlags` bit changes. Position and size are not flags. They are `SetWindowPos`.

| Entry | What it does | Typical calls |
| --- | --- | --- |
| `mutate_window_geometry` | Changes only the window rectangle. Does not touch chrome or redraw | `Move`, `SetBounds`, `Size` / `MinSize` / `MaxSize`, size adjustment from `SetContentAspectRatio` |
| `mutate_window_visibility` | `WS_VISIBLE` changes, chrome is replayed; the caller decides about frames | `Visible`, the bring-to-front show in `focus_window` |
| `mutate_native_style` | A window flag changed; replays chrome and requests a frame | maximize, minimize, fullscreen, window level, resizable, `set_cursor_hittest` |

So continuously dragging a window does not redraw, does not rewrite DWM corners and border, and does not reinstall the style guard. `LiveFrameMove` calls `SetWindowPos` directly anyway and does not go through these three entries. `CompositionWork::native_chrome_writes` counts how many times the host replays chrome. The real-hardware probe uses it to prove move-only never increments it (`NATIVE_MOVE_ONLY_PASS`). **Every** path that writes chrome must be counted, including the appearance-change path that goes straight to `apply_resolved_presentation`. Missing one opens a hole in the idle gate: the window rewrites frame styles every frame while the contract reads 0.

### DirectComposition is a retained compositor

The DComp tree is not committed at the GPU frame rate. Three gates, from cheap to expensive (`NativeContentMirror`, `crates/nana-ui/src/native_content.rs`):

1. Neither the scene's `projection_revision()` (`instance` + `attribute_epoch`, the pair the scene maintains for its own caches) nor the viewport changed → reuse the previous frame's answer. No nodes are scanned.
2. The scene has no native-content nodes → `native_content_regions()` does not run at all.
3. The computed regions equal what the backend got last time → not handed to the backend. The tree is not dirty. This frame's commit does nothing.

`IDCompositionDevice::Commit` happens only when the tree really has staged changes: `create_native_visual`, `set_geometry`, `set_visible`, `remove`, and visual destruction only **stage** (`touch()`). The host publishes them in one go. So with a 120 FPS HostTexture such as Live2D / video / particles, while the UI and native visual geometry are static, the **delta** after going idle is 0. Not "close to 0". The contract checks exactly `eq 0`.

Conversely: when the backend changes a visual itself (`set_visible` and so on), it marks the tree dirty, and the next frame commits as usual. Zero during idle means "nothing changed", not "changes are not sent".

Counters are read from `RuntimeProgramContext::composition_work()`:

```text
commits                             platform compositor transactions
tree_mutations                      staged visual tree changes
native_content.region_rebuilds      region extractions that walked the scene
native_content.regions_considered   regions produced by those extractions
native_content.regions_changed      regions actually handed to the backend
native_chrome_writes                times the host rewrote native chrome
```

`native-content-probe` opens four windows to cover these acceptance items: two composition windows (main + auxiliary), one ordinary opaque window (both targets mixed on the same GPU device), and one **transparent window pinned to the ordinary path** (the DX12 HWND surface can only negotiate `Opaque`, so it must fall back to `Solid` and get opaque chrome; this assertion runs on **every frame** it produces, so maximize, restore, and DPI changes midway are covered). Then, after geometry is static, it runs 120 frames in a row asserting the counters do not move (`NATIVE_STEADY_STATE`). Then it does the move-only check. Finally it writes the **delta** for this idle period to `target/performance/windows-composition-steady.json`.

This report feeds the Performance Contract: `perf/scenarios/windows-composition-steady.json` uses the same rule engine as the other scenarios to check all six counters as `eq 0`.

```bash
python3 perf/contract.py --self-test
python3 perf/contract.py --evaluate-invariants target/performance/windows-composition-steady.json
```

`windows_composition_tests` in `--self-test` verifies that this gate really blocks: all green for an idle window. Any one of the six counters becoming 1 must be red. A missing `composition_work` must be not-evaluable, not green. Real numbers can only be produced on real Windows + DX12 hardware. This is not an id in the Issue #8 §8.1 catalog, nor a weekly DoD gate. Machines that cannot reach the composition target **must not** write this report. No report is better than a fake all-zero one.

### There is only one commit authority

```text
RuntimeProgram / native backend  →  may only mutate the composition tree / native visual
Scene Host                       →  solely responsible for the transaction Commit
```

`native_content_frame` receives a `WindowsCompositionTree`. It has **no** `commit`. `commit` is on the host-owned `WindowsComposition`. Both sides committing at once is therefore unrepresentable.

Conversely, `native_content_frame` is **not** the only place the backend can change visuals. It is only called back when the scene's regions change. When the backend has its own reason to change a visual (the engine produced a new frame, the application hid a layer), it just changes the `WindowsNativeVisual` it holds: the change marks the tree dirty, and the next frame's commit sends it out naturally. Waiting for this callback means waiting for a geometry change that may never come. Advanced hosts that really need to commit manually (they drive the GPU context themselves and do not run the Scene host) take `WindowsComposition` directly. This is an explicit advanced API, not something the default callback can do in passing. `native-content-probe --surface-lifecycle` is such a host.

Custom title bar buttons are `WINDOW_CONTROL_WIDTH` wide and tall, vertically centered in the controls slot of height `TITLE_BAR_HEIGHT`. The button group keeps `WINDOW_CONTROL_PADDING` on both sides.

Hit order (logical pixels, current `scale_factor` already applied):

1. Custom window buttons (AccessKit names `Minimize`, `Maximize`/`Restore`, `Close`) come first. They do not start a drag. Where the outermost 8px resize area of the client area intersects the actual buttons, it yields too. Corners outside the buttons still support window resizing.
2. Empty title bar space emits `WindowChromeAction::Drag`: on macOS it is handed to AppKit at press, and `currentEvent` must still be a left-button press or drag, otherwise no drag. Other platforms emit it after the pointer moves more than 4px after press. The Scene host calls `nana_window::drag_custom_title_bar`; on non-macOS, if that fails, `winit::drag_window`. The native window drag takes over the button release. Right after the drag starts, the host sends one `PointerPhase::Cancel`, ending the button mask, Runtime press/capture, the title bar gesture, and the program-side gesture. `WindowService::begin_drag` works the same way.
3. With no system caption, resizable, not maximized, and not fullscreen, the outermost `RESIZE_HANDLE_SIZE` (8px) of the client area goes through `LiveFrameResize` (macOS `setFrame`, Windows `SetWindowPos`). It does not enter the system's nested size-move loop. System caption windows do not stack a second set of resize hit areas.

`WindowCommand::Drag` is the single signal for "move this window with the gesture currently in flight": title bar drags and custom gestures defined by the consuming application (for example a whole-window middle-button drag) send the same one. The host dispatches by the pressed button. By default the primary button takes the native window drag above. A window that sets `WindowDescriptor::host_managed_drag = true` also hands the primary button to `LiveFrameMove`, so it does not enter the Windows nested move loop, and the host event loop can keep driving other windows. Other buttons still go through `LiveFrameMove` (macOS `setFrameOrigin`, Windows `SetWindowPos`), because platform window drags only accept the primary button. AppKit rejects them outright. The Win32 caption move loop only finishes on primary-button release, so the window would stick to the cursor. A host-managed move ends when that button is released, a new primary button press arrives, Esc is pressed (restores the original position), or focus is lost. On end, one `PointerPhase::Cancel` is sent. Fullscreen windows do not move. Maximized windows are restored first. The restored window is placed back under the cursor, with the cursor's proportion across the width and its distance from the top edge the same as at press (like the system window drag; it does not jump back to its original restore position). During the move, pointer events are consumed before reaching the document. The move itself does not change the drawable, so it does not redraw. Native mode keeps platform drag abilities such as system edge snapping. Host-managed mode only keeps NanaUI's continuous event loop and pointer following. This path exists only on macOS and Windows. Other platforms still fall back to the native window drag.

The window cursor also consumes common CSS `cursor` keywords: `default`, `pointer`, `text`, `move`, `grab`, `grabbing`, `not-allowed`, `crosshair`, `help`, `wait`, `progress`, `zoom-in`, `zoom-out`, `none`. The property is inherited as in CSS. Unknown keywords and `url()` cursors fail closed. The programmatic `WindowCursor` entry matches the keywords above, plus `Automatic` to restore the Runtime/CSS choice. Cursor priority is below window frame resizing and split/dock/workspace resize handles, and above TextInput's I-beam cursor when no cursor is declared. `none` only hides the system cursor. It does not load custom images.

### Live resize

When the frame is dragged in the client area, pointer movement changes the window rectangle directly, and the event loop keeps running. `SurfaceResized` syncs geometry and requests the next frame. When drawing a frame, `surface.configure` runs only if the physical size or present policy changed. Same-size frames skip it. Windows system frame resizing may still go through `WM_ENTERSIZEMOVE`. Steady-state frames use `Mailbox` (`Immediate` if unavailable, then `AutoVsync`), to avoid FIFO following the primary display's compositor clock under mixed refresh rates. `LiveSizeMove` keeps the same present mode and raises frame latency to 2. Transparent windows take the same path.

DPI and multiple displays: pointer, drag, and resize all use logical coordinates. Physical pixels are only used for the Surface. Window positions are recorded by the host and clamped to the current display's work area before creation (if the original display is disconnected, centered on the primary display). Auxiliary windows with a `parent` (modal and non-modal, excluding Underlay) stay above their parent, remain independently draggable top-level windows, and are not confined to the parent's client area. On Windows, before first show, `AuxiliaryWindow::attach` writes `GWLP_HWNDPARENT` to bind the parent HWND, so the window minimizes with its parent and has no separate taskbar button (a subclass guard blocks the `WS_EX_APPWINDOW` that winit writes back on every style change). This is not left to winit's `with_owner_window`: that turns the owned window into `WS_POPUP`, drops the system title bar, adds `WS_EX_NOACTIVATE`, and converts the position relative to the parent's client area (see the upstream fix in [winit#4753](https://github.com/rust-windowing/winit/pull/4753)). macOS uses the AppKit child-window relationship; Linux X11 uses `WM_TRANSIENT_FOR`, and Wayland uses `xdg_toplevel.set_parent`, with ordering enforced by the window manager / compositor. Embedded child windows on Windows / X11 cannot be used instead, and ordering is not maintained by repeated focusing or global always-on-top.

The effective level of an auxiliary window is derived by the host in one place: when the parent is always-on-top, the auxiliary window also enters the always-on-top level; after the parent returns to the normal level, the auxiliary window's own request applies again. `ModeChanged.level` reports the effective level. Creation, hide-then-show, and parent level changes all use live parent state, so the application does not need to copy the always-on-top setting or forward commands. Windows inherits the level first in the native creation attributes, and checks the actual topmost flag after window style, shadow, and show/hide operations; it corrects the flag only if it does not match the effective request, so the framework cache does not keep believing the window is topmost after its owner group changes. The native window-order commit point also keeps the requested topmost target, so a hidden IME window taking part in owner-group resolution cannot leave the main window at the normal level; during native fullscreen the level is managed by the backend, and its temporary level is not overridden. When the main window is first hidden, an auxiliary window used for preparation can be shown independently; an association that needs a shown parent is deferred until the parent's first frame commit. Wayland clears the association when the parent is unmapped, so the first frame commit after each re-show restores it; it does not wait for GPU completion or block the UI. The association is released before the child's native resources, and closing the parent still closes its children first. A failure to initialize the association is reported through the existing `OpenFailed` / window creation request error. The native acceptance entry is `cargo run -p nana-ui --example auxiliary-window-probe --features hosted,bundled-fonts,controls`: the main window starts hidden and a preparation window is shown; `main/top/normal/hide/show/focus/minimize/restore/close/reopen/quit` on stdin drive the real host. JSON mode and focus events must be checked against the actual window order on the target platform.

Underlay windows (`WindowRole::Underlay`, which must have a `parent` and cannot be modal): a native window with its own surface that always covers the parent's client area and sits directly below the parent (at the same always-on-top level as the parent). It follows the parent's move, resize, fullscreen, minimize, and hide. It takes no input and never activates; all clicks land on the parent above it. The purpose is to split what looks like one window into two native windows: the parent is the transparent UI layer, and the underlay window draws the content. It has its own title, so screen recording software can list and capture it on its own, and the capture contains none of the parent's UI. The host fills its geometry from the parent and forces it to be transparent, shadowless, absent from the taskbar, with no persisted geometry and no accessibility tree (the parent's tree is the one screen readers read). It keeps producing frames when covered by the parent. macOS keeps the two windows independent of each other, so ScreenCaptureKit's `includeChildWindows` does not pull the whole native association group (including the UI) into the capture. The back layer is corrected synchronously from main-thread AppKit move / resize notifications, so following completes inside the native operation without waiting for the application event queue or a redraw. Focus, application activation, minimize, and restore notifications synchronize visibility and order, and the host keeps correcting on mode and visibility changes. The parent's shadow still uses the parent's shape, but its native companion is re-hung below the underlay window; when the underlay window closes it returns below the parent, keeping the UI, content, shadow order. A capture that includes child windows only additionally includes the shadow companion, never the UI; turning off `includeChildWindows` takes only the content's own surface. On Windows the underlay is not owned by the parent (owned windows sit above their owner), and it is not `WS_EX_TOOLWINDOW` (OBS's window list filters those out): the parent's subclass places it after the parent within the same `WM_WINDOWPOSCHANGED` message, and hides it with DWM cloak on minimize or hide. Its own subclass guarantees `WS_EX_NOACTIVATE` and `HTTRANSPARENT`, and keeps it inserted after the parent when reordered. Other platforms fail to open (`Unsupported`). Real-hardware entry: the `underlay-probe` example in `nana-ui` (`UNDERLAY_PROBE_STEPS=1` automatically moves, resizes, minimizes, and restores).

On macOS, a parent window with an underlay automatically uses `LiveFrameMove`, so the UI and the back layer update their positions in the same input handling. The system's native drag moves the UI first and then sends the move notification, so an independent back layer can appear to lag even when synchronized inside the notification. That is why, in this case, AppKit's native drag loop is not entered, and the native drag's edge snapping and Spaces gestures are not used. After the underlay window closes, the descriptor's original drag policy is restored. Capture stays independent, and moving requires no new drawable or GPU wait.

On macOS, geometry snapshots and the start of a host-managed move query AppKit's zoom state read-only through `nana-window::native_window_maximized`; windows without a title bar or not resizable report not maximized, and fullscreen is not counted as maximized. Geometry reads must not temporarily change the style or frame, or the underlay window would generate more notifications inside move / resize notifications and form an event loop. `underlay-probe` has a regression assertion for consecutive notifications with unchanged geometry. `underlay-native-probe` in `nana-window` verifies on real AppKit windows that the capture group is independent, that 64 moves and resizes synchronize without manual `sync` or event pumping, the shadow order, and notification unbinding and hiding after release.

IME: when focus enters an editable field, `Window::request_ime_update(Enable)` is sent once (hint / purpose, caret box, surrounding text for non-password fields). After that, caret or surrounding changes go through `Update`, and nothing is sent if the state equals the last applied one. A new session starts only when the field (owner) or purpose changes: first `Disable`, then `Enable`. When the same field temporarily cannot provide a caret or surrounding text (after an edit, before the new layout), the previous value in the session is reused and the session is not torn down: tearing it down would cancel the composition in progress. It reopens only when a new capability appears. After leaving the editable field, `Disable`. When the native window loses focus, Runtime cancels the preedit and keeps field focus; the host records a preedit reset, and also resets on focus gain. It must not merely forget the applied state and downgrade `AlreadyEnabled` to `Update`. A focus loss followed by a focus gain within one turn must also keep the reset edge. If there is an editing field, the session is restored; if not, it is not enabled. IME is not continuously blocked based on key-window state, so the candidate window does not get in the way of editing. macOS cancels native conversion only for the current key window: when the old window's focus-loss message is dequeued, the new window may already be active, and the current conversion session must not be canceled through the client that lost focus. When the old window regains focus it must reset, and before resetting, disabling, or replacing the session, `nana-window::discard_native_ime_preedit` calls the text client's `unmarkText`: it first clears the view's marked range, then winit cancels the input context's `discardMarkedText` and returns to Ground, without committing the old text. Canceling only the context while keeping the client's marked range is not allowed; clearing only winit's marked-text buffer is not enough to cancel an old Simplified Chinese Pinyin preedit. When the editing field and capabilities are unchanged, AppKit's active input context is kept and synchronized through Update; do not close and reopen the same session after unmarkText. On Windows, `ImmNotifyIME(NI_COMPOSITIONSTR, CPS_CANCEL)` is sent to the input context outside winit's lock before Disable: winit calls `ImmAssociateContextEx` while holding the window state lock, IMM synchronously sends back `WM_IME_ENDCOMPOSITION` when ending an active composition, and the window procedure deadlocks taking the same lock again (see the upstream fix in [winit#4798](https://github.com/rust-windowing/winit/pull/4798); this step can be removed after upgrading to a version that includes the fix). Other backends cancel the native preedit through Disable / Enable. During routing, the host only records the state Runtime wants and sends it to the window at the end of this turn's `about_to_wait`: calling IME while key and composition messages are being processed re-enters the window procedure (winit's Win32 backend calls IMM while holding the window state lock, and the messages IMM sends back synchronously take the same lock again and deadlock). winit's empty `Ime::Preedit` (whether or not cursor is None) means clearing the preedit, and the bridge must send `CompositionInput::End`; it must not send an empty Update, or Runtime would re-enter an empty composition and block editing operations such as select all. Text that the platform attaches to Command or Control shortcuts (not including AltGr) must not be inserted as input, even when the shortcut operation produces no change because the selection already satisfies it; only the attached text with the same sequence number is suppressed, and application routing of the key itself is kept. A failure of the native request or of enable-data validation is reported through `HostFailure::ImeDispatch`, and the applied cache is revoked; the session must not be silently assumed enabled. The candidate window is relative to the caret, not to the system non-client area. AccessKit incremental updates and visual geometry use the same layout boxes. No dangling `parent_and_index` may appear during composition.

Transparent alpha (`settings.transparent`) forces `MaterialEffect::Transparent`. It does not retry Mica / Acrylic. On failure it can only fall back to opaque solid, with a `MaterialFallback`. Real-hardware entry: the `transparent-window` example in `nana-ui`.

## Menu bar

The native application menu bar is the **only** desktop chrome that **cannot be drawn into the UI tree**: on macOS it belongs to the application, not the window, and lives in the system menu bar. So it is in `nana-window`, described by a platform-neutral model (`MenuBar` / `Menu` / `MenuEntry` / `MenuShortcut`; the model itself is in `nana-ui-core` and is pure data).

The application **declares** the menu. The host **installs** it by calling `window.set_menu_bar(bar)`. Ordinary controls never get the window handle, and on Windows the menu belongs to the window. So installation must be done by the Scene host that holds the window, on the same path as `SetIcon`.

Selected items come back through `take_menu_activations()`: drained once per frame. What you get is the `id` of a `MenuEntry::Item`. **The framework only reports which item the user chose. What the id means is still decided by the application**, on the same principle as `SecondaryPress`. See `examples/component-gallery`: menu ids are mapped to exactly the same business messages as the UI operations.

Platform support is not equal. `menu_bar_support()` reports it honestly. It does not pretend:

| Platform | Result | Notes |
| --- | --- | --- |
| macOS | `System` | System menu bar. The first menu lands in the application menu position and holds application-level commands. `install_application_menu_bar` can also be used directly without a window |
| Windows | `InWindow` | An in-window `HMENU`. Selection arrives through `WM_COMMAND` and is retrieved by a hook installed with `SetWindowSubclass` |
| Others | `Unavailable` | Nothing is installed. Put these commands in the UI |

`installed_menu_bar()` reads back the menu the platform actually holds (macOS), for host self-checks. `crates/nana-window/examples/menu-probe.rs` uses it for real-hardware acceptance.

For a menu bar drawn in the UI that follows the theme (custom title bars, Linux), use the runtime component [`AppMenuBar`](https://github.com/sena-nana/NanaUI/blob/main/docs/components/app-menu-bar.md). It reads the same model, and selected items go into the same `take_menu_activations()` queue.

Menus are replaced as a whole: calling `SetMenuBar` again replaces the entire bar. There is no incremental entry API. Rebuilding a menu is cheap. Diffing across three platforms is not.

## Tray

A tray icon cannot be drawn in the UI tree any more than the menu bar can: it is the icon at the right of the menu bar on macOS and the notification-area icon on Windows. It belongs to the application, not to a window: it is how the user gets back to an application whose windows cannot be clicked (click-through, hidden).

`install_tray(&Tray, wake)` shows it. `Tray` is plain data: a tooltip, a PNG icon, a menu (the same `MenuEntry` model as the menu bar), and the id a primary click reports on Windows. Choices go to the same `take_menu_activations()` queue. A choice can arrive while the application draws no frames, so `wake` runs once for every id queued: the host wakes its event loop there (by sending itself a program message, say) and then drains.

Calling `install_tray` again updates the menu, tooltip and icon in place, and the icon keeps its position; `remove_tray()` takes it away. Call both on the thread that runs the event loop.

| Platform | `tray_support()` | Notes |
| --- | --- | --- |
| macOS | `MenuBarExtra` | `NSStatusItem`. A click always opens the menu; `primary` has no effect. `template_icon` lets the system draw the icon in the menu bar's colour |
| Windows | `NotificationArea` | `Shell_NotifyIcon`. A right click opens the menu; with `primary` set a left click reports that id, otherwise it opens the menu too. The icon is added back after Explorer restarts |
| Others | `Unavailable` | Nothing is installed. Keep the commands in a window |

`installed_tray()` reads back the tooltip and menu entries the platform actually holds (macOS); `crates/nana-window/examples/tray-probe.rs` uses it for native acceptance.

## Start at sign-in

`login_item_state(&LoginItem)` and `set_login_item(&LoginItem, enabled)` read and write whether the application starts at sign-in. The state is read from the system every time: the user can change it in system settings, so the application does not remember it.

`LoginItem` gives the entry's stable id, its display name and the command to start. The command may be the running executable or a program that starts it (a store client, say).

| Platform | Where it is kept |
| --- | --- |
| Windows | The current user's `Run` registry key; the value is named `id` |
| macOS | The main-app login item (`SMAppService`), macOS 13 and later. It registers the running application bundle and ignores the command; outside a bundle it is `Unavailable` |
| Linux | `$XDG_CONFIG_HOME/autostart/<id>.desktop` |

## Path normalization and display

The public path interface is `nana_ui::path` (backed by `nana_ui_platform::path`):

- `normalize_path(&Path) -> Cow<Path>` does not touch the disk. It removes `\\?\` only when the ordinary Windows path and the extended path have the same semantics; UNC becomes `\\server\share\…`. Long paths, trailing dots or spaces, DOS reserved names, device/volume paths, and non-Unicode paths are kept as is. Other platforms do not rewrite paths.
- `canonicalize_path(path) -> io::Result<PathBuf>` applies the same normalization after `std::fs::canonicalize`; errors are returned unchanged. It touches the disk, is called only on worker threads that allow file I/O, and must not be used for UI formatting.
- `display_path(path, wrap_separators) -> String` does not touch the disk. Ordinary drive letters and UNC are displayed without the extended prefix (including long paths); special names and device paths keep their original spelling. `true` inserts U+200B after separators so narrow cards can wrap.

Read-only path summaries use `display_path(path, false)` and pass it to Runtime `PathText`, `LabeledValue::path_value`, or `SettingsRow::path_hint`. They measure with the same text engine against the actual available width and elide middle directories, preferring to keep the root and the tail; when very narrow they drop parent directories and the root first, then shorten the final name by Unicode grapheme while preferring to keep the extension. The full value stays in the accessibility tree and in a wrapping Tooltip that appears after 350ms of hover; hiding, parking, or unmounting the page closes the tooltip. Ordinary text still follows the original control's rules.

Display strings are only for read-only text; file access, saving, copying, editing, resource identity, and safe-directory checks keep using the real path. Do not write text containing U+200B back into settings, protocols, or the clipboard. `PathField`'s value and `TextChanged` are passed through unchanged; native file dialogs still return real paths and do no display formatting.

## File dialogs

System file dialogs need a parent window handle: macOS attaches them as a sheet, and Windows needs an owner HWND. Handles exist only at the host layer. So dialogs take the same path as the menu bar: the model is in `nana-ui-core` (`FileDialogRequest` / `FileDialogResult` / `FileFilter`), the application requests through `WindowHandle::open_file_dialog`, and the host executes it. Controls still never get the handle: `PathField` only emits `BrowseRequested`, which the application translates into a request.

The result is **asynchronous**: the host returns it through `WindowEvent::FileDialogCompleted { id, result }` and actively wakes the event loop. `id` is the window identity. `result.id` is the application's `u64` request identity. The application stores the business object and edit baseline for the request, then consumes the result. There is no global result queue, and no per-frame polling is needed.

Each window can have only one active dialog. Rejections come back through a separate `WindowEvent::FileDialogRejected { id, request_id, error }`. A second request with a different identity receives `FileDialogError::Busy` and does not override the first request. A duplicate active identity receives `DuplicateRequest`. The consumer keeps the original pending request and does not treat the rejection as completion of the active request. When the window closes, the active request receives `WindowClosed`. The host assigns an internal token to each open, so late callbacks after close (including reused window or request IDs) never complete a new request. Each accepted request completes exactly once. The host also holds the native session handle: on close it ends the macOS sheet, closes the Windows worker's picker, or cancels the portal/zenity. A worker that keeps the parent handle does not leave a visible orphan window.

**Cancellation is not an error**: `result.error` is `None` and `paths` is empty. Observable platform/thread errors are returned through `FileDialogError::Platform`. Unsupported targets return `Unavailable`. On Windows, the user closing the dialog is `HRESULT_FROM_WIN32(ERROR_CANCELLED)`. Other HRESULTs are `Platform`. The two are not mixed. `PickFolders` and `OpenFiles` return multiple paths. The others return a single path or cancel. Filters, the initial directory, and the save file name are kept in the request. An initial directory that does not exist or is inaccessible is skipped, and the dialog opens at the system default location. This is not treated as cancellation.

| Platform | Execution |
| --- | --- |
| macOS | Main-thread `NSOpenPanel` / `NSSavePanel` sheet, completed in a callback; supports file, multiple files, directory, multiple directories, and save |
| Windows | `IFileOpenDialog` / `IFileSaveDialog` on a dedicated worker thread, with the parent window as owner HWND; `FOS_PICKFOLDERS` picks directories; does not block host rendering |
| Linux | Dedicated portal worker thread with the parent window identifier; the response is subscribed before opening and correlated by the request path actually returned, compatible with older portals; when unavailable, falls back to a cancelable zenity fallback that reaps its child process |
| Others | Returns `Unavailable`; never hangs silently |

**Selected file type**: `result.selected_filter` is the index (0-based) in `request.filters` of the type selected at close. On Windows it comes from `IFileDialog::GetFileTypeIndex` (natively 1-based; out of the request's range becomes `None`), reported for both open and save. The Linux portal takes it from `current_filter` in the response: first matching the filters sent in the request by name and patterns together, then by name; older portals that do not return it give `None`. The macOS panel only has a merged extension list, and zenity does not output the selected type, so both are `None`. Directory selection does not apply filters; requests without filters, cancellation, and failure are all `None` too. Hosts or tests inject results with `FileDialogResult::selected(..).with_selected_filter(..)`; headless hosts use `HeadlessFileDialog::SelectWithFilter(paths, index)`. `FileDialogRequest` and `FileDialogResult` are marked `#[non_exhaustive]` and are built with constructors and builders.

**Save type linkage**: the Windows save dialog subscribes to `IFileDialogEvents`. `OnTypeChange` sets `SetDefaultExtension` to the new type's first concrete extension, and replaces the visible file name's extension with the new type's only when the name ends with the extension of one of the request's types; a dot inside a name like `v1.2` does not count as a suffix and is left to the default extension. Wildcard types (`*`) keep the original default extension and file name. macOS, portal, and zenity do no linkage; their native behavior decides.

**Enforced extension**: `FileDialogRequest::enforce_extension(true)` only applies to save and is off by default. On completion, `FileDialogRequest::apply_extension_policy` handles it: when the file name does not end with one of the selected type's extensions (comparing the end, case-insensitive), the type's first extension is appended; it never uses `Path::with_extension`, which would cut `.2` off `v1.2`. When the platform did not report the selected type, ending with any extension in the request is enough; otherwise the first type's extension is appended; wildcard types accept any name. Every platform's completion callback and the headless host go through this step. Windows also precomputes it in `OnFileOk`: when the file with the appended extension already exists (or this cannot be determined), it fills the completed name back into the dialog and returns `S_FALSE` to stay open, so the native overwrite confirmation on the next save targets the file actually written; when it does not exist, the dialog closes directly and the completion callback appends the extension. macOS's `NSSavePanel` does not allow other types and appends the extension itself, so the policy usually has nothing to do. The overwrite confirmation of portal and zenity only covers the name the user typed: if the file after appending already exists, there is no further native confirmation, and the application should check before writing if needed.

The Linux portal returns a URI array and can keep newlines in paths. The zenity fallback uses newline-separated CLI output for multi-select. When file names themselves contain newlines they cannot be split unambiguously. Single select strips only one protocol-terminating newline and keeps the actual file name. This fallback's multi-select boundary cannot be used as grounds for claiming support for arbitrary paths.

`describe_configured_dialog(&request)` reads back the platform's actual configuration (title, starting directory, extensions) without presenting the dialog. `crates/nana-window/examples/file-dialog-probe.rs` checks this configuration. macOS reads back all three from the AppKit panel. On Windows the starting directory comes from `GetFolder`. `IFileDialog` has no GetTitle / GetFileTypes. The title is the echo after a successful `SetTitle`. Directory selection applies no filters (so extensions are empty) and no prefilled file name. Real interaction uses `crates/nana-ui/examples/hosted-file-dialog-probe.rs`: inside the application window it covers the five kinds of selection, duplicate and busy rejection, cancellation, and window exit; `save-typed` uses two types and `enforce_extension` to check type linkage, `selected_filter`, and extension appending (type `v1.2`, switch type, overwrite an existing file), and observes continuous `window_frame_presented` while the dialog is open. Configuration checks and cross-compilation cannot replace native interaction acceptance on each platform.

## Reveal in file manager

`reveal_in_file_manager(&path)` (exported by `nana_window` and by `nana_ui` with the `hosted` feature) shows a path in the system file manager: a file is selected in its containing directory, and a directory is opened. The file manager is another process and does not need a parent window handle. The call hands off the request and returns. It does not wait for the file manager itself.

| Platform | Execution |
| --- | --- |
| macOS | `/usr/bin/open -R` selects files; ordinary directories are opened with `open`. Directories with an extension (packages such as `.app`) are also selected with `-R`, so `open` does not launch them or hand them to an application |
| Windows | `explorer.exe /select,"<path>"` selects files, `explorer.exe "<path>"` opens directories; arguments are passed to explorer verbatim, and the path is first made absolute and safely normalized through the public `normalize_path`. Ordinary paths use backslashes; paths that must keep extended semantics are not forced to drop the prefix. explorer also returns 1 on success, so the exit code is ignored |
| Linux | Calls `ShowItems` (files) / `ShowFolders` (directories) on `org.freedesktop.FileManager1` on a worker thread; if no service answers, falls back to `xdg-open` to open the directory (for a file, its parent directory). Failures of this step are not reported back |
| Others | `RevealError::Unavailable` |

A path that does not exist returns `RevealError::NotFound` and starts no process. Failing to read metadata or to start the process returns `RevealError::Unavailable(reason)`. Child processes are reaped on a background thread, leaving no zombie processes. Unit tests only cover path classification and the commands built for each platform; they do not start the file manager. Real display needs manual confirmation on the target platform.

## Icons

Icons on the taskbar, the exe, and the Dock are application identity, not the `Icon` glyphs inside the UI.

- Rust: `register_application_icon`, or `WindowDescriptor::icon` / `WindowHandle::set_icon`
- When unset, a default geometric mark is used. Do not treat it as a brand
- When the application always registers its own icon, it can turn on `nana-ui`'s `no-default-app-icon`: the default mark (a PNG of about 1 MB) is no longer linked into the binary, and windows without an icon use the platform default. Only the application itself turns this on; libraries must not. The default ICO/ICNS used at packaging time (`embed_windows`, `encode_icns`) is not affected
- A Windows exe can call `nana_app_icon::embed_windows()` in `build.rs`
- macOS Dock: `nana_window::set_application_icon_png`. The `.app` is generated by `nana-packager` (see [Packaging and distribution](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/packaging.md))

## Packaging

Release artifacts use the `dist` profile. Not `release`, and certainly not `debug`:

```bash
cargo build -p component-gallery --bin component-gallery --profile dist
cargo run -p nana-packager -- macos-app --exe target/dist/component-gallery --name NanaUI --identifier dev.nanaui.gallery --out target/dist
```

`dist` = `release` + `lto = "fat"` + `codegen-units = 1` + `strip = "symbols"` +
`panic = "abort"`. `release` stays as it is, so CI and benchmarks keep iterating quickly.

`macos-app` only produces a bare `.app`. It replaces the former `nana-package-app`. For a complete product package (resource packs, manifest, Steam/installer layout, final artifact verification), use `nana-packager package`; see [Packaging and distribution](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/packaging.md). Both run `strip -x` again by default (`--no-strip` turns it off). When the executable contains overflow-check panic text, they warn (debug builds always have it; release builds for some targets may too), and record `debug_assertions_suspected` in the manifest. A 108 MB `.app` once turned out to be a debug build packaged by mistake, 55 MB of which was the symbol table.

Use `scripts/report-artifact-size.py <artifact>` to check size section by section: it counts how many copies of fonts are embedded in the binary, and it also warns when it finds a debug build.

## Materials

Request **one** system effect through `RuntimeProgram::window_material_mode`. When the host provides an Appearance setting, Mica / Acrylic / Vibrancy can be selected. On failure it falls back to solid and gives a reason. It does not retry another effect.

| Platform | Can request | On failure |
| --- | --- | --- |
| macOS 10.10+ | The specified Vibrancy / UnderWindowBackground | Opaque theme background |
| Windows 11 | The specified Mica **or** Acrylic | Opaque theme background |
| Windows 10 1809+ | The specified Acrylic | Opaque theme background |
| Linux | No system blur API | Opaque theme background |

`Translucent` only turns on window transparency. It is not blur. A transparent window and system blur are two different things.

A transparent (`Transparent`) window turns off the system shadow on macOS, and it is restored when switching back to solid. AppKit generates the shadow from the window alpha. Whatever is drawn in a transparent window (a character outline, a feathered glow), it traces a ring around it, and it also stacks with the shadow the application draws for its own card, producing two layers. On Windows, transparent windows have no system shadow anyway. The application draws the edges itself. Vibrancy fills the whole window. The shadow is kept as usual.

Currently, requesting Vibrancy on a GPU window on macOS may not produce the system effect (the Metal layer covers the system material). The Windows transparent client area and Mica / Acrylic are judged on real hardware. A successful compile does not mean it looks right on that machine. Compare against `crates/nana-ui/examples/transparent-window.rs`.

Native materials are applied by `nana-window`: `apply_system_material` / `apply_hosted_system_material`. `run_runtime` applies, refreshes, and cleans them up separately for the main window and each tool window. A theme or material switch first clears the old effect, then retries with the current request. After device recovery, the current request is applied again. When native succeeds, the overlay colors for the sidebar/primary/title bar come from the Runtime Style Model (`ThemeTokens::with_backdrop`), not a whole-window clear.

## Multiple windows

Ordinary applications create windows through `context.windows()` and control the current window through `context.window()`. `WindowHandle` can be cloned and sent to worker threads. It does not hold the native window. All operations are queued back to the window thread. Size and position use logical coordinates.

```rust
// Worker thread; in async code you can also use .await.
let window = service.create_window(WindowDescriptor {
    title: "Notes".into(),
    initial_size: (480.0, 320.0),
    minimum_size: (240.0, 120.0),
    system_caption: true,
    ..Default::default()
}).wait()?;
window.set_title("Preview").wait()?;
window.set_size((640.0, 480.0)).wait()?;
window.close().wait()?;
```

All `WindowDescriptor` fields are public, and more will be added as platform capabilities grow (`shadow` and `host_managed_drag` were added that way). Always finish construction with `..Default::default()` or the `WindowDescriptor::new(...)` builder. A struct literal that spells out every field fails to compile the next time a field is added.

The create result completes only after the hidden native window, the Surface, input state, and the application document have initialized successfully. Failure rolls back, and no `Ready` is sent. The exception is a main window configured with Early Splash: the window is shown first with the logo, and the device and program become ready after it. On failure the logo is taken down and the window closes. See [Two-phase startup](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/startup.md). `ApplicationState::build` builds a separate document for each window. A custom `RuntimeProgram` builds in `initialize_window` and undoes failed application state in `discard_window`. Only on success is the window registered and shown according to `WindowDescriptor::visible`.

Window ids are assigned by the service. The application uses `WindowDescriptor::tag` to declare what kind of document this window is. Do not rely on the order of create requests to match ids. The tag is opaque to the host. Read it back from `RuntimeProgramContext::window_tag()`: it is readable when `initialize_window` / `ApplicationState::build` builds the document, and in every later callback for that window (including `Ready`). After the window closes it is `None`. State held by id is cleaned up in `window_closed` / `discard_window`.

```rust
let character = service
    .create_window(WindowDescriptor::new("Character").tag("character"))
    .wait()?;

fn build(&mut self, window: &mut ApplicationWindow, context: &RuntimeProgramContext<Message>) -> Result<(), Error> {
    match context.window_tag() {
        Some("character") => build_character(window),
        Some("tracking") => build_tracking(window),
        _ => build_main(window),
    }
}
```

A program-chosen `WindowId` + `WindowCommand::Open` is still only a channel for framework adapters (Dock). Ordinary applications do not need it to distinguish window kinds.

Operations return `WindowRequest<T>`, which supports `.await`, `.wait()` on worker threads, and non-blocking `try_take()` on the window thread. Calling `.wait()` on the window thread returns `HostThreadWait`. It does not block the event loop. At most 1024 window requests can be pending. When the queue is full, `QueueFull` is returned immediately. The caller can wait for submitted requests to complete and retry. A handle used after close returns `WindowClosed`. After the host is released, `HostStopped` is returned. Identity and generation checks keep old requests from acting on a recreated window.

The main window and additional windows share the registry and the Device/Queue. Each window owns its own Surface, input, IME, document, and render target. Windows with the same texture format share one Scene painter. Each document's compositor motion descriptors are identified by `MotionDescriptorStore::source` and the structure epoch, and are re-uploaded when the painted window switches, so one window never reads another window's animations. By default, closing a window releases only that window and its native child windows. Standalone mode exits after the last window closes. The application can still explicitly return `RuntimeProgramUpdate::exit()` to close the whole application.

`ApplicationState::window_event` and `RuntimeProgram::window_event` receive framework window events. Pointer and keyboard input arrive as `RoutedInput` through `RuntimeProgram::input_event`, with hit and handling results attached. The same input is never dispatched twice. Scale changes are notified through `Resized` with the new `scale_factor`.

### Pointer presence

`WindowEvent::PointerPresenceChanged { id, inside }` reports whether a hovering pointer (mouse, pen) is inside the window's client area. It is sent only on change. It is for applications that reveal chrome on pointer movement, so they do not need to poll the cursor. A leave in the input stream is still normalized to `PointerPhase::Cancel`, which is indistinguishable from a gesture cancel, so do not use that to infer presence.

- Touch contacts do not report presence.
- When the host starts a native window drag (title bar drag, `WindowHandle::begin_drag`), leaves produced by the platform move loop are held back until the platform reports that pointer again. If the drag is stopped by a screen edge and the pointer leaves the window, the leave is reported only after the pointer comes back into the window and leaves again.
- During a host-managed window move (`LiveFrameMove`), presence changes are not reported: the window follows the pointer, and each time it crosses its own former bounds it is the window leaving the pointer, not the pointer leaving the window.
- While the pointer is captured, moves outside the client area do not change the state back to present.
- When `WindowHandle::set_visible(false)` hides the window, one leave is reported. When the window is minimized or fully covered (the platform reports occluded), one leave is reported as well. The platform may not send that leave itself. While occluded, Forward pass-through does not sample that window. After it is revealed again, it returns to present only when the platform reports the pointer again.
- For Forward pass-through, the host samples the global pointer, and presence is updated from the samples. Platforms where sampling is unavailable (Wayland) report not present.
- With a modal child window, the parent still reports presence. Pointer events themselves continue to be handled by the modal chain.

### Reduced motion

`RuntimeProgramContext::reduced_motion()` returns whether the system asks for reduced motion. It is read when the window is created. When the user toggles it at runtime, the host sends `WindowEvent::ReducedMotionChanged { id, reduced }` to every window, and later callback contexts read the new value. The application uses it to choose transition durations or turn off movement. NanaUI does not rewrite animations the application has declared.

- Windows: reads "Show animations in Windows" (`SPI_GETCLIENTAREAANIMATION`; off means reduced motion). After the host window subclass receives `WM_SETTINGCHANGE` (`SPI_SETCLIENTAREAANIMATION`), it rereads at the next `about_to_wait`. The event is sent only when the value changes. No polling.
- macOS: reads "Reduce motion" (`NSWorkspace.accessibilityDisplayShouldReduceMotion`). On first read, a process-level observer is registered with the workspace notification center (`NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification`). Notifications arrive on the main run loop, and the host rereads at the next `about_to_wait`. The event is sent only when the value changes.
- Linux: not reported yet. Always `false`. No events are sent.

### System high contrast

`RuntimeProgramContext::high_contrast()` returns whether system high contrast is on. It is read when the window is created. When the user toggles it at runtime, the host sends `WindowEvent::HighContrastChanged { id, high_contrast }` to every window, and later callback contexts read the new value. The native host calls `AppContext::set_high_contrast` on the window document at window initialization and on every toggle, and the installed theme then draws with the high-contrast palette of the same lightness (see [Look](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/look.md)). This is a presentation-layer override: the installed theme is not rewritten, and it comes back unchanged when turned off. Embedded hosts that own their own `AppContext` must call it themselves.

- Windows: reads `HCF_HIGHCONTRASTON` from `SPI_GETHIGHCONTRAST`. After the host window subclass receives `WM_SETTINGCHANGE` (`SPI_SETHIGHCONTRAST`), it rereads at the next `about_to_wait`. The event is sent only when the value changes. This is not DirectWrite text gamma.
- macOS: reads "Increase contrast" (`NSWorkspace.accessibilityDisplayShouldIncreaseContrast`). Changes go through the same display options notification as reduced motion. The event is sent only when the value changes.
- Linux: not reported yet. Always `false`. No events are sent.
- Tests use `nana_window::force_high_contrast(Some(true))` to force the flag; `None` returns to reading the platform.

### Reduced transparency

`RuntimeProgramContext::reduced_transparency()` returns whether the system asks for reduced transparency. It is read when the window is created. When it is toggled at runtime, the host sends `WindowEvent::ReducedTransparencyChanged { id, reduced }` to every window. NanaUI does not change materials for the application: the application uses it to switch to opaque surfaces (for example by not requesting a `MaterialEffect`). `nana_window::system_reduced_transparency()` / `take_reduced_transparency_change()` are the platform interfaces the host reads.

- macOS: reads "Reduce transparency" (`NSWorkspace.accessibilityDisplayShouldReduceTransparency`). Changes go through the same display options notification.
- Windows and Linux: not reported (Windows "Transparency effects" is a personalization setting, not the accessibility preference meant here). Always `false`. No events are sent.

### System text size

`RuntimeProgramContext::text_scale()` returns the system text size factor (`1.0` is the default size). It is read when the window is created. When it changes at runtime, the host sends `WindowEvent::TextScaleChanged { id, scale }` to every window. NanaUI does not apply it automatically: an application that offers "follow system" UI scaling reads it and passes it to `WindowHandle::set_content_zoom` ([Whole-window zoom](#whole-window-zoom)) or its own text styles. The platform interfaces are `nana_window::system_text_scale()` / `take_text_scale_change()` / `set_text_scale_waker(..)`.

- Windows: reads "Text size" (`UISettings.TextScaleFactor`, 1.0–2.25). The process holds one `UISettings` subscribed to `TextScaleFactorChanged`. The event fires on a worker thread; the waker installed by the host wakes the event loop, and it rereads at the next `about_to_wait`. The event is sent only when the value changes.
- macOS: there is no system-level text size (Dynamic Type is iOS only). Returns `None`, treated as `1.0`. No events are sent.
- Linux: there is no portable system text size. Returns `None`. No events are sent.

### Displays and fullscreen

`WindowService::displays()` (embedded hosts use `EmbeddedRuntime::displays(event_loop)`) enumerates the currently connected displays on the window thread. It returns `DisplayInfo`: id, name, physical position/size, scale, refresh rate, and whether it is the primary display. Wayland does not report the primary display. `DisplayInfo::logical_bounds()` converts the physical rectangle into the global logical coordinates used by `WindowDescriptor::initial_position`. It returns `None` when there is no position or size.

`DisplayId` is a display identity within this process. It is equal across enumerations. Whether it still refers to the same screen after unplugging and replugging depends on the platform:

| Platform | Unplug and replug |
| --- | --- |
| macOS | `MonitorHandle::id()` is stable across reconnects |
| Windows / Linux | Not stable across reconnects; treated as a new display after reconnecting |

There are no display topology events: winit provides no connect/disconnect notifications. After a display is unplugged, the next `displays()` no longer lists it. Windows already fullscreen on that display land on a remaining display as the platform decides. Specifying a display that no longer exists: an explicit `set_fullscreen(Some(FullscreenRequest { display: Some(id), .. }))` returns `WindowError::InvalidParameter`, and the window state does not change. `WindowDescriptor::fullscreen` on the descriptor falls back to windowed mode instead (best effort; creation itself still succeeds).

Fullscreen is only borderless, covering the current video mode. There is no exclusive fullscreen (it does not change the display resolution, so it does not fight other Surfaces or capture tools over the display mode).

```rust
#[derive(Default)]
pub struct FullscreenRequest {
    pub mode: FullscreenMode,       // Borderless (default) or Simple
    pub display: Option<DisplayId>, // None = the display the window is currently on
}

pub enum FullscreenMode {
    Borderless, // Borderless, covering the display's current video mode
    Simple,     // macOS does not switch Spaces; Windows fills the work area with the taskbar staying usable on top; other platforms same as Borderless
}
```

`WindowHandle::set_fullscreen(None)` exits fullscreen. `Simple` is reported on macOS and Windows as `WindowModeState::fullscreen = Some(Simple)`. Windows `Simple` does not go through winit fullscreen: it fills the work area of the display the window is on (or the specified one), leaving 1 pixel on the edge of an auto-hidden taskbar so it can slide out; a window that fills the whole display is treated by the system as fullscreen, the taskbar sinks below it, and a transparent window would even show the taskbar through while eating its clicks. It refills when the taskbar moves or the resolution or scale changes, and on exit restores the position from before entering (including maximized). When switching from macOS native fullscreen to Simple, the host first exits native fullscreen, and applies Simple only after `fullscreen()` reads `None`.

The effective state is reported through `WindowEvent::ModeChanged { id, mode }`. `mode` contains the actual fullscreen mode, window level, and current display. Timing:

- Always sent once after each window's `Ready`, as the initial state
- After every fullscreen or level request
- After `Resized` / `Moved` / `ScaleFactorChanged` / `Occluded` / `RedrawRequested`, if the observed result changed

It is sent only when it differs from the last delivery. The level is the value the host last applied: the platform does not report topmost changes made outside the application. During the macOS native fullscreen animation, the target state is read. Under X11, fullscreen/topmost changes made by the window manager itself cannot be observed. `RedrawRequested` and `Occluded` are used to complete observation after asynchronous Borderless completion.

Descriptor fullscreen is applied only after the window becomes visible (the window is first created hidden and finishes document initialization). Do not use the descriptor to express "this display must exist". That is the contract of the explicit request.

`window-service-lifecycle` / `embedded-window-lifecycle` cover non-empty enumeration, receiving `ModeChanged` after fullscreen on a specified display, topmost, the platform result of `set_skip_taskbar` (success on Windows, `Unsupported` on other platforms), exiting fullscreen, an invalid `DisplayId`, and descriptor fullscreen. The macOS green button, a second display, unplugging a display, and Windows multi-display need manual checks.

### Content aspect ratio

When the whole window shows a single picture (video player, image viewer), use `WindowDescriptor::content_aspect_ratio(Some(width / height))` to keep the client area at that ratio. To change the ratio at runtime (the next video), send `WindowCommand::SetContentAspectRatio { id, ratio }` or call `WindowHandle::set_content_aspect_ratio(ratio)`; `None` removes it. A ratio that is not a finite positive number is treated as `None`; an invalid value in the descriptor makes creation fail (`InvalidParameter`). Do not send `SetBounds` after `Resized` to correct the size anymore: that path always gets pulled back when dragging the top or bottom edge, and it also fights with maximize and fullscreen.

```rust
let player = WindowDescriptor::new("Player")
    .initial_size(960.0, 540.0)
    .minimum_size(320.0, 320.0)
    .content_aspect_ratio(Some(16.0 / 9.0));
// After switching clips
commands.push(WindowCommand::SetContentAspectRatio { id, ratio: Some(9.0 / 16.0) });
```

Rules:

- **The ratio constrains the client area only.** Non-client borders and the title bar are not part of the calculation.
- **The minimum size** becomes `aspect_minimum_size(minimum_size, ratio)`: the smallest size that keeps the ratio and is at least `minimum_size` on both edges. One edge always stays at the minimum and the other grows, so a landscape minimum (say 640×360) makes portrait content at least 1138 tall. When the content may be portrait, give a minimum that both orientations can accept; a square is best. `WindowHandle::set_min_size` at runtime follows the same rule.
- **The size on opening and on a ratio change** is `aspect_window_size`: keep the current area (or the descriptor's, or the restored one), shrink it until it fits the display the window is on, then clamp it to the minimum above (the minimum wins, matching the platform minimum). Keeping the area makes switching back and forth return to the same size. The window's top-left corner does not move.
- **User border drags**: dragging the top or bottom edge follows the pointer in height and the width follows; dragging the left or right edge follows in width and the height follows; dragging a corner follows whichever axis has changed more since the gesture began. The edge opposite the dragged one stays put; a vertical-only drag keeps the left edge, a horizontal-only drag keeps the top edge. The system border (`WM_SIZING`) and the NanaUI client-area border (`LiveFrameResize`) share one pure function.
- **Maximized and fullscreen** windows are unconstrained and use the platform size. A ratio change made while maximized or fullscreen is applied at the first size change after the window returns to normal.
- **Platforms**: keeping the ratio while dragging is implemented on Windows only. On macOS and Linux the opening size, the adjustment on a ratio change and the minimum size still apply; user drags are unconstrained.

For a window that only shows video, leave chrome visibility to the media bar's `auto_hide(true)`: it collapses when the pointer is idle or has left the window (including in fullscreen), and the title bar follows the bar's `OverlayVisibilityChanged`. Write `AppTitleBar::over_media(true)` on the title bar rather than only `transparent(true)`: dark foreground in the light theme is unreadable over a picture. Do not track "is the pointer inside" yourself with `PointerPresenceChanged` to toggle chrome — in fullscreen the chrome would then never collapse. See [MediaTransportBar](https://github.com/sena-nana/NanaUI/blob/main/docs/components/media-transport-bar.md#自动收起).

### Changing size limits at runtime

To change the minimum / maximum client size after the window is open (for example when a zoomed UI needs a larger minimum width) use `WindowCommand::SetSizeLimits { id, minimum, maximum }` or `WindowHandle::set_size_limits(minimum, maximum)`; to change one of them use `WindowHandle::set_min_size` / `set_max_size`. Units are the display's logical units and do not follow [whole-window zoom](#whole-window-zoom).

- `None` removes that limit (the minimum falls back to the platform's own lower bound). A maximum below the minimum is raised to the minimum: the minimum wins, consistent with each platform.
- An ordinary window whose current size falls outside the new limits is adjusted immediately (`size_within_limits`, each edge clamped on its own, top-left corner unchanged); the platform itself only applies new limits on the next user drag. A maximized or fullscreen window keeps the platform size and is constrained by the platform after returning to normal.
- The new minimum is written back into the window descriptor, so a content-aspect-ratio lock still derives its own minimum from it through `aspect_minimum_size`.
- The command has no reply: if a size is not a finite positive number the whole command is ignored; `WindowHandle` requests return `InvalidParameter`.
- Platforms: macOS (`contentMinSize` / `contentMaxSize`), Windows (`WM_GETMINMAXINFO`), Linux X11 and Wayland all take effect through winit's `set_min_surface_size` / `set_max_surface_size`, and the host then requests a size once to clamp the current window; a Wayland compositor may refuse an application-initiated size change, in which case the limits apply at the next user resize. Mobile hosts (Android, iOS, OHOS) have system-decided window sizes and ignore the command.

### Whole-window zoom

The application's "interface scale" setting (including following the system text size) uses `WindowHandle::set_content_zoom(zoom)` or `WindowCommand::SetContentZoom { id, zoom }`, where `1.0` means no zoom; the value is clamped to `CONTENT_ZOOM_RANGE` (0.25–5.0) and a non-finite or non-positive number is treated as `1.0`. Zoom multiplies the display's `scale_factor` and applies to everything this window draws: fixed pixel widths, icons, borders, radii, the current value inside a dropdown, text layout and grids, hit testing, focus rings, accessibility-tree bounds and IME candidate placement all grow together, so the application does not restyle anything. The window is not recreated and its size does not change; the next frame lays the document out again in a smaller logical viewport and draws it at a larger scale. Read the current value with `RuntimeProgramContext::content_zoom()`.

```rust
// A settings page's "interface scale 125%", or following the system:
let zoom = cx.text_scale().unwrap_or(1.0);
let _ = cx.window().set_content_zoom(zoom);
```

Rules:

- **Content geometry is `WindowGeometry::content(zoom)`**: the physical size is unchanged, `scale_factor` is multiplied by `zoom` and `logical_size` is divided by it. The layout viewport, the paint viewport (including `WindowOutput`), pointer and drag-and-drop coordinates, accessibility, IME, native web views, the placement of native macOS window buttons and the scaled hit areas of self-drawn borders all use it, so scene coordinates times `content_geometry().scale_factor` are surface pixels. `RuntimeProgramContext::content_geometry()` gives applications the same value.
- **Window management does not zoom**: `RuntimeProgramContext::geometry()`, `WindowEvent::Resized`, minimum / maximum sizes, `SetBounds` and geometry persistence all stay in the display's logical units. Zoom changes neither the window size nor the persisted geometry.
- **GPU content**: the layout box of a `CustomRenderNode` or `HostTexture` is in logical units and grows with the zoom. A texture size declared in texture pixels does not change: the texture is sampled at its own size into the larger box. A renderer that picks its render-target size from `scale_factor` (the `GpuView` context) receives the zoomed value, so it draws sharper instead of being stretched. Native content areas (Windows composition) are handed scene coordinates and convert with `content_geometry().scale_factor`.
- **Embedded hosts and headless**: `HeadlessRuntime` records the command and `present` lays out in the `content(zoom)` viewport. Embedded hosts that own their `AppContext` compute the viewport and paint scale themselves.

### Embedding into an existing host

`platform_host::EmbeddedRuntime` takes the host's `ActiveEventLoop`, its proxy and `HostedGpuShared`. It never creates or exits the host event loop. The host forwards `window_event`, `wake` and `about_to_wait`. `HostedGpuShared::from_device` plugs into an existing Instance/Adapter/Device/Queue. It never asks for a second Device.

From a window-thread callback a host can create a window synchronously with `create_window(event_loop, descriptor)`. The result is the same as `WindowService::create_window`, but it does not go through the request queue. The process has exactly one event loop: calling `run_runtime` again in standalone mode returns `HostedRunError::EventLoop` (winit's `RecreationAttempt`). A host that already owns an event loop should switch to `EmbeddedRuntime`.

Device recovery stays the embedded host's job: after the host receives an external device-loss notification it first calls `notify_device_lost()` on the window thread, then forwards the other window events. It checks the pending state with `needs_gpu_replacement()`. After rebuilding the host device it calls `replace_gpu()`. Replacement always switches to the new Device: every live window's Surface is rebound in place, and the old swap chain is released before the new one is configured, because DXGI allows one swap chain per HWND, a new one cannot be tried while keeping the old Surface as a fallback, and after device loss the old Surface could not present anyway. After the switch the application's GPU-rebuild callback runs. Windows whose rebind failed are retried individually per Surface failure recovery below. Only when every window fails to rebind does the call return the first error for the host to report — but the switch has already happened.

`WindowHandle::with_native_handle` schedules a callback on the window thread. The raw handle is only valid for the duration of the callback; do not keep the pointer past it. It does not take ownership of `winit::Window` either.

`WindowHandle::pointer_position()` asks where the pointer is over this window right now, in logical client pixels. It is `None` while the pointer is outside the window or the platform cannot sample it (Wayland). It samples the system, the same way Forward passthrough does, and does not depend on the pointer events this window receives, so it also answers during whole-window or Forward passthrough: a click-through window uses it to tell whether the pointer is over its own content. It does not account for another window on top. An expired window generation returns `WindowClosed`. `HeadlessRuntime` answers the position of the last injected pointer input, or what `set_pointer` put.

`WindowHandle::surface_info()` explicitly queries the `WindowSurfaceInfo` the window thread holds: the configured physical size, format, color space, present mode, alpha, maximum frame latency, device generation and the current display. The query does not acquire, reconfigure, submit or wait on the GPU, and ordinary frames do not poll it. An expired window generation returns `WindowClosed`; a Surface that is pending recovery or reconfiguration returns an error instead of reporting requested values as applied. Automatic present mode keeps its automatic identity, so the backend's final mode and the display's actual scan cadence cannot be inferred from it. `presentation-output-probe --surface-info` requests it once after the first direct present and during warmup; a missing record, an error, or a resize/move/mode change between the query and the end of the direct measurement withholds that observation. Diagnostics startup comparisons can add `--presented-counter` so the text counter increments only after a successful present, keeping prepare-only retries from changing the content of later measurement frames; the default fixture keeps counting prepares. This is a different diagnostic population and cannot be mixed directly with the default latency.

`window.effects().set_material()` returns the actual `MaterialOutcome`. Click-through is controlled with `set_mouse_passthrough()`. `window.capture().set_protected()` requests macOS/Windows native capture protection; other backends return `Unsupported`. This is not a guarantee against every capture method.

Full examples: `window-service-lifecycle` needs no winit import. `embedded-window-lifecycle` shows advanced host adaptation. Both automatically verify three real windows presenting, two attached window documents built by `tag`, cross-thread control, the main window closing, failure rollback, child-window release and re-creation.

### Migrating from the older API

- `RuntimeWindowSettings` / `WindowSettings` are unified into `WindowDescriptor`. Explicit struct initialization needs a `visible` field or the defaults.
- `WindowDescriptor` gains `tag`. Explicit struct initialization needs `tag: None` or `..Default::default()`.
- `WindowDescriptor` gains `restoration_scope`. Explicit struct initialization needs `RestorationPath::root()`. Multiple profiles / workspaces / windows should pass their own stable scope path, or use `..Default::default()`.
- Ordinary applications use `WindowService` / `WindowHandle` instead of allocating window ids and pushing `WindowCommand` themselves.
- `WindowCommand` moved from the platform crate root into `nana_ui_platform::host`. It is only for framework adapters such as Dock and chrome; the batches they submit still enter the same WindowManager when the host commits.
- The main window no longer has an implicit exit privilege. A product that wants "closing the main window exits" should return an exit update explicitly.
- `WindowCommand::SetFullscreen` takes `Option<FullscreenRequest>` instead of `bool`: `None` leaves fullscreen, `Some` enters it. `SetSimpleFullscreen` is removed; the macOS fullscreen that keeps the Space becomes `FullscreenRequest { mode: FullscreenMode::Simple, display: None }`. `WindowHandle::set_simple_fullscreen` is removed with it.
- `WindowLevel` moved down from `nana_ui::window_service` to `nana_ui_platform`, and `nana_ui` re-exports it. Effective fullscreen and always-on-top values arrive through `WindowEvent::ModeChanged`; do not keep your own mirror of the request.
- LiliaBilibili's `app/presentation.rs` still compiles against `SetFullscreen { fullscreen: bool }`. On the next pin upgrade change it to `fullscreen: on.then(FullscreenRequest::default)` and handle `ModeChanged` in the `WindowEvent` match.

Window geometry can be left to the framework: `WindowDescriptor::persist_key("main")` restores position / size / maximized from a versioned `ViewStateStore` before creation and merges changes back asynchronously after moves and resizes. Fullscreen, minimized and the `-32000` origin that Win32 minimized windows commonly report are not written. The host chooses the directory: `app_data_dir("YourApp")` gives the platform data directory, then `FileStore::open` feeds `run_runtime_with_store` (landing as `local-storage.bin`); without it everything stays in memory and is lost when the process exits. Dock layout goes into `ViewStateStore`; Appearance goes into its own Settings namespace. The old `nana.dock.*` and `nana.appearance.*` are migrated once.

### Standalone transparent tool windows

`WindowDescriptor::focus_on_show = false` makes showing the window not steal foreground focus, for both the first show and a later `set_visible(true)`. On macOS it shows with `orderFront` without becoming the key window (winit's show calls `makeKeyAndOrderFront`). Windows and other platforms still go through winit's show path. The default `true` keeps the old behavior. A tool layer can combine `transparent = true`, `always_on_top = true` and the non-modal `WindowRole::Tool`. A tool window belonging to a main window sets `parent`; on Windows it is owned by the parent and clicking the parent does not push the tool window behind. `DesktopShell` is not needed to use edge resizing.

`WindowService::create_window` completes its handle after the window is fully ready and `WindowEvent::Ready` has been sent. A creation failure returns the error through the handle. If the application closes the window while `Ready` is being handled, the handle returns `WindowClosed`. The service allocates window ids from `1 << 63` up and skips program-chosen ids that are still live (such as the hash ids of Dock floating windows). Host batch adapters are notified of failures separately through `OpenFailed { id, error }`, which rolls back the in-creation state. `SetMousePassthrough { id, enabled }` turns off native hit testing for the whole window (Windows `WS_EX_TRANSPARENT`); the overlay then receives no pointer and cannot withdraw itself by hit. `SetMousePassthroughForward { id, enabled }` is the host-held Forward mode: OS passthrough stays on, the host samples the global pointer on the window thread (Windows `GetCursorPos`, macOS `mouseLocationOutsideOfEventStream`, Linux X11 `XQueryPointer`; Wayland cannot sample it yet, so it cannot withdraw itself automatically), converts to overlay logical coordinates and goes through the existing `pointer_target`. When it hits opaque or interactive content whose `pointer-events` is not `none` it calls `set_cursor_hittest(true)` to take the window back, and passes through again after leaving that area or the window. A sampled Move can still feed overlay hover and cursors. A press is only received by this window after the window has taken itself back. The window underneath is clickable while passing through. Controls never see a HWND. `WindowHandle::set_mouse_passthrough_mode` maps to `MousePassthroughMode::{Off, Passthrough, Forward}`. Every hit-test change reports `MousePassthroughChanged { id, enabled, result }` (`enabled` is whether OS passthrough is currently on; unknown windows report a failure too). The application shows a locked state only after the success confirmation, and keeps the other window's passthrough-release entry point. Shaped windows / per-pixel alpha OS shapes are not part of this contract.

`RuntimeProgram::window_material_mode_for(id)` and `appearance_backdrop_opacity_for(id)` default to the existing global methods. They let the main window and transparent tool windows be configured separately; the host calls them for the target window on creation, on appearance changes and on Surface recovery. A transparent background does not change the foreground text opacity; the content background of a fully transparent window is drawn by the application's Runtime nodes.

`WindowDescriptor::constrain_to_work_area = true` is for restoring a tool window completely: a position that is partly off-screen is corrected and a size larger than the screen is shrunk. Windows uses the native work area with the taskbar removed; other platforms currently fall back to the display bounds. The default `false` keeps the old "keep the position if it intersects any screen" behavior.

`WindowDescriptor::skip_taskbar = true` keeps the window out of the taskbar; the default is `false` and the main window still shows. At runtime toggle it with `WindowCommand::SetSkipTaskbar { id, skip_taskbar }` or `WindowHandle::set_skip_taskbar`. The descriptor's setting is applied before the window first shows and is reported after `Ready`; a failure does not roll back creation. Every request (including the descriptor application and unknown windows) reports `WindowEvent::SkipTaskbarChanged { id, skip_taskbar, result }`: `skip_taskbar` is the current actual state and keeps its previous value on failure. An explicit request that arrives before the descriptor result supersedes it. `WindowHandle` requests return the same result. Platforms that cannot do it report `Unsupported`; a system refusal is `OperationFailed`.

- Windows: the host gives the window a hidden owner window of its own and clears `WS_EX_APPWINDOW` (a subclass guard blocks the bit winit writes back on every style change). The system never gives an owned window without that bit a taskbar button, on every show and after an Explorer restart alike, so nothing races the system's button creation. It does not use `ITaskbarList::DeleteTab`: Windows Graphics Capture's `CreateForWindow` returns `E_INVALIDARG` for a window whose button was deleted, and window capture in OBS and similar tools gets no picture. There is one owner per window (windows sharing an owner move together in the z-order); it is never shown, sits at the bottom of the z-order and is destroyed after its window closes. It is an ordinary popup, not a `WS_EX_TOOLWINDOW`: capture also refuses a window whose owner is a tool window. A window that already has an owner (an auxiliary window, a modal child) keeps it. A shown window is hidden and shown again without activation: a shown window given an owner loses its button, but capture accepts it only after it is shown again. Every failure is reported, and a repeated request changes nothing. `skip_taskbar: false` removes the owner and restores the style bit, and a shown window gets its button back at once through `ITaskbarList::AddTab` (a button added back does not affect capture). It only restores buttons hidden by this contract and never adds a button to a window that has none (a modal child window, for example). It does not change `WS_EX_TOOLWINDOW` or the title bar style; Alt+Tab lists the window by the system's rules for owned windows.
- macOS: returns `Unsupported`. The Dock shows an icon per application; ordinary windows have no per-window Dock entry, though a minimized window still goes to the Dock and the application icon is always there. Hiding the application's Dock icon is an application-level activation policy, not part of this per-window contract.
- Linux: returns `Unsupported`.

Native acceptance probe (it briefly moves the pointer onto the probe's own windows):

```powershell
cargo build -p nana-ui --example desktop-overlay-probe --features hosted,bundled-fonts --locked
python -m pip install -r scripts/requirements-desktop-overlay.txt
python scripts/validate-desktop-overlay.py
```

The probe verifies Solid/Opaque on the main window against Transparent/PreMultiplied on the tool window, the first show not stealing focus, the tool window staying out of the taskbar (comparing UI Automation taskbar-button snapshots, independent of window titles, button grouping settings and system language, needing `comtypes`; both toggling `SetSkipTaskbar` and hiding then showing again are re-checked), creation-failure feedback, passthrough-toggle feedback, real pointer routing 1→0→1, Forward taking the window back over opaque hit areas and receiving the following click, and the screen pixels underneath matching after the transparent area closes. Results are written to `target/desktop-overlay-native.json`. It does not replace visual acceptance of a specific product's layout. Windows is the mandatory platform for the issue; macOS covers behavior tests and the documentation.

Presentation cadence is accepted separately. It does not move the pointer and does not need `comtypes`:

```powershell
python scripts/validate-desktop-overlay.py --presents
```

The probe starts a synthetic process with `--presents` (`GpuBackendPolicy::CompositionCapable`) and opens three windows with `FrameDemand::Continuous(30)`: a click-through composition-transparent window, an idle composition-transparent window with no input at all, and a click-through ordinary window. `PASSTHROUGH_PRESENTS` asserts that the passthrough window presents on cadence through three rounds of passthrough on, off and on again (at least 36 frames in 2 seconds) and is still visible in screen pixels. `COMPOSITION_CONTINUOUS_PRESENTS` asserts that the idle composition window keeps presenting without any `InvalidateRect`. Results are written to `target/desktop-overlay-presents.json`.

Two host behaviors support both:

- When winit turns hit testing off it adds `WS_EX_LAYERED | WS_EX_TRANSPARENT` without setting the layer attributes. A layered window without attributes is not composited by DWM, so the host calls `nana_window::settle_layered_window` after every style write to add `LWA_ALPHA` 255; an existing color key or alpha is left alone.
- A due frame is serviced by the `WM_PAINT` the host requested. Its deadline has already passed, so the host waits one frame period as well. If no paint arrives during that period, the host on Windows presents the frame directly in the event loop (counted as `host.frames_served_without_paint`) and other platforms request again. Applications do not need their own `InvalidateRect` / `UpdateWindow` thread.

**Validation status**: on 2026-10-08 on real Windows 11 hardware (RTX 5060 Ti, DX12), `--presents` ran three rounds of 60 frames per window in 2 seconds, with the marker pixels of all three windows on screen. The full `validate-desktop-overlay.py` (pointer routing, taskbar) was not run this time because that machine has no `comtypes`.

Hidden, minimized or occluded windows can still present: the program asks for a window output with `RuntimeProgram::window_output` and sets `while_hidden`. The host flushes the document in a GPU-only hidden tick and draws only the output. The contract and the real-hardware results are in [Window-independent presentation](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/output.md#window-outputs).

## Native web content in the Runtime

`runtime::BrowserView` is a layout and accessibility node in the retained tree. Application toolbars, address input and domain state are still carried by the Runtime. With `hosted` enabled, `RuntimeProgram::native_browser_requests` returns `NativeBrowserRequest { id, node, policy, restore_url, visible, revision, command }`. The host creates native content only for a `BrowserView` in the current window's document whose `browser_id` matches the request's `id`. A duplicate `id` in the same window, a node from another document and a destroyed node all leave no native view attached.

`revision` is the application's monotonically increasing command identity. Re-projecting the same revision does not navigate back, refresh or capture again; `command: None` on an existing instance only syncs state. Events actively wake the current host through the platform callback and flow back through `native_browser_event(window, NativeBrowserEvent { id, node, revision, event }, context)`. The event keeps the command revision that produced it. The host again checks the window, the current request and the node, so a late result is never re-labelled as a later request. A capture returns PNG through `Captured(Vec<u8>)` and failure through `CaptureFailed`; navigating or hiding during the capture invalidates it.

The current macOS backend uses a main-thread `WKWebView` subview bound to the parent window and supports HTTP(S) navigation, back/forward, refresh/stop, focus and asynchronous capture. `BrowserPolicy::allow_web` is off by default; `about:blank` is always allowed. Once enabled it still rejects file, script and credential-bearing URLs; redirects and new-window links go through the same policy, and new-window links stay in the same browsing content. Windows/Linux currently return an explicit not-available state instead of creating a placeholder browser. That boundary does not affect their existing window, file dialog and NativeContent capabilities.

Hiding a mounted node keeps the browsing history: the native view hides and releases focus. Parking or destroying the node, withdrawing the request, changing the node or policy, or closing the parent window removes the native view, and late callbacks are dropped. Re-mounting creates a new instance that only navigates to the target of the current Navigate command or to `restore_url`; back, stop and captures are not replayed. After a page is restored the native history starts from the new instance. The application should wait for the flowing-back state before updating its buttons, and must withdraw its own capture intent when the business task switches or closes — it cannot just compare the current task at completion time (switching away and back is a new intent).

A native subview supports only limited translation and rectangular clipping. Layout uses window logical coordinates and follows scrolling. Rounded clipping, non-translation transforms, opacity/filter groups, or overlapping Runtime content drawn afterwards temporarily hide the page so it does not cover menus and overlays; hiding does not shrink the page's own layout size, and repeated frames do not re-set identical native geometry. Native web content does not appear in Runtime offscreen captures; user captures must use `BrowserCommand::Capture`.

Run `cargo run -p nana-ui --example native-browser --features hosted,bundled-fonts`. Its example page, description page, back, hide and capture buttons verify a real parent window. Setting `NANA_BROWSER_CAPTURE_OUTPUT=/tmp/nanaui-browser.png` saves the page PNG. Platform visuals and interaction may be claimed as passing only after actually looking at the native window and the PNG; compiling and offscreen tree checks do not replace WebKit acceptance.

### Headless web surfaces

`RuntimeProgram::web_surface_requests` declares web instances attached to no window: frames go straight to the application through `WebFrameSink` and events flow back through `web_surface_event`. The interactive window that `ShowWindow` opens is owned by the host platform layer, is not a Runtime window and does not go into `WindowService`. The contract is in the [GPU reference](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/gpu.md#无头网页画面websurface).

### Surface failure recovery

A failed Surface creation, validation or material alpha configuration only pauses the affected window and retries on the existing shared GPU every two seconds; other windows keep drawing. `HostFailure::SurfaceRecovery` reports the window identity and the first error. After a successful recovery that window's material cache is reset and it repaints. A failure does not close the application document. During recovery material operations return `OperationFailed`, and a failed material operation does not store a new override value or transparency preference. If the native configuration has already partly changed, a later Surface recovery reapplies the last successful settings. Only device loss starts global GPU recovery; Surface failures caused by device loss do not report `SurfaceRecovery` per window. Standalone recovery tries the surviving windows in turn as the base for the new Device, and one window that cannot be rebuilt does not block the others. An embedded partial Surface failure does not ask the host to replace the Device.

Window dragging, edge resizing and passthrough control keep the underlying error classification: `Unsupported` when the platform explicitly cannot do it, `OperationFailed` when the system ignores the operation or another OS error occurs. Passthrough events and request completions report the result of the same operation.

When a parent window already has a modal child window, creating another modal child fails; nested modals can be created inside the existing one. A focus request for the parent is forwarded along the modal chain to the deepest active child window. Closing the parent recursively tears down the whole child chain, and the closing Windows parent is never re-enabled or re-focused during that teardown.

`embedded-window-lifecycle --probe-device-loss` has the host destroy the external Device before the first frame, notify the manager to suspend and replace the GPU, and then run the same multi-window and focus acceptance. It verifies the host notification/replacement protocol and presenting after recovery; it is not performance data for recovering mid-draw.

The sizes in a creation descriptor must be finite and positive, positions must be finite, and a modal window must name a parent. `WindowService::create_window` completes these errors directly as `InvalidParameter` without enqueuing or waking the host. Standalone/embedded initial windows are validated the same way before native resources are allocated, and must not name a parent; a window with a parent is created through the service.

When the host destroys `EmbeddedRuntime` directly it first closes the request queue and completes waiting requests, then destroys application state, so application teardown may wait for a worker thread that is waiting on a window request. Unprocessed and later requests all get `HostStopped`. `embedded-window-lifecycle --probe-host-stop` covers this exit order.
