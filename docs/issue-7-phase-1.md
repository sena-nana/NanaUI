# Issue #7 Phase 1：Monorepo / Compatibility Boundary

本阶段在 `2026-08-14`、NanaUI `f5687685197e337f0c8e245197722efcc170bf74` 上审核并收口。目标是把 Iced 固定为可退出的兼容实现，而不是在 UiWorld 尚未建立前提前删除现有 working path。

## 验收结论

| Issue #7 要求 | 结果 | 证据 |
| --- | --- | --- |
| Iced fork 合仓 | 通过 | `engine/iced` 为仓内源码；`f568768` 以 `31bde4e` 为第二父提交保留 fork 历史 |
| public API 隔离 Iced types | 通过迁移边界 | backend-neutral contract 位于 `nana-ui-core`、`nana-ui-platform`、`nana-window` 和 JS/Web API crates；现有 `nana-ui` Iced `Element` API 明确定义为 compatibility adapter，不作为新增 Nana-native contract |
| compatibility adapter | 通过 | Issue #7 范围内仅 `nana-ui`、`nana-ui-vue` 直接接入 Iced；仓内既有 `nana-android-host` 不属于本 Issue 的产品范围，也不作为未来移动端架构先例；其他 `nana-*` package 禁止新增非 dev Iced 依赖 |
| upstream sync 流程 | 通过 | [`iced-engine.md`](iced-engine.md) 记录来源、共同祖先、保留 patch、拒绝的 draft patch、同步步骤、验证和退出指标 |

“public API 隔离”在本阶段不是声称现有组件已完成 Nana-native API 改写。`nana-ui` 返回 `iced::Element` 的接口仍是保留行为所需的兼容面；Phase 4 建立新 public API，Phase 5 迁移组件/Vue，Phase 9 达到等价后才移除 Iced 核心路径。提前包装或复制这些接口只会形成第二套长期 API。

## 强制边界

`scripts/check-engine-boundary.py` 以锁定的 Cargo metadata 一次检查三个方向：

1. `engine/iced` 不能依赖任何 `nana-*` package，也不能引用引擎目录外的 path dependency；
2. workspace 中的 `iced`、`iced-wgpu`、`iced-winit` 必须解析到 `engine/iced`；
3. 除显式 compatibility adapter 外，`nana-*` package 不能出现非 dev Iced 依赖。

dev-only Iced 依赖可以用于后端一致性测试，但不会进入中立 crate 的正式依赖图。新增 adapter 必须修改同一 allowlist，因此会成为可审查的架构决定。

复测命令：

```bash
python3 scripts/check-engine-boundary.py
cargo metadata --format-version 1 --no-deps --locked
cargo tree --locked -p nana-ui -i iced
cargo tree --locked -p nana-ui -i iced_wgpu
```

## 阶段复核

后续 Epic 审计将原定义在 `nana-ui::hosted_runtime` 的 pointer/wheel/keyboard event、modifier、pointer type/phase 与 input disposition 下沉到 `nana-ui-platform`；Hosted 原名称只做 compatibility re-export。winit modifier conversion 留在 adapter 内，稳定输入合同不依赖 Iced、winit 或 renderer。

同次审计发现 `nana-ui-platform` 的基础输入/IME/window 合同被无条件 HTTPS `ureq/ring` 和 clipboard 依赖拖入 cross-build。现已拆为默认开启的 `fetch` / `clipboard` features：默认消费者行为不变，`--no-default-features` 可只构建 platform core。backend-neutral core 的可移植构建不据此声称任何目标平台 backend 已验收。

- 已确认 fork 来源不是仅复制文件：`f568768` 的父提交同时包含 NanaUI 合仓提交与原 fork revision。
- 已确认 workspace 不再从远程 git 解析 Iced，正式 `nana-ui` 路径只消费仓内 compatibility engine。
- 已补齐反向门禁：原检查只能阻止 `engine/iced -> nana-*`，现在也能阻止 Iced 依赖继续扩散到新的中立 Nana package。
- 已消除门禁冗余：双向规则共用一个脚本、一次 root metadata 和一个 CI 入口。
- 未新增只做类型转发的 facade crate，也未把 legacy `Element` 包装成另一套假 native API；真正的 identity/state contract 留给 Phase 2，使用者 API 留给 Phase 4。

据此，Phase 1 已形成可执行、可持续且不会扩大迁移面的 compatibility boundary，可以进入 Phase 2。
