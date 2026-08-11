# NanaUI arboard vendor

Vendored from crates.io `arboard` 3.6.1 with an **Android stub platform**
(`src/platform/android.rs`) so `iced_winit` can cross-compile to
`aarch64-linux-android`.

- Desktop / iOS backends are unchanged from upstream.
- On Android, `Clipboard::new()` returns `ClipboardNotSupported`; iced treats
  clipboard as unavailable.
- Patched into the workspace via root `Cargo.toml` `[patch.crates-io]`.

Do not bump casually — keep API surface compatible with the iced rev pinned in
the workspace.
