# Teleport

Some content needs to be written next to a button but painted in an overlay. `teleport` builds this view at the declaration site and swaps its parent in the tree for the target you give. Layout, painting, hit testing, focus order, and accessibility all follow the target's position. The scope still belongs to the declaration site; when the declaration site is destroyed, the content is destroyed with it.

`layer` is a `node_ref()`, first handed to the target node. The toggle at the top of the sidebar swaps only the teleported part.

```rust
use nana_ui::runtime::view::{column, node_ref, widget};
use nana_ui::runtime::Stack;

let layer = node_ref();
```

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <Column gap=8>
        <Widget of={Stack::column(0.0)} ref={layer} />
        <Teleport to={layer}>
            <Text>"In the overlay"</Text>
        </Teleport>
    </Column>
}
```

```rust rust
use nana_ui::runtime::view::{column, teleport, text, widget};
use nana_ui::runtime::Stack;

column().gap(8).children((
    widget(Stack::column(0.0)).node_ref(layer),
    teleport(layer, text("In the overlay")),
))
```

:::

`to` can be a `NodeRef`, a node id, or a closure that selects an `Option<StableNodeId>`. When it changes, the content moves with it. When the value is `None`, or the target is not in the tree, the content returns to its exact slot at the declaration host. Inside a host container it is a boxless fragment by default, occupying zero `gap` and zero space while teleported away; only when given `.key`, `.class`, `.class_when`, `.css`, or built without a host parent does it materialize a `Stack::column(0)` anchor container at the declaration site.

In a template, record the target with `ref={layer}`, which expands to `.node_ref(layer)`. `layer` has an id only after mounting. Teleporting happens when this content enters the tree.

Explicit `.class`, `.class_when`, and `.css` (in templates `<Teleport to={layer} class="fill">`) materialize an anchor container so content that has not been teleported yet fills the height. If the target appears later, the content moves there.

The content's scope belongs to the declaration site. When the declaration site is destroyed, the content painted under the target at that moment is destroyed with it. Windows are still opened by the host. Teleporting only changes the parent within this tree.

## Classes on the anchor

`.class`, `.class_when`, and `.css` are written on the return value of `teleport(…)`, or on the `<Teleport>` tag, and materialize and decorate that anchor column (without container options or `.key`, no wrapper node is created). Once the content has been mounted under the target, that explicit anchor column becomes empty.

When `to` is a closure, the content moves again whenever the id it selects changes. If the target is no longer in the tree at that evaluation, the content returns to the anchor.

## Next steps

<div class="next-steps">
  <a class="next-step" href="/guide/built-ins/suspense">
    <p class="next-step-link">Suspense</p>
    <p class="next-step-caption">A fallback shown until the first load completes.</p>
  </a>
  <a class="next-step" href="/guide/components/slots">
    <p class="next-step-link">Named slots</p>
    <p class="next-step-caption">A dialog's own slots, not teleporting elsewhere.</p>
  </a>
  <a class="next-step" href="/guide/essentials/refs">
    <p class="next-step-link">Handles</p>
    <p class="next-step-caption">Which node node_ref records at mount time.</p>
  </a>
</div>
