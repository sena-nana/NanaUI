# .vue Dialect

Day-to-day views are written with `view!` or the function style. `.vue` files suit migrating from Vue with file-by-file comparison, or keeping template, script and style apart. They are not part of the toggle at the top of the sidebar. The output goes through the same code generator and the same calls as `view!`; the runtime gains no extra capabilities. What you gain is that the compiler sees the whole script, so it can analyze dependencies and fold constants. Editors can bridge `<script>` and template expressions to `rust-analyzer` via `nana-sfc-lsp` for live completion, hover types, go-to-definition, and inline diagnostics.

```vue
<!-- views/TodoList.vue -->
<script setup lang="rust">
let draft = signal(String::new());
let list: Signal<Vec<Todo>> = signal(Vec::new());
let add = move || { /* … */ };
</script>

<template>
  <Column :gap="8">
    <TextInput placeholder="New task" v-model="draft" />
    <Button :disabled="draft.with(|d| d.trim().is_empty())" @activate="add">Add</Button>
    <TodoItem v-for="todo in list" :key="todo.id" :todo="todo.clone()"
              @remove="list.update(|l| l.retain(|t| t.id != todo.id))" />
    <Text v-if="list.with(Vec::is_empty)">No tasks yet</Text>
    <Text v-else>{{ list.with(Vec::len) }} items</Text>
  </Column>
</template>
```

The build script compiles `views/` into ordinary functions. Template errors carry file, line and column.

```rust
// build.rs
fn main() {
    if let Err(error) = nana_ui_sfc::Compiler::new("::nana_ui::runtime").build("views") {
        panic!("{error}");
    }
}

// src/lib.rs
pub mod views {
    include!(concat!(env!("OUT_DIR"), "/nana_views.rs"));
}
```

`TodoList.vue` becomes `pub fn todo_list(…) -> impl IntoView`. A child component declares its parameters with `defineProps!`:

```vue
<script setup lang="rust">
defineProps!(todo: Todo, on_remove: impl Fn() + Send + 'static);
</script>
```

Components compiled in the same batch match parameters by prop name, not by written order. `@remove` maps to `on_remove`. Children map to the last prop named `children`. Missing or extra arguments are compile errors.

## Slots, refs and styles

A child component declares a view parameter such as `header: impl IntoView` and places it in its template with `<slot name="header"/>`. `<slot/>` places `children`. The parent writes `<template #header>…</template>`. `#default` equals the remaining children. Slot content is passed by value, can be placed only once, and has no fallback content.

`ref="name"` writes the node id into `let name = node_ref();` in the script. `on_mount(move |cx| …)` runs after the view enters the tree; use it to focus or read layout. A `key` on a component lands on its first root node. Tags not in this batch call the same-named `#[component]` Rust function under the `view!` rules, passing arguments by attribute name (see [Composing views](https://github.com/sena-nana/NanaUI/blob/main/docs/guide/essentials/view-functions.md)).

`<style>` is component-scoped by default whether or not `scoped` is written; use `<style global>` or a standalone `stylesheet!` for explicit global styles, all compiled at build time. Elements also support inline `style="..."` attributes (composed at Layer 2). Templates still follow Rust ownership: when the same value is passed to a component and also used by an event closure, write `todo.clone()`.

The generated code is formatted and written to `$OUT_DIR/nana_views.rs`, along with `$OUT_DIR/nana_views.map.json` (token-aligned bidirectional spans), which `nana-sfc-remap` uses to map rustc JSON spans back to `.vue` and `nana-sfc-lsp` uses to bridge `rust-analyzer`. Each component's signals and the classification of each binding go into `$OUT_DIR/nana_views.deps.md`.

## What the compiler folds for you

The scan treats `get` / `with` as reads and `set` / `update` as writes. Calls it cannot see through count as dynamic. It cannot see variable shadowing, and does not optimize uncertain cases.

A signal that is never written and never passed out is folded to `constant(…)`: the handle remains, reads create no dependency, and as an attribute it is written once. A binding that only reads constants is written once when the node is built and creates no effect. A computed cycle is a compile error. A signal that is written but never read, or a `watch_effect` that writes a signal it reads, is reported through `cargo:warning`.

Visible static dependencies are wrapped in `__checked`. In debug builds every run checks that the signals actually read do not exceed the declared ones, and a mismatch reports `runtime.reactive.static_deps_mismatch`. In release the closure is returned as-is, so correctness does not rest on this scan. Rerunning a node whose value did not change costs about 70 ns.

When only static text or static `<style>` properties in a `.vue` file change, write `Compiler::new(..).hot(debug)` in the build script and a running window can swap the text or style patches in place. See [Hot reload](https://github.com/sena-nana/NanaUI/blob/main/docs/guide/scaling/hot-reload.md).

## How it compares with hand-written code

`.vue` is within 3% of idiomatic hand-written code (static values passed directly, single signals bound directly) for mount and update. A naive style that makes unchanging values into bindings too costs 2 extra effects and about 830 B resident per row. `.vue` reaches the idiomatic result by folding. `constant(…)` still occupies a signal slot, so it uses about 68 B more per row than idiomatic hand-written code.

Conditional classes are about 11% slower to toggle than a hand-written `FieldWrite`: a rerun has to compute the class mask and look up the `StyleSite`. In debug, `hot(true)` adds one table read per purely static text node. Measured on its own in release, that generated code adds 1 effect and about 315–330 B resident per row, and mounts about 3% slower. The build script compiles 9 `.vue` files in about 0.25 s.

When building trees only from Rust types, `view!` or `.vue`, built-in controls are registered on demand by default and unused controls are tree-shaken automatically with no extra configuration. For size figures see [Extending controls](https://github.com/sena-nana/NanaUI/blob/main/docs/en/guide/components/registry.md).

The contract and measurement conditions are in [Declarative views](https://github.com/sena-nana/NanaUI/blob/main/docs/en/reference/reactive-view.md).

## Next steps

<div class="next-steps">
  <a class="next-step" href="/guide/scaling/hot-reload">
    <p class="next-step-link">Hot reload</p>
    <p class="next-step-caption">How to see the result after changing CSS, packaged output or .vue text.</p>
  </a>
  <a class="next-step" href="/guide/scaling/store">
    <p class="next-step-link">Store</p>
    <p class="next-step-caption">Tracking one nested state by path.</p>
  </a>
  <a class="next-step" href="/guide/components/registry">
    <p class="next-step-link">Extending controls</p>
    <p class="next-step-caption">On-demand registration and where distribution size comes from.</p>
  </a>
</div>
