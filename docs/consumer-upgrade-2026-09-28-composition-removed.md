# Composition 移除，并入 builder / assembly key

`CompositionSpec` / `CompositionRegistry` / `CompositionHost` 已删除。它在 [上一轮](consumer-upgrade-2026-09-28-composition.md) 并入 assembly key 之后，每一步都已经是 builder 和 assembly 表上的调用，剩下的规则校验、可移动白名单和跨 host 归属检查都是应用策略。它原来能做的事，现在直接用 builder 写：

| 旧 | 新 |
| --- | --- |
| `CompositionSpec` + `CompositionNode<K>` | 应用自己的数据结构；在 `cx.build_child(parent, …)` 里对它递归调用 `ui.with(key, component, \|ui\| …)`。key 可以是运行时的 `String` |
| `CompositionRegistry::register(path, component)` | 递归时按 kind `match` 出控件，可以按数据现场构造，不再是克隆一份登记好的值 |
| `spec.validate(rule)` / `mount(.., rule)` 的规则闭包 | 调用 `build_child` 之前由应用校验自己的数据 |
| `host.mount(cx, parent, …)` | `cx.build_child(parent, …)`：一次 commit 完成建树和插入，任何失败都不改世界。要保留父节点自己的其他 keyed 子节点，就先建一个无 key 的容器，再往容器里 `build_child` |
| `host.node(cx, path)` | `cx.resolve_assembly_path(root, path)` |
| `host.entity::<C>(cx, path)` | `cx.resolve_assembly_entity::<C>(root, path)`（新增）：找不到时是 `MissingView`，类型不对时是 `ViewType` |
| `host.bind_slot(cx, path, parent)` + `.rebindable()` | `cx.place_assembled(node, parent)`；哪些节点允许移动由应用自己维护 |
| `CompositionError::CrossHostParent` | 已删除。节点可以放进任意子树；那棵子树先被销毁时，节点跟着一起销毁，声明它的父节点会忘掉这个 key，重新装配时新建 |
| `host.unmount(cx)` | 销毁根节点（`MutationQueue::despawn_subtree`）；移到别处的节点跟着声明它的父节点一起销毁 |
| `COMPOSITION_PATH_SEPARATOR` | `ASSEMBLY_PATH_SEPARATOR` |

## 行为变化

- **key 合同统一**：`UiBuilder::child` / `with` 和 `mount` 的 `child` 现在都拒绝含 `/` 的 key，返回 `FrameworkError::InvalidInput`，这次构建不提交。此前 builder 接受这种 key，但按路径永远找不到这样的节点。
- `AppContext` 不再维护组合根表。
