# Rust 权限 / transport 边界（Issue #5 MVP #12）

敏感操作必须经 Rust `HostApiRegistry` + [`PermissionPolicy`](../crates/nana-ui-vue/src/capabilities.rs)。Vue UI 改外观或 patch JS **不能**自行授予 capability。

本边界属于 **L1/L2 桥接宿主**（非 Style Model 绘制合同）。三层兼容与 Style Model 见
[`vue-nana-renderer-system.md`](vue-nana-renderer-system.md) §0。  
兼容性阶段 Todo（clipboard = Phase C / X5）见 [`compatibility-roadmap.md`](compatibility-roadmap.md)。

## 默认策略

| Capability | 默认 | Host API |
|------------|------|----------|
| `workspace.read` | 授予（demo） | `workspaceGetBootstrap` |
| `workspace.switch` | **拒绝** | `workspaceSwitch` |
| `secret.read` | **拒绝** | `secretGet` |
| `github.token` | **拒绝** | （经 `secretGet` / 后续专用 API） |

`VueHost` 在 `host_api_registry()` 中自动注册上述 ops。`nana-tauri-demo` 演示宿主默认额外授予 `workspace.switch`（仍由 Rust 代码授予；可用 `--no-grant-workspace-switch` 关闭）。

## 平台 clipboard（与 PermissionPolicy 分立）

`navigator.clipboard.readText` / `writeText` 走 web-api host ops → [`ClipboardHost`](../crates/nana-ui-platform/src/clipboard.rs)，**不是** `PermissionPolicy` 位。

| 目标 | `PlatformCapabilities::clipboard` | 后端 |
|------|-----------------------------------|------|
| 桌面 | `true`（[`desktop()`](../crates/nana-ui-platform/src/lib.rs)） | `OsClipboard`（`arboard`） |
| Android MVP | `false` | `UnsupportedClipboard`（真后端 defer） |

验收：`cargo test -p nana-ui-platform --lib --locked`；`cargo test -p nana-ui-web-api --lib --locked`。

## 验证

```bash
# Unit：HostApiRegistry 直调
cargo test -p nana-ui-vue --lib capabilities --locked

# Integration：VueHost + QuickJS bridge（未 grant → permission denied）
cargo test -p nana-js-quickjs --lib vue_host_denies_privileged_ops_without_grant --locked

# Desktop clipboard（X5）
cargo test -p nana-ui-platform --lib --locked
cargo test -p nana-ui-web-api --lib --locked
```

Consumer app transport（例如 LiliaGithub `src/nana/mockTransport.ts`）应调用 `workspaceGetBootstrap` / `workspaceSwitch`，经 Rust `HostApiRegistry` 强制走 host；NanaUI 不再内置业务 mockTransport fixture。

`nana-tauri-demo` 演示宿主默认 **额外 grant** `workspace.switch`；默认 `VueHost` / 未 grant 策略下 `workspaceSwitch` 与 `secretGet` 必须稳定拒绝。
