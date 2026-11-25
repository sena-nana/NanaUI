# nana-ui-sfc

[English](README.md) · 简体中文

`.vue` 方言的编译器。脚本部分用 Rust 写，在构建脚本里编译成普通的视图函数，每个 `Foo.vue` 生成一个 `pub fn foo(..) -> impl IntoView`。

模板与 `view!` 共用同一个代码生成器，生成的是一个人手写也会写出的调用链，运行时没有 JS，也不解析模板。编译时会分析整个组件的信号用法：从未写入的信号折叠为常量，`computed` 成环报错。输出写到 `$OUT_DIR`，除了 `nana_views.rs`，还有依赖报告和双向源映射；`nana-sfc-remap` 用它把 rustc 诊断回指 `.vue` 的行列，`nana-sfc-lsp` 用它把 `.vue` 的 LSP 请求桥接到 `rust-analyzer`。

它是构建期依赖，生成的代码不依赖它。文件监视和开发期文字/`<style>`热替换在 [`nana-ui-dev`](../nana-ui-dev)。

## 示例

`build.rs`：

```rust
fn main() {
    let debug = std::env::var("PROFILE").as_deref() == Ok("debug");
    if let Err(error) = nana_ui_sfc::Compiler::new("::nana_ui::runtime")
        .hot(debug)
        .build("views")
    {
        panic!("{error}");
    }
}
```

应用里引入生成的代码：

```rust
include!(concat!(env!("OUT_DIR"), "/nana_views.rs"));
```

完整示例在 `examples/reactive-sfc`。

## 测试

```bash
cargo test -p nana-ui-sfc
cargo test -p reactive-sfc
```

其余检查见 [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml)。

更多内容见 [.vue 方言](../../docs/guide/scaling/sfc.md) 和 [声明式视图](../../docs/reference/reactive-view.md)。
