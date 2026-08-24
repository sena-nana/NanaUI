# NanaUI arboard vendor

Vendored from crates.io `arboard` 3.6.1 with an **Android stub platform**
(`src/platform/android.rs`) so `nana-ui-platform` can cross-compile to
`aarch64-linux-android`.

- Desktop / iOS backends are unchanged from upstream, except macOS
  `osx.rs`: drop `unsafe` around objc2 APIs that are already safe (rustc
  `unused_unsafe`).
- On Android, `Clipboard::new()` returns `ClipboardNotSupported`; the
  platform clipboard host reports clipboard as unavailable.
- Patched into the workspace via root `Cargo.toml` `[patch.crates-io]`.

Do not bump casually — keep the Android stub and desktop API surface.
