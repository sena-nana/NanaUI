# nana-ui-platform

English · [简体中文](README.zh-CN.md)

The layer between NanaUI and the host platform that holds no native handles: window contracts, path spelling, application directories, and host services for clipboard, HTTP(S), WebSocket, and file persistence.

`WindowDescriptor`, `WindowEvent`, and `WindowId` describe window-open requests and results, and are data only. The `path` module (`nana_ui::path` in applications) owns shared path spelling: `normalize_path` never touches the disk, `canonicalize_path` is called only on I/O threads, and the text produced by `display_path` is for display only and must never be written back into settings or used to open files. `ApplicationPaths::resolve` maps data, config, cache, log, and other directories onto the current platform. The crate also re-exports the input contract of [`nana-ui-input`](../nana-ui-input) unchanged.

Native windows, system materials, and title bars are carried out in [`nana-window`](../nana-window); input routing is in `nana-ui-runtime`.

## Features

- `clipboard` (on by default): the system clipboard, based on `arboard` on desktop.
- `fetch` (opt-in): `FetchHost` and the `ureq`-based `NativeFetchHost`. No origin is allowed by default; the application's `FetchPolicy` decides.
- `ws`: `WebSocketHost` and the `tungstenite`-based `NativeWebSocketHost`.
- `experimental-android`: platform capabilities for the experimental Android host.

## Tests

```bash
cargo test -p nana-ui-platform
cargo test -p nana-ui-platform --features ws
```

Other combinations are in [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml). Real clipboard, network, and native window behavior must be verified on the target platform.

See also [Window reference](../../docs/en/reference/window.md) and [Window and platform](../../docs/en/learn/window-platform.md).
