# builder 转为内部：界面用视图建

`UiBuilder` 只剩视图层内部在用，它把视图落到 `UiWorld` 上。应用建界面用声明式视图（`view!` 或 Rust 写法，见 [L3：用 Rust 建界面](l3-authoring.md)），单个节点的补丁用 `create_component` / `append_child` / `update_component`，keyed 的动态区仍用 `mount`。

## 需要改的地方

| 旧 | 新 |
| --- | --- |
| `cx.build(document, \|ui\| …)` | `cx.mount_view_root(document, \|\| view)`；要拿句柄，闭包返回 `with_refs(view, refs)` |
| `cx.build_detached(document, \|ui\| …)` | `cx.mount_view_detached(document, \|\| view)` |
| `cx.build_child(parent, \|ui\| …)` | `cx.mount_view(parent, \|\| view)`；要和以后的 `mount` 共用 key，用 `cx.mount(parent, \|scope\| …)` |
| `ui.child(key, c)` / `ui.with(key, c, \|ui\| …)` | `widget(c).key(key)` / `widget(c).key(key).children(…)`；静态结构不需要 key |
| `resolve_assembly_path(parent, "page/content")`，`page` 是 `build_child(parent, …)` 建的 | 视图的根不带 key 插进 parent，根的 key 不登记在 parent 下（否则以后对 parent 的 `mount` 会删掉它）：路径从根算起，`resolve_assembly_path(root, "content")`，根从 `MountedView::roots()` 或 `entity_ref` 拿。只为拿节点的话用 `with_refs` |
| `ui.column(gap, …)` / `ui.row(gap, …)` | `column().gap(gap).children(…)` / `row()…` |
| `ui.on(entity, \|c, e, cx\| …)` | 元素上写 `.on_cx(\|c, e: &E, cx\| …)`（不要组件时用 `.on(\|e\| …)`） |
| `ui.detached(c)`：交给组合控件按 id 收 | 组合控件的具名 slot（`.primary(view)`、`.body(view)`…）或通用的 `.slot(view, write)`；交给 `set_modal_slots` 这类按 id 收的接口时用 `detached(view)` 加 `entity_ref` |
| `ui.parked(c)` + `ui.nest(p, \|ui\| ui.adopt(c))` | 直接写成子节点；一定要先建子节点时用 `.child_slot(view, write)` |
| 组合控件建好后调 `assemble_*` | 在视图里建的组合控件自己装配，不用再调 |
| `FrameworkError::UnplacedNode` | 删除：视图里不存在"挂起了却没人放置"的节点。穷尽匹配 `FrameworkError` 的地方去掉这个分支 |

`UiBuilder` 不再从 `nana_ui::runtime` 导出，`AppContext::build` / `build_detached` / `build_child` 不再公开。

## 行为变化

没有。视图本来就落到同一条路径上：同一次 commit、同一张 assembly key 表、同样的 `ComponentView::reconcile`。

## 性能

`nana-reactive-benchmark` 的对照组从 builder 换成逐个 `create_detached_component` + `append_child`（每次一次 commit）。挂载时视图快一成多，数字见 [声明式视图的成本](reactive-view.md#成本)。
