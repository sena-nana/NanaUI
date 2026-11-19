# nana-window

English · [简体中文](README.zh-CN.md)

The native window layer for NanaUI applications: system materials, custom-drawn title bars and frameless windows, live resize, workspace fullscreen, native menu bars and file dialogs, system preference queries, screen color sampling (`ColorSampler`), auxiliary parent-ordered windows (`WindowDescriptor::parent`), and native splash windows.

Functions borrow the window via `impl HasWindowHandle` for the duration of the call without retaining handles; raw window handles never leave this crate or the host layer. When requesting a system material, `apply_system_material` returns the actual `MaterialOutcome`, or an explicit `MaterialFallback` if unsupported—never silently substituting another material. `WebSurface` is a headless web page producer whose captured frames route to an application sink without entering any window's view tree.

Only [`nana-ui`](../nana-ui)'s `hosted` host depends on it; ordinary controls never access these APIs. Window, Surface, and the event loop belong to the host, while platform-neutral contracts (`WindowDescriptor`, `WindowEvent`, `ColorSampler`) live in [`nana-ui-platform`](../nana-ui-platform).

Capabilities vary by platform: macOS and Windows support system materials, `Underlay`, and `NativeSplash`, whereas on Linux materials fall back to opaque and those two modes are unsupported. See the window reference for the full compatibility matrix.

## Examples

Standalone probes under `examples/` exercise native code paths on the main thread and read back platform-held state:

```bash
cargo run -p nana-window --example file-dialog-probe
cargo run -p nana-window --example web-surface-probe
```

## Tests

```bash
cargo test -p nana-window --lib
```

Unit tests cover pure logic such as path classification and command spelling; native behaviors require manual verification on target platforms.

Learn more in [Window Reference](../../docs/en/reference/window.md) and [Native Window, Title Bar & Materials](../../docs/en/architecture/window.md).
