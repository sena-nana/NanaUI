# nana-ui-sfc

English · [简体中文](README.zh-CN.md)

The compiler for NanaUI's `.vue` dialect. Scripts are written in Rust and compiled in `build.rs` into ordinary view functions, turning each `Foo.vue` into `pub fn foo(..) -> impl IntoView`.

Templates share the same code generator as `view!`, expanding into the exact call chains you would write by hand—no JS runtime and no runtime template parsing. At compile time it analyzes signal usage across the component: signals that are never written fold into constants, and cyclic `computed` dependencies raise compile-time errors. Output goes to `$OUT_DIR`, including `nana_views.rs`, a dependency manifest, and a source map used by `nana-sfc-remap` to point `rustc` diagnostics back to `.vue` line and column numbers.

This is a build-time dependency; generated code does not depend on it at runtime. File watching and live text patching during development live in [`nana-ui-dev`](../nana-ui-dev).

## Example

`build.rs`:

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

Include the generated views in your application:

```rust
include!(concat!(env!("OUT_DIR"), "/nana_views.rs"));
```

See the complete example in `examples/reactive-sfc`.

## Tests

```bash
cargo test -p nana-ui-sfc
cargo test -p reactive-sfc
```

See [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml) for the rest of the checks.

Learn more in [`.vue` Dialect](../../docs/en/guide/scaling/sfc.md) and [Reactive View](../../docs/en/reference/reactive-view.md).
