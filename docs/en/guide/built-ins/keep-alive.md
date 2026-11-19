# KeepAlive

By default `when` tears down the branch that is not shown. `.keep_alive()` keeps it, along with its nodes and state, so it reappears exactly as it was when you switch back. List rows do not need this: `each` already keeps rows by key.

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <KeepAlive>
        <TextInput v-if={editing} v-model={draft} />
        <Text v-else>{draft}</Text>
    </KeepAlive>
}
```

```rust rust
use nana_ui::runtime::view::{text, text_input, when};

when(editing, || text_input().model(draft))
    .otherwise(|| text(draft))
    .keep_alive()
```

:::

`<KeepAlive>` wraps one `v-if` / `v-else-if` / `v-else` chain. Apart from `class`, it has no attributes. A hidden branch moves into a parked hidden `Stack` outside the host's child list: it occupies no child slot, takes no part in layout, painting, or hit testing, and focus moves away. When the host is destroyed, the kept branches and parked holder are destroyed with it.

Combined with `<Transition>`, a kept-alive branch plays no leave animation when switched away and plays its enter animation when switched back. See [Transition](https://github.com/sena-nana/NanaUI/blob/main/docs/guide/built-ins/transition.md).

## Keeping several pages by key

Tabs are not two branches, true and false. `dynamic(key, render)` shows one view per key, corresponding to Vue's `<component :is>`. `.keep_alive()` keeps the view for every previous key, and `.max(n)` keeps at most n that are not showing, dropping the one shown least recently first.

```rust
use nana_ui::runtime::view::dynamic;

dynamic(tab, |tab| match tab {
    Tab::Notes => notes(),
    Tab::Preview => preview(),
})
.keep_alive()
.max(2)
```

The limit exists only in the function style. The template `<KeepAlive>` has no `max`. Wrapping a `v-for` is also an error: list rows are kept by their own keys, not by this chain.

A kept branch's scope is not reclaimed; its nodes wait in that parked hidden `Stack`. Switching back splices the original nodes directly into the host's child slot.

`when(…).keep_alive()` keeps at most one branch that is not showing. `v-if` with `v-else` is exactly that one. When the chain also has `v-else-if`, branches that left earlier are dropped and only the most recently hidden one is kept. To keep several pages by key, use `dynamic`.

A class on `<KeepAlive class="panel">` (or `.class` after `keep_alive()`) materializes a wrapper container for the chain and applies the class there. Without container options it is boxless and adds no extra `gap`.

## Next steps

<div class="next-steps">
  <a class="next-step" href="/guide/built-ins/teleport">
    <p class="next-step-link">Teleport</p>
    <p class="next-step-caption">Content stays where it is declared and paints on another layer.</p>
  </a>
  <a class="next-step" href="/guide/built-ins/suspense">
    <p class="next-step-link">Suspense</p>
    <p class="next-step-caption">A fallback shown while waiting for the first load.</p>
  </a>
  <a class="next-step" href="/guide/built-ins/transition">
    <p class="next-step-link">Transition</p>
    <p class="next-step-caption">How the enter animation plays when switching back.</p>
  </a>
</div>
