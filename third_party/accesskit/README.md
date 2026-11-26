# NanaUI's `accesskit_windows` fork

[中文](README.zh-CN.md)

`third_party/accesskit/windows` holds NanaUI's patched `accesskit_windows` 0.35.1 crate, published to crates.io as `nana-accesskit-windows` (`0.35.1-nana.1`).

It carries the unmerged upstream [AccessKit PR #816](https://github.com/AccessKit/accesskit/pull/816) change in `src/node.rs` (localized `Switch` type and `ScrollPattern`), while `accesskit`, `accesskit_macos`, `accesskit_unix`, and `accesskit_android` resolve directly from crates.io.

Once PR #816 is merged and released on crates.io, `nana-ui` can switch `accesskit_windows` back to the upstream crate and remove this vendored copy.
