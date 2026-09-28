# Composition declarations

`CompositionSpec` is a business-agnostic declaration of a retained UI
hierarchy. Nodes have stable `CompositionId` values and a constrained parent /
child relationship. `CompositionSpec::validate` rejects duplicate identities,
cycles, invalid roots, and invalid child kinds before a host mounts anything.

`CompositionHost` owns the mount/unmount bookkeeping. Applications remain
responsible for mapping validated nodes to product controls and for registering
typed extension renderers; the runtime does not depend on application state.

Mounting validates registry coverage before creating Runtime entities, so a
missing renderer leaves the parent subtree untouched. Refresh code should keep
the `CompositionId` and ignore declaration-array positions; reordering a
declaration therefore preserves the same node identity and binding target.

Visibility is not composition state. `CompositionHost` binds identities to
Runtime nodes but does not own their style, and a component's style is written
by its own projection. To hide a declared node, update its component through
the typed binding and set `layout.hidden` (for example
`cx.update_component(host.entity::<Stack>(&cx, &id)?, |stack, _| ...)`). A
hidden node takes no space in its parent and is not painted or hit-tested; see
[layout](layout.md). A parallel host-side visibility flag would have no effect
on layout or paint, so the host deliberately has none.
