# Lists

For rows added and removed with a `Vec`, use `v-for`. It expands to `each`. Rows whose key is still present are kept and not rebuilt; node ids and the interaction state on the controls are unchanged.

```rust
#[derive(Clone)]
struct Todo {
    id: u64,
    title: String,
}
```

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::view::IntoView;

fn task_list(list: nana_ui::runtime::view::Signal<Vec<Todo>>) -> impl IntoView {
    view! {
        <Column gap=8>
            <Text v-for={todo in list} key={todo.id}>{todo.title}</Text>
        </Column>
    }
}
```

```rust rust
use nana_ui::runtime::view::*;

fn task_list(list: Signal<Vec<Todo>>) -> impl IntoView {
    column()
        .gap(8)
        .children(list.each(|todo| todo.id, |todo| text(todo.title)))
}
```

:::

`list.each(key, row)` is `each(list, key, row)`. `list` is a `Readable<Vec<T>>`, which `Signal<Vec<T>>` satisfies. `T` must be `Clone`. `key` is `Fn(&T) -> K`, and `K` must be `Eq + Hash + Clone + Send`. `row` receives the cloned item and returns an `impl IntoView`.

In templates `v-for` must write `key`. Leaving it out is a template error. The function style needs a key too: a position name without one counts separately each time a row is built and cannot find that row again.

While the key is still present, a change to a field inside the row should be written on the row's own signal or [Store](https://github.com/sena-nana/NanaUI/blob/main/docs/guide/essentials/reactivity.md), so the whole list is not recomputed. When a row is deleted, only that row's scope is collected and its nodes are destroyed. A new row is mounted in a separate build.

A duplicate key keeps only the first and skips the rest.

::: warning
Do not write `v-for` and `v-if` on the same element. That is a template error and does not become "filter, then loop".
:::

## Fragments and explicit containers

Inside a host parent, `each` defaults to a **boxless fragment**: rows are spliced directly into the host parent's child list, an empty list takes zero space and zero `gap`, and each row participates directly in the parent's flex or grid layout. Only explicit container options such as `.gap(8)`, `.horizontal(8)`, `.class(..)`, or `.key(..)` (or a host-less boundary) materialize a wrapper `Stack`:

```rust
use nana_ui::runtime::view::each;

each(list, |todo| todo.id, |todo| text(todo.title)).gap(8)
```

To wrap that `v-for` in an explicit styled container in a template, wrap that one `v-for` element in `<Block class="strip">`. The `v-for` element's own `class` still lands on each row.

Do not use `each` for a fixed set of children. When the count is fixed, use a tuple. A tree-building `for` goes inside `.with(|c| { … })`; it runs once and does not follow a later `push`.

## Keys also serve lookup

A static node with no key is named by position, as `#v0`, `#v1` and so on. Write `.key("title")` on a node only when `resolve_assembly_path` has to find it by name. Each row in `each` is its own range: different rows can write the same key without covering each other. Explicitly keyed nodes in a row added after mount are registered under their container too.

For a `Store` list use `keyed(|item| item.id).each(|item| …)`; the key is already in `keyed`, and the row closure is only the view. See [Reactivity](https://github.com/sena-nana/NanaUI/blob/main/docs/guide/essentials/reactivity.md).

Do not build every row of a very long list at once. `each_virtual` builds only the rows the viewport covers; the contract is in [Declarative views](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/reactive-view.md). The `each` in this chapter keeps every existing row in the tree.

## Next steps

<div class="next-steps">
  <a class="next-step" href="/guide/essentials/events">
    <p class="next-step-link">Events</p>
    <p class="next-step-caption">on_activate without arguments, and on_input with an event.</p>
  </a>
  <a class="next-step" href="/guide/essentials/conditional">
    <p class="next-step-link">Conditionals</p>
    <p class="next-step-caption">Use when to swap in a hint for an empty list, not v-for on the element.</p>
  </a>
</div>
