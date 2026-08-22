# Vue 兼容目标收缩记录

2026-08-12 起，Issue #5 的 L1 合同从“完整 Tauri Vue”收缩为 WebView 中常见 Vue 3
+ JavaScript 源码的 Nana 兼容子集。

权威范围与验收：

- [`../compatibility-roadmap.md`](../compatibility-roadmap.md)
- [`../vue-nana-renderer-system.md`](../vue-nana-renderer-system.md)
- [`../capabilities.md`](../capabilities.md)
- [`../release-artifacts.md`](../release-artifacts.md)

旧 Tauri/Lilia runtime 性能报告和 L1/Lilia 快照不再属于当前合同，已删除。原生
Gallery/LiliaUI 设计基线、通用 Vue Counter 证据以及其他进行中的原生工作不受影响。

本次没有视觉行为变更，因此不新增视觉快照。验收以可复现 SFC 构建、Fetch 行为、
当时 QuickJS/V8 同产物语义树、workspace/Clippy 和 Android ARM64 交叉编译为准。
QuickJS 此后已移除；产品引擎为 V8。Android ARM64 交叉编译现不链接 V8 预编译库。
