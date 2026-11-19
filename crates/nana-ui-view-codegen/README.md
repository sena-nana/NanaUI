# nana-ui-view-codegen

English · [简体中文](README.zh-CN.md)

The code generator that expands template element trees into `nana-ui-runtime` declarative view calls, shared by `view!` and the `.vue` compiler. Applications do not need to depend on this crate directly.

Built-in tags and attributes come from the control table in [`nana-ui-view-schema`](../nana-ui-view-schema), so misspelled tags or properties produce span-accurate template errors while unrecognized tags call same-named component functions. `<style>`, `stylesheet!`, and `css!` are also compiled into style data here via [`nana-ui-css`](../nana-ui-css). Template text parsing lives separately in [`nana-ui-view-macros`](../nana-ui-view-macros) and [`nana-ui-sfc`](../nana-ui-sfc).

## Tests

```bash
cargo test -p nana-ui-view-codegen
cargo test -p nana-ui-runtime --features view-macro --test contracts view_macro
```

Learn more in [Authoring Toolchain](../../docs/en/learn/toolchain.md).
