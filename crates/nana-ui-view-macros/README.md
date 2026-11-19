# nana-ui-view-macros

English · [简体中文](README.zh-CN.md)

Proc macros for NanaUI's declarative view layer: `view!`, `css!`, `stylesheet!`, `#[component]`, and `#[derive(Store)]`.

`view!` accepts Vue-like templates and expands into plain view-function calls with zero extra runtime overhead. `css!` and `stylesheet!` run through the CSS engine at compile time, generating typed `view::Class` handles for each class in `stylesheet!`. `#[component]` wires named template parameters and `#[prop(default)]` attributes, and `#[derive(Store)]` generates field-tracked reactive accessors for structs.

Consume these macros through `nana_ui::runtime` (or `nana_ui_runtime`) by enabling the `view-macro` feature on `nana-ui` or `nana-ui-runtime` instead of depending on this crate directly. Code generation itself lives in [`nana-ui-view-codegen`](../nana-ui-view-codegen).

## Example

```rust
view! {
    <Column gap=12>
        <Text>"Count {count}"</Text>
        <Button @activate={count.update(|c| *c += 1)}>"Increment"</Button>
        <Text v-if={loading}>"Loading"</Text>
        <Text v-else>"Done"</Text>
        <Slider min=0 max=1 step=0.05 v-model={volume} />
    </Column>
}
```

Literals are constants, `{path}` passes through as-is, and any other `{expr}` expands to `move || expr`.

## Tests

```bash
cargo test -p nana-ui-runtime --features view-macro --test contracts view_macro
```

This test verifies that templates mount the exact same tree and update identically to equivalent hand-written view functions. See [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml) for the rest of the checks.

Learn more in [Reactive View](../../docs/en/reference/reactive-view.md) and [Composing Views](../../docs/en/guide/essentials/view-functions.md).
