# nana-ui-dev

[English](README.md) · 简体中文

NanaUI 应用的开发期热重载，只用于开发构建。

它监视源码，在后台增量编译，成功后把窗口几何和应用交给它的状态传给下一个进程，然后原地 re-exec。不做进程内 dylib 热替换。用 `.vue` 视图时，只改了静态文字或静态 `<style>` 属性的保存会直接送进运行中的窗口，不必重建。重载时保留哪些状态由应用决定，本 crate 只交付 `DevSignal` 消息和交接数据。

没有 `debug_assertions` 的构建会被 `compile_error!` 拒绝，所以 release 入口应该是另一个只调用 `run_runtime` 的 `main.rs`，不引用本 crate。无头会话本身在 [`nana-ui-devtools`](../nana-ui-devtools)。

## Features

- `templates`：引入 `nana-ui-sfc`，提供 `watch_templates`、`watch_template_events`、`TemplateText` 与 `TemplateStyle`，用于 `.vue` 静态文字与静态 `<style>` 补丁热替换。
- `headless`：引入 `nana-ui-devtools`，让 dev 二进制响应 `--stdio`、`--screenshot`、`--a11y`，不开窗也能验证重建结果。

## 示例

完整的 dev 入口在 `examples/l3-dev-entry.rs`：

```bash
cargo run -p nana-ui-dev --example l3-dev-entry
cargo run -p nana-ui-dev --example l3-dev-entry --features headless -- --a11y
```

## 测试

```bash
cargo test -p nana-ui-dev --features templates
```

本 crate 只能在 debug 配置下编译，不要加 `--release`。其余检查见 [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml)。

更多内容见 [热重载](../../docs/guide/scaling/hot-reload.md) 和 [开发期热重载合同](../../docs/reference/hot-reload.md)。
