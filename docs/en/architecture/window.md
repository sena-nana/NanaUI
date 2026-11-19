# Native Windows, Title Bars and System Materials

NanaUI targets native desktop software: title bars, system frosted-glass materials and high-DPI scaling follow desktop operating system semantics, not browser rules.

Your application owns the `Window` and the `Surface`. Calling `run_runtime(WindowDescriptor::new("Title"))` creates the main window, initializes the process's single `GpuContext` and starts the event loop. Later auxiliary windows share the same `GpuContext` and component registry, but each window holds its own Surface, input endpoint, IME session, document and render target. Raw window handles (`HWND` / `NSWindow`) are strictly sealed inside `nana-window`; ordinary controls never get a handle. For the complete technical contract, see the [Window reference](../reference/window.md).

## Custom title bar and client-area chrome

By default a window uses a custom title bar (`AppTitleBar`, standard height `36 px`), made of a leading slot (`leading`), a center title (`center`), a trailing business slot (`trailing`) and window control buttons (`controls`):

- **Dragging and button hits**: pressing on empty title bar space drags the window, while buttons and interactive controls in the bar capture the pointer first and are not dragged by mistake (note: CSS `app-region: drag` is not NanaUI's drag contract; writing `drag` on an ordinary box does not turn it into a title bar). Set `.transparent(true)` to remove only the title bar background while keeping its content and drag hit area. Set `.drag_enabled(false)` (for example in fullscreen) to disable dragging.
- **Window control semantics**: the minimize, maximize and close buttons only emit typed `WindowChromeAction` requests, which the host that owns the window carries out. Clicking the close button is exactly the same as a system-level close: it only dispatches `WindowEvent::CloseRequested` to the program, and the application decides whether to close immediately, confirm in a dialog, or minimize to the tray.
- **System title bar and edge resizing**: if you do not use the custom title bar, set `WindowDescriptor::system_caption(true)` to keep the system's native title bar and close button (window resizing is then left to the operating system, and the framework does not add a second set of edge hit areas). With custom frameless chrome, when the window is resizable, not maximized and not fullscreen, the outermost `8 px` edge of the client area handles window resizing automatically.
- **Coordinates and IME**: pointers, dragging and window sizes use **logical pixels** throughout the API; physical pixels are only used to configure the GPU Surface (it is reconfigured only when the physical size or presentation policy changes). When focus enters an editable field the window opens one IME session (caret movement or context changes within the same field are incremental updates and do not reopen the session; the candidate window is always anchored at the text caret), and it closes automatically on blur.

## System materials and fallback semantics

The application requests a system background material through `RuntimeProgram::window_material_mode` (or per window through `window_material_mode_for`). The framework strictly separates **the material the business requested** from **the material actually applied to the window**. When a request fails it immediately falls back to a readable opaque solid theme background and returns an explicit reason; it never silently substitutes another material family:

| Platform | Requestable system materials | Result when unavailable or failed |
| --- | --- | --- |
| macOS 10.10+ | The specified `Vibrancy` / `UnderWindowBackground` | Opaque solid theme background (with `MaterialFallback`) |
| Windows 11 | The specified `Mica` or `Acrylic` | Opaque solid theme background (with `MaterialFallback`) |
| Windows 10 1809+ | The specified `Acrylic` | Opaque solid theme background (with `MaterialFallback`) |
| Linux | No system-level background blur | Opaque solid theme background (`PlatformDoesNotProvideNativeMaterial`) |

Keep four boundaries in mind when using materials and shadows:

1. **Transparent (`Translucent`) is not system blur**: `Translucent` only enables a transparent compositing surface. If a transparent surface cannot be negotiated it falls back to an opaque solid color; it does not try Mica or Acrylic on its own.
2. **`WindowDescriptor::transparent` is a creation-time, lifetime contract**: if it is `true` when the window is created, that window can never switch back to an opaque solid color. If you want users to switch freely between "solid / transparent material" in settings, keep the descriptor's `transparent` at `false` and return the target material dynamically from `window_material_mode_for`.
3. **One presentation truth, `ResolvedWindowPresentation`**: the material, Surface alpha mode, client-area chrome and window shadow each window actually gets are settled together in `ResolvedWindowPresentation` (read through `RuntimeProgramContext::presentation()`), so rendering and the native frame always agree on "is it transparent right now".
4. **Native window shadow vs. in-frame `DropShadow`**: the window-level shadow is drawn by the platform window layer and can be toggled at runtime with `WindowCommand::SetShadow` (read the applied state from `presentation().shadow()`; it is `Pending` before the window is shown). If a transparent window already draws its root card shadow in its own framebuffer, set the window shadow to `WindowShadow::None` to avoid a doubled shadow.

## Multiple windows, companion windows and embedded hosts

The application gets the window service through `context.windows()`. The returned `WindowHandle` can be sent safely to worker threads (internally it queues commands back to the window thread):

```rust
// Create an auxiliary window from a worker thread; in an async context you can also .await directly
let window = service.create_window(WindowDescriptor {
    title: "Notes".into(),
    tag: Some("notes".into()),
    initial_size: (480.0, 320.0),
    minimum_size: (240.0, 120.0),
    system_caption: true,
    ..Default::default()
}).wait()?;
```

- **Use `tag` to identify a window's role**: do not guess a `WindowId` from window creation order. Set `tag` in the descriptor and read it with `window_tag()` when building the document. Creation of a new window completes only after the hidden native window, Surface, input endpoint and application document are all ready (if any step fails it rolls back automatically and no `Ready` is emitted). Window position and size can be persisted and restored automatically through `WindowDescriptor::persist_key` (fullscreen and minimized states are not written to the restore record).
- **Auxiliary parented windows (`WindowDescriptor::parent`) vs. `WindowRole::Underlay`**: top-level auxiliary windows use `WindowDescriptor::parent` for native ordering above their owner and host-managed level inheritance, separate from `Underlay`'s below-owner contract. `Underlay` sits directly below its parent window, shares the same client-area rectangle, follows the parent's movement and visibility, receives no input and never takes focus, yet keeps producing frames independently while covered (designed for recording software such as OBS; see [Window reference](../reference/window.md)).
- **Native screen eyedropper (`ColorSampler`)**: desktop color picking for controls such as `ColorField` goes through the platform boundary (macOS `NSColorSampler`, Linux Wayland XDG Desktop Portal v2+ without X11 fallback / X11 visual-mask decoding, Windows per-monitor DPI magnifier); unsupported hosts call `set_eyedropper_supported(false)` and log `window.color_sampler.failed`.
- **Embedded host (`EmbeddedRuntime`)**: when an external host already owns the event loop and the GPU device, use `EmbeddedRuntime` and inject the existing device through `HostedGpuShared::from_device`. To access the native window handle temporarily inside a callback, use `with_native_handle` (the borrow is valid only during the callback; never keep the raw pointer).

## Menu bars, system file dialogs and web content

1. **Menu bars (native menus and `AppMenuBar`)**: the menu model `MenuBar` is defined in `nana-ui-core`. On macOS the host can install it into the system menu bar at the top of the screen. On a custom title bar, or on platforms without native menus such as Linux, the [`AppMenuBar`](/components/app-menu-bar) component draws the same `MenuBar` inside the UI. Both paths share the `take_menu_activations()` activation queue, and the application only needs to dispatch business actions by menu item ID.
2. **System file dialogs**: because they need the parent window's native handle, file dialogs are always opened by the host (for example, after `PathField` emits a browse request, the result flows back asynchronously through `WindowEvent::FileDialogCompleted`; when the user cancels, the path list is empty rather than an error, and each window has at most one dialog open at a time).
3. **In-app web pages (`BrowserView`)**: acts as a layout and accessibility anchor on the retained tree, while the host mounts a native `WKWebView` child view on macOS (Windows and Linux return an explicit `Unsupported`). When it meets rounded corners, non-translation transforms, opacity/filter groups, or is covered by later Runtime content, the host temporarily hides the native child view so it does not cover overlays and menus.

## Read next

<div class="next-steps">
  <a class="next-step" href="/reference/window">
    <p class="next-step-link">Window reference contract</p>
    <p class="next-step-caption">The full specification for shadows, custom chrome, material negotiation, multiple windows and underlay windows.</p>
  </a>
  <a class="next-step" href="/architecture/">
    <p class="next-step-link">Architecture overview</p>
    <p class="next-step-caption">Back to the overall architecture diagram from the retained tree to the host Surface.</p>
  </a>
  <a class="next-step" href="/architecture/gpu">
    <p class="next-step-link">Live GPU content</p>
    <p class="next-step-caption">Mount live textures into the window tree on the same GPU device.</p>
  </a>
</div>
