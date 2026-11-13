---
title: "Window and Platform"
---

# Window and platform: neutral contracts, native boundaries and fallback semantics

This chapter completes the last stretch of the "who owns the window" chain: **what the platform-neutral window contract in `nana-ui-platform` looks like**; **how `nana-window` manages handles, system materials, frameless chrome, high-DPI scaling and fullscreen at the operating system's native layer**; **why material, composition and shadow failures always "report the actual applied outcome" instead of retrying silently**; **why ordinary controls never receive a native window handle**; and **what a headless `WebSurface` page actually is**. The chapter follows on from the Surface ownership model in [GPU and frame lifecycle](/learn/gpu-frame).

## Responsibilities and boundaries

- **`nana-ui-platform` (platform-neutral contract layer)**: defines the pure-data window contracts `WindowId`, `WindowDescriptor`, `WindowEvent`, `WindowCommand`, `DisplayInfo` (`crates/nana-ui-platform/src/lib.rs:42-49`), re-exports the canonical input contract of `nana-ui-input` unchanged (`:156`), and covers the cross-platform path/clipboard/network host interfaces. **No platform-specific handle crosses this boundary** (`window.rs:494`).
- **`nana-window` (platform-native execution layer)**: wraps raw window handles (`raw-window-handle`), Windows DWM / DirectComposition, macOS AppKit / Core Animation, Mica / Acrylic / Vibrancy system materials, custom-drawn frameless window edge resizing and style guards, the below-owner companion window (`Underlay`), the native startup splash (`NativeSplash`) and headless web capture (`WebSurface`) (`crates/nana-window/src/lib.rs:1`).
- **The iron rule of handle isolation**: the application/host owns `Window` and `Surface`; `nana-window` functions borrow the window only for the duration of a call through `impl HasWindowHandle`. Ordinary controls cannot obtain a native handle at compile time or at runtime; operations that need a parent window handle (installing a system menu bar, opening a system file dialog) are performed on their behalf by the host, and results flow back asynchronously as events.

## Dependencies and neighbors

In dependency order: `nana-ui-core`, `nana-ui-input` → `nana-ui-platform` → `nana-window` → `nana-ui`. `nana-window` depends on `nana-ui-platform` only for its `path::normalize_path` (file dialogs and "reveal in Finder / Explorer" normalize a path before handing it to the system). Neither `nana-ui-runtime` nor `nana-ui-scene` depends on it — the Runtime only depends on the canonical input contract of `nana-ui-input`, so ordinary controls cannot reach the window contract or native window APIs at compile time; only the host adapter `nana-ui` connects both sides:

| Neighbor crate | Collaboration |
| --- | --- |
| `nana-ui-input` | The canonical input contract is re-exported by `nana-ui-platform` (`lib.rs:156`); the host lowers native window events into it |
| `nana-ui` | `WindowService` / `WindowHandle` queue cross-thread window commands back to the window thread for execution; `ResolvedWindowPresentation` settles each window's final presentation state and `SurfacePresentationPolicy` color negotiation here |
| `nana-gpu` | Device, frame, and texture backend contract (see [GPU and frame lifecycle](/learn/gpu-frame)) |
| `winit` | `nana-window` fills desktop-level capabilities between native OS APIs and winit (Win32 frameless style guards, layered window composition settling, macOS show-without-activation and precise traffic-light placement) |

## Key types and data

| Type | File and line | Purpose |
| --- | --- | --- |
| `WindowId` | `crates/nana-ui-platform/src/window.rs:5` | Platform-neutral window ID (`pub u64`, `PRIMARY = 0`; the underlying platform window ID stays in a private mapping) |
| `WindowDescriptor` | `crates/nana-ui-platform/src/window.rs:429` | Window request form: title, initial/minimum size, position, transparency, shadow, role, `persist_key`, `tag` |
| `WindowEvent` | `crates/nana-ui-platform/src/window.rs:114` | Window result events: `Ready` / `Resized` / `ModeChanged` / `CloseRequested` / `FileDialogCompleted` |
| `WindowCommand` | `crates/nana-ui-platform/src/window.rs:684` | Window commands the host can execute: `Move` / `Resize` / `SetFullscreen` / `SetShadow` / `OpenFileDialog` |
| `WindowGeometry` | `crates/nana-ui-platform/src/window.rs:12` | Position/size in both physical and logical pixels, along with `scale_factor` and `maximized` state |
| `DisplayInfo` / `DisplayId` | `crates/nana-ui-platform/src/window.rs:214` / `:210` | Display enumeration info and a stable per-process identity across enumerations |
| `WindowSurfacePreference` | `crates/nana-ui-platform/src/window.rs:496` | Per-window presentation target preference: `Auto` / `NativeWindow` / `Composition` / `RequireComposition` |
| `MousePassthroughMode` | `crates/nana-ui-platform/src/window.rs:653` | Mouse passthrough mode: `Off` / `Passthrough` (whole window) / `Forward` (pass through transparent gaps, restore over controls) |
| `MaterialEffect` / `MaterialOutcome` | `crates/nana-window/src/material.rs:36` / `:121` | The system material request enum and the actual applied outcome (including an explicit `MaterialFallback` reason) |
| `NonClientRenderingStrategy` | `crates/nana-window/src/chrome.rs:71` | Windows frameless transparent window strategy: `StripFrameStyles` (default) / `SuppressNonClientRendering` |
| `ResolvedWindowPresentation` | `crates/nana-ui/src/presentation.rs:343` | The final settled presentation state of one window: effective material + alpha mode + presentation target + chrome + shadow |
| `WindowHandle` | `crates/nana-ui/src/window_service.rs:271` | A cloneable, sendable window control handle (internally queued back to the window thread over a bounded channel) |
| `Underlay` | `crates/nana-window/src/underlay.rs:42` | The `WindowRole::Underlay` below-owner companion native window binding |
| `WebSurface` / `WebFrame` | `crates/nana-window/src/web_surface.rs` | A headless web engine instance and the RGBA8 frames it captures in the background |
| `NativeSplash` | `crates/nana-window/src/splash.rs` | The standalone native splash window and system compositor animation for two-phase startup |

## Internal structure

### 1. Separating requests from results: `WindowDescriptor` / `WindowEvent` / `WindowCommand`

`crates/nana-ui-platform/src/window.rs` keeps "what the application requests" strictly separate from "what the platform actually applied":

- **`WindowDescriptor` (`:429`)**: default size `1200×800`, minimum size `760×520`, shadow `WindowShadow::Auto` (`:535-541`). The `visible` field only takes effect after document initialization succeeds (`:442`, preventing an empty window from flashing when initialization fails); `persist_key` provides a size/position restore key that is stable across process restarts (restoration is validated through `clamp_position_to_displays:64`, so if the secondary display saved last time has been unplugged it clamps back to the nearest online display); `tag` lets the application identify windows by business role instead of guessing `WindowId` from creation order (`:464`).
- **`WindowEvent` (`:114`)**: the final result of fullscreen and always-on-top level requests is `WindowEvent::ModeChanged` (`:201`), emitted only when the state actually differs from the previous delivery.

### 2. Native materials, the Win32 frameless chrome guard and the below-owner companion window (`Underlay`)

- **System materials (`crates/nana-window/src/material.rs`)**: `MaterialEffect` covers `Solid`, `Transparent`, `Vibrancy`, `Mica`, `Acrylic` (`:36`). `apply_system_material` (`:176`) always clears the old effect before switching materials, and on failure immediately falls back to opaque solid color and returns `MaterialFallback::{PlatformDoesNotProvideNativeMaterial, NativeMaterialUnavailable}` (`:61`) — it never silently tries another material.
- **Windows frameless transparent windows and style guards (`crates/nana-window/src/chrome.rs`)**: on Windows, as long as the `HWND` keeps the `WS_CAPTION | WS_THICKFRAME | WS_SYSMENU` style bits, DWM draws a native title bar and border below the client area — an opaque window can cover it, but a transparent or rounded window shows the native white edge underneath (`chrome.rs:5-13`). The default is therefore `NonClientRenderingStrategy::StripFrameStyles` (`:71`), which strips those three frame style bits and installs a Win32 window subclass that intercepts `WM_STYLECHANGING` after `DefSubclassProc` (`:1364`) to keep the mask from being overwritten by system operations; the `8 px` outer-edge resize and title-bar drag are then taken over by `LiveFrameResize` / `LiveFrameMove` (`:313 / :471`). On the macOS side it provides `set_present_transaction` (`:256`, locking Metal presentation into a Core Animation transaction to prevent flicker) and `place_native_window_controls` (`:726`, aligning the native traffic-light buttons precisely to the custom-drawn title-bar rect).
- **Below-owner companion window (`Underlay`, `crates/nana-window/src/underlay.rs`)**: `WindowRole::Underlay` creates an ordinary top-level native window sitting directly beneath the parent window, with its own Surface and window title, receiving no input and stealing no focus (`window.rs:302`), so recording tools such as OBS can capture the underlying live image separately without recording the transparent UI panel above it. The parent window is the only geometry authority; the host calls `Underlay::sync` when the parent's geometry, mode, focus or visibility changes (not on every rendered frame, to avoid macOS Window Server round-trip cost, `underlay.rs:13`).

### 3. The single presentation settlement truth: `ResolvedWindowPresentation`

`ResolvedWindowPresentation` (`crates/nana-ui/src/presentation.rs:343`) settles the material negotiation result, the Surface alpha mode, the presentation target (ordinary swapchain vs DirectComposition) and client-area chrome **atomically in one pass** (`:363`). If a transparent material degrades to solid, client-area chrome derives from solid in the same step, so the tear where "the renderer believes it is opaque solid while the native frame is in a transparent frameless state" can never occur.

### 4. Why raw window handles never leave the boundary (three defenses)

| Defense layer | Mechanism |
| --- | --- |
| **1. Compile-time dependency isolation** | `nana-ui-runtime` does not depend on `nana-window`, so ordinary controls cannot even see any platform handle type at compile time |
| **2. Runtime host proxy execution** | For menu bar installation (`SetMenuBar`), file dialogs (`OpenFileDialog`) and other operations that need a parent window handle, controls emit only intent or data models, and the host executes them on the window thread |
| **3. Cross-thread queueing and momentary borrowing** | The `WindowHandle` the application receives (`window_service.rs:271`) is just a `(WindowId, generation, sender)` value object; the only path that touches a native handle, `with_native_handle` (`:279`), provides a temporary borrow reference only inside a window-thread callback |

### 5. Menu bar dual channels, file dialogs and the native splash

- **Menu bar dual channels**: the application declares a `nana_ui_core::MenuBar` model. On macOS it can be installed as the native system menu bar at the top of the screen through `WindowHandle::set_menu_bar`; on custom-drawn title bars or platforms without a native menu such as Linux, the Runtime's [`AppMenuBar`](/components/app-menu-bar) component draws it in the interface. Selected item IDs from both channels flow uniformly into the `push_menu_activation` / `take_menu_activations` queue in `nana-ui-core`.
- **System file dialogs**: controls such as `PathField` only emit `BrowseRequested`; the host calls the native dialog and delivers `WindowEvent::FileDialogCompleted` when it finishes (user cancellation returns an empty path list rather than an error, `window.rs:775`).
- **Native startup window (`NativeSplash`)**: lives in `crates/nana-window/src/splash.rs`, shows a standalone non-activating window before the GPU device is ready, animates it through the system compositor, and takes it down seamlessly once the main window confirms the frame `present` has taken over (see [Two-phase startup](/reference/startup)).

### 6. The headless web content producer: `WebSurface`

`WebSurface` (`crates/nana-window/src/web_surface.rs`) is a platform web engine instance running outside the window view tree (an offscreen `WKWebView` on macOS; a WebView2 Composition Controller on a dedicated STA thread plus `Windows.Graphics.Capture` on Windows): the page is laid out at `WebSurfaceDesc::size` and delivers captured premultiplied RGBA8 frames to the application's `WebFrameSink` on a background thread at up to `max_fps` (capped at `60 fps`, edge length capped at `4096 px`, `:29-31`), with zero conversion and zero blocking on the UI thread (a slow consumer simply drops later frames rather than queueing them, `:21`). When the user needs to log in on a web page or change in-page settings, sending `ShowWindow` temporarily places the same page instance into an ordinary native window for interaction, with cookies and page state preserved throughout (`:11`).

## Code map

| File path | Core content |
| --- | --- |
| `crates/nana-ui-platform/src/lib.rs` | Overall platform-neutral contract exports and input contract re-export |
| `crates/nana-ui-platform/src/window.rs` | `WindowDescriptor`, `WindowEvent`, `WindowCommand`, `WindowGeometry`, `DisplayInfo`, `MousePassthroughMode` |
| `crates/nana-ui-platform/src/window_shadow.rs` | The `WindowShadow` window-level shadow contract |
| `crates/nana-window/src/lib.rs` | Native window support exports, `skip_taskbar`, `show_without_activation`, `settle_layered_window` |
| `crates/nana-window/src/material.rs` | `MaterialEffect`, `MaterialOutcome`, `MaterialFallback` and platform material capability probing |
| `crates/nana-window/src/chrome.rs` | Frameless window style stripping, the `WM_STYLECHANGING` guard subclass, traffic-light placement and edge drag resizing |
| `crates/nana-window/src/{underlay,pointer,splash,menu,file_dialog,size_move}.rs` | Below-owner companion window, pointer-safe conversion, native splash, menu installation, file dialogs and real-size moving |
| `crates/nana-window/src/web_surface.rs` | Headless `WebSurface` descriptor, commands, events and the background frame receiver |
| `crates/nana-ui/src/{window_service,presentation}.rs` | The `WindowService` / `WindowHandle` cross-thread queue and `ResolvedWindowPresentation` |

## Further reading

- [GPU and frame lifecycle](/learn/gpu-frame): Surface ownership, frame slot reservation and the 35-step redraw sequence
- [Native windows, title bars and system materials (Architecture)](/architecture/window) · [Window reference contract](/reference/window)
