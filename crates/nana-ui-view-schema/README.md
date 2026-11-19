# nana-ui-view-schema

English · [简体中文](README.zh-CN.md)

The registry table of built-in controls recognized by declarative views, exporting a single macro: `for_each_control!`. Applications do not need to depend on this crate directly.

Each entry specifies a control's template tag, constructor function and arguments, bindable fields, `v-model`, and events. `nana-ui-runtime` expands the table into typed setters and event methods, while [`nana-ui-view-codegen`](../nana-ui-view-codegen) expands the exact same table into accepted template tags and attributes—so adding a control updates both sides together. Control implementations live in `nana-ui-runtime`; custom application components do not need to register here.

This crate has no standalone tests; table entries are verified by expansion on both sides:

```bash
cargo test -p nana-ui-runtime --features view-macro --test contracts view_macro
```

Learn more in [Authoring Toolchain](../../docs/en/learn/toolchain.md).
