# 开发期热重载合同

本文说明开发期热重载（`nana-ui-dev`）的响应延迟、状态交接机制以及为什么 Rust L3 采用进程重执（re-exec）而非 dylib 热替换。快速配置步骤见指南中的 [热重载](https://github.com/sena-nana/NanaUI/blob/main/docs/guide/scaling/hot-reload.md)。

## 各层改动方式与实测延迟

`nana-ui-dev` 顶部带有编译期门禁 `#[cfg(not(debug_assertions))] compile_error!(...)`——而发布配置 `[profile.dist]` 继承自 `release`，因此一旦产品正式构建误引入该 crate 会直接编译失败，绝无可能将文件监视器带入用户交付物。

| 修改的内容 | 触发的重载动作 | 实测量级 |
| --- | --- | --- |
| **Rust `.vue` 方言：仅改静态文字或静态 `<style>` 属性** | **替换静态文本表（`TemplateText::apply`）或开发期样式补丁表（`TemplateStyle::apply`）**：不重编译、不重启，节点与焦点原地保留 | `< 50 ms` |
| **Rust L3：修改 `.rs` 源码** | **后台增量编译 → 交接窗口几何与会话状态 → `re-exec`**：窗口仅闪动一次 | `1.5–3.6 s`（链接占大头） |

在 macOS（Apple Silicon，已预热的默认 `dev` 配置）上实测 `cargo build -p component-gallery`：仅修改应用 crate（1.8 万行）并重链接 131 MB 二进制耗时 **`1.5–1.6 s`**；修改底层 `nana-ui-runtime`（12.7 万行，连带重编 `scene` 与 `nana-ui`）耗时 **`3.6 s`**。Windows 平台链接耗时更长，建议在本地通过 `cargo build -p <包名> --timings` 实测并为 dev 配置开启 `lld`。

## Rust L3 路径：为什么采用进程重执（Re-exec）而非 dylib 热替换

Rust L3 不做进程内 `dlopen / dlclose` 代码热替换，因为在不破坏框架核心合同的前提下有五道硬障碍：

1. **`TypeId` 跨编译不稳定**：声明式视图与 keyed 装配依赖 `TypeId::of::<C>()`，重新编译出的 dylib 对同一类型会生成不同的 `TypeId`，导致所有 keyed 子节点失配重建、`remove_view` 直接失败；
2. **`RuntimeProgram` 不是对象安全的（not object-safe）**：`with_document` 为泛型闭包方法，无法放进 vtable，若跨 FFI 包装则等于引入 `AGENTS.md` 严令禁止的第二套 C-ABI；
3. **事件闭包持有指向旧代码段的裸函数指针**：安全卸载 dylib 前必须先拆毁整棵树，与重建无异；
4. **平台动态库卸载限制**：macOS 的 `dlclose` 不会真正卸载含有 Objective-C 元数据或线程本地存储（TLS）的镜像；
5. **全局静态量分裂**：dylib 若重复链接 `wgpu` 会产生两套实例。

因此 Rust L3 的诚实方案是**后台增量编译 + 状态交接 + 原地 `re-exec`**（完整示例见 [`crates/nana-ui-dev/examples/l3-dev-entry.rs`](https://github.com/sena-nana/NanaUI/blob/main/crates/nana-ui-dev/examples/l3-dev-entry.rs)）：

```rust
// 1. main 入口使用 run_with_restart 包装
nana_ui_dev::run_with_restart::<MyProgram>(settings, Path::new("target/nana-dev-handoff"))?;

// 2. initialize 中恢复上一轮交接的会话状态（首次启动或结构体字段变更反序列化失败时优雅回退 Default）
let session = nana_ui_dev::restored_handoff()
    .and_then(|handoff| handoff.state_as::<Session>())
    .unwrap_or_default();
let watcher = nana_ui_dev::watch_and_rebuild(
    &DevConfig::new_rust("src"),
    RebuildCommand::cargo_package("my-app"),
    context,
)?;

// 3. update 中收到 DevSignal::Rebuilt 时交接窗口几何与业务会话状态并退出重执
Message::Dev(DevSignal::Rebuilt) => {
    nana_ui_dev::request_relaunch(DevHandoff { /* 窗口几何 */ }.with_state(&self.session));
    RuntimeProgramUpdate { exit: true, ..Default::default() }
}
```

此外，若使用 `.vue` 方言（Rust 脚本）编写视图且仅修改了静态文案或静态 `<style>` 规则声明，可在 `build.rs` 中开启 `Compiler::new("::nana_ui::runtime").hot(debug)` 并在 dev 入口使用 `watch_templates`（或 `watch_template_events`，`templates` feature）。静态文字通过 `TemplateText::apply()` 原地替换，静态 `<style>` 补丁通过开发期专用数据通道 `TemplateStyle::apply()`（调用 `nana-ui-css` 重编译为 `HotStyleSheet`，仅在开发热模式启用，不进入产品路径）原地注入；若新增了 `transition`、`@keyframes`、组合器或新的交互伪类掩码，则仍会触发增量重建。

## 结合无头 Agent 会话实现免开窗验证

为 `dev` 二进制同时开启 `headless` feature（引入 `nana-ui-devtools` 的 `runtime-agent` 层），即可让同一个 `dev` 可执行文件兼具「正常桌面窗口」与「无头 JSON 自动化验证」两种模式——每次重编译后无需人工重新点击进入深层页面，直接通过管道发送 JSON 指令即可完成点击、无障碍树检查与离屏截图：

```bash
cargo build -p my-app --bin dev --features dev,headless

printf '%s\n' \
  '{"id":1,"cmd":"a11y"}' \
  '{"id":2,"cmd":"screenshot","path":"target/agent/after.png"}' \
  '{"id":3,"cmd":"click","agent_path":"open"}' \
  | ./target/debug/dev --stdio --width 320 --height 200 --theme dark
```
