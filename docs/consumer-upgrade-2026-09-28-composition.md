# Composition 并入 assembly key

`CompositionSpec` / `CompositionHost` 不再有自己的一套身份。节点身份就是 assembly key（与 `AppContext::build` / `mount` 同一合同），一个节点由从声明根开始、用 `/` 拼接的 key 路径命名。框架不再内置 Page/Pane/Group/Option/Slot/Extension 分类与父子规则；分类是应用自己的类型参数，规则由应用在校验时传入。说明见 [composition](composition.md)。

## 删除

| 旧 | 新 |
| --- | --- |
| `CompositionId`（全局唯一字符串） | 节点的 `key: String`（兄弟间唯一）；查找用路径 `"page/content/list"` |
| `CompositionNodeKind` 与内置父子规则 | `CompositionNode<K>` 的 `kind: K` 由应用定义；规则作为 `validate(rule)` / `mount(.., rule)` 的闭包 |
| `CompositionIndex` / `CompositionEntry` / `validate() -> CompositionIndex` | `validate(rule) -> Result<(), CompositionError>`；遍历用 `spec.nodes()`（父先于子、按声明顺序，带路径） |
| `CompositionHost::index()` | 无；需要结构时从 spec 读 |
| `CompositionError::{EmptyId, RootMustBePage, DuplicateId, Cycle, NotSlot, InvalidSlotParent}` | `InvalidKey(path)`、`DuplicateKey(path)`、`InvalidChild { parent, child, reason }`、`NotRebindable(path)`、`UnknownPath(path)`、`NotMounted`；环、跨文档、死父节点来自 `Runtime(..)` |
| `set_visible` / `is_visible`（此前已删除） | `host.entity::<C>(cx, path)` + `update_component` 设置 `layout.hidden` |

## 变化

| 旧 | 新 |
| --- | --- |
| `CompositionNode::leaf(id, kind)` / `with_children(id, kind, children)` | 签名不变，`id` 改为本层 key；可移动的节点追加 `.rebindable()` |
| `registry.register(id, component)` | `registry.register(path, component)`，路径含根 key，例如 `"page.stage/content/list"` |
| `host.mount(cx, document, parent, &spec, &registry)` | `host.mount(cx, parent, &spec, &registry, rule)`；文档取自 `parent` |
| `host.node(&id)` | `host.node(cx, path)` |
| `host.entity::<C>(cx, &id)` | `host.entity::<C>(cx, path)` |
| `host.bind_slot(cx, &id, parent)` | `host.bind_slot(cx, path, parent)`；只允许声明为 `rebindable()` 的节点 |
| 仅 `Slot` 分类可以跨布局边界 | 由声明上的 `rebindable()` 决定 |

## 行为

- `mount` 一次提交整棵树；任何失败都不改世界。根节点无 key 地插入父节点，不影响父节点自己的 keyed 子节点。
- `bind_slot` 移动后节点仍以声明路径命名，`AppContext::assembly_path` 不变；重新装配声明父节点不会把它拉回。
- 放到别处的 slot 跟随声明它的父节点一起销毁：`unmount`、应用直接销毁页面父节点、或重新装配时去掉它的 key，都会带走它。此前 `unmount` 会把已绑定到外部父节点的 slot 留在世界里。
- `commit_mutations` 里的 `DespawnSubtree` 现在会清掉被删节点的 view、事件处理器和 assembly 记录，此前只有 `despawn_node` 与 `build` 会清。

## 新增的通用 API（`AppContext`）

- `place_assembled(node, parent)`：保持身份移动 keyed 节点。
- `resolve_assembly_path(root, "a/b/c")`：按 key 路径在 root 下查找。
- `assembly_key` 改为 O(1) 反查；`assembly_path` 沿声明父链拼接。
