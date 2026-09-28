# Composition declarations

`CompositionSpec` declares a retained subtree as keyed nodes. It adds no
identity of its own: every node's identity is its **assembly key**, the same
contract `AppContext::build` and `AppContext::mount` use
(`DuplicateAssemblyKey`, `assembly_key`, `assembly_path`). Keys are unique
among siblings; a node is named by the `/`-joined keys from the declaration
root, for example `page/content/list`.

```rust
let spec = CompositionSpec::new(CompositionNode::with_children(
    "page",
    Kind::Page,
    [CompositionNode::with_children(
        "content",
        Kind::Pane,
        [CompositionNode::leaf("list", Kind::Slot).rebindable()],
    )],
));
let mut registry = CompositionRegistry::default();
for (path, _) in spec.nodes() {
    registry.register(path, Stack::column(0.0))?;
}
let mut host = CompositionHost::default();
host.mount(cx, surface_root, &spec, &registry, |parent, child| {
    my_rules(parent.kind, child.kind) // Err(reason) rejects the declaration
})?;
host.bind_slot(cx, "page/content/list", scroll_body)?;
let list = host.entity::<Stack>(cx, "page/content/list")?;
```

## Structure is the application's

Nodes carry a kind of the application's choosing (`CompositionNode<K>`,
`K = ()` by default) and the framework attaches no meaning to it. Which
children a parent may have is the rule passed to `validate` / `mount`; the
framework itself only checks that keys are non-empty, contain no `/`, and
are unique among siblings.

## Mounting

`mount` validates the declaration, checks that every path has a registered
component, then builds the whole tree in one `build_detached` batch and
inserts its root under the parent. Any failure leaves the world as it was. The
root is inserted unkeyed, so the parent's own keyed children (from `build` or
`mount`) are not disturbed; everything below the root is keyed. Nodes are
created in declaration order, so their ids are reproducible.

A mounted host holds only its root and the nodes it may move. Looking a node up
(`node`, `entity`) resolves the path through the assembly tables
(`AppContext::resolve_assembly_path`); there is no host-side identity table to
go stale.

## Moving a slot

A node declared `rebindable()` may be placed under an application-owned parent
with `bind_slot`. This is `AppContext::place_assembled`: the node keeps its key
under the parent it was **declared** under, so its path, `assembly_path` and
lookups are unchanged, and reassembling the declared parent neither pulls it
back nor duplicates it. Placing it back under its declared parent ends the
arrangement. Moving a node into itself, into a dead parent, into another
document, or into a parent that belongs to another composition is rejected
before anything moves.

Ownership follows declarations: a node belongs to the first composition root
found walking from it along declared parents (world parents for unkeyed
nodes), so a slot placed elsewhere still belongs to its host.

## Lifetime

`unmount` despawns the root. A node placed elsewhere goes with the parent that
declared it, whichever path despawns that parent — `unmount`, the application
tearing down the surface, or a reassembly that drops its key. The host is empty
after `unmount` even when the despawn fails, and can mount again.

## Visibility

Visibility is not composition state. A component's style is written by its own
projection, so hide a declared node through its typed binding and set
`layout.hidden` (for example
`cx.update_component(host.entity::<Stack>(cx, path)?, |stack, _| ...)`). A
hidden node takes no space in its parent and is not painted or hit-tested; see
[layout](layout.md).
