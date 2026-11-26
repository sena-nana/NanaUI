# NanaUI 的 `accesskit_windows` fork

[English](README.md)

`third_party/accesskit/windows` 存放 NanaUI 基于 `accesskit_windows` 0.35.1 打补丁的版本，发布到 crates.io 时包名为 `nana-accesskit-windows`（`0.35.1-nana.1`）。

它仅包含尚未合入上游的 [AccessKit PR #816](https://github.com/AccessKit/accesskit/pull/816) 对 `src/node.rs` 的改动（本地化 `Switch` 控件类型与 `ScrollPattern`）；`accesskit`、`accesskit_macos`、`accesskit_unix` 和 `accesskit_android` 均直接使用 crates.io 官方版本。

待 PR #816 合入并发布新版本后，即可切回上游 `accesskit_windows` 并移除此目录。
