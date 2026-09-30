# L3：用 Rust 建界面

Rust 应用用 [声明式视图](reactive-view.md) 建界面。视图是一个表达式。挂载时一次 commit 进保留的 `UiWorld`。之后只有绑定改过的那个字段会更新。

写成 `view!` 模板，或写成 Rust 函数调用，都可以。两者展开成同一套调用。

底层仍是 `create_component`、`append_child` 和 `on`。不要把挂载当成每帧 `Render`。

第一扇窗口见 [开始](start.md)。开发期改完代码怎么最快看到界面，以及为什么 Rust 层做不到进程内热替换，见 [热重载](hot-reload.md)。

## 写法

```rust
let (view, start) = cx.mount_view_root(document_id, || {
    let start = entity_ref::<Button>();
    let page = column().gap(12).children((
        text("你好"),
        button("开始")
            .entity_ref(start)
            .on_cx(|_, _: &Activate, cx| cx.dispatch_program(Start)),
        row().gap(8).children((button("打开"), button("浮窗"))),
    ));
    with_refs(page, start)
})?;
```

| 写法 | 作用 |
| --- | --- |
| `mount_view_root(document, \|\| view)` | 视图的根作为文档根 |
| `mount_view(parent, \|\| view)` | 根插到已有节点下（不占用父节点的 key） |
| `mount_view_detached(document, \|\| view)` | 根先挂起，交给别处放置（组合控件按 id 收的 slot） |
| `with_refs(view, refs)` | 挂载直接返回 `(MountedView, 解析好的句柄)`：`EntityRef<C>` → `Entity<C>` |
| `.on(\|e\| …)` / `.on_cx(\|组件, e, cx\| …)` | 事件写在元素旁边；`on_cx` 能原地改组件、发程序消息 |
| `.slot` / `.child_slot` / 具名 slot | 交给组合控件的内容（shell 区域、分组的工具、行的图标） |

`Stack` 仍然只是样式容器。子列表不进 `Stack` 字段。组合控件在视图里自己装配。你不用再调 `assemble_*`。

## 身份

- 静态结构只建一次。没写 key 的节点按位置命名。需要按路径找的节点，才写 `.key("…")`。
- 同一 parent 下 key 重复，得到 `DuplicateAssemblyKey`，不 commit。
- key 不能为空，也不能含 `/`（`ASSEMBLY_PATH_SEPARATOR`）。否则是 `InvalidInput`，不 commit。所以视图里每个带 key 的节点都能按路径找到：`resolve_assembly_path(root, "content/list")`。带类型的版本是 `resolve_assembly_entity::<C>(root, path)`。路径从视图的根算起。`mount_view(parent, …)` 的根不带 key 插进 parent。根自己的 key 不登记在 parent 下。否则以后对 parent 调用 `mount` 时，没写到的 key 会连同整个视图一起被删掉。你只是想拿到节点的话，用 `with_refs` 更直接。
- 结构来自数据时，在视图里用普通 Rust 生成子节点。可以 `.children(items.iter().map(…).collect::<Vec<_>>())`，也可以 `.with(|c| for … { c.add(…) })`。要跟着数据变，用 `each` 或 `when`。
- Dock 的面板是 key 等于面板 id 的子节点。

## 更新（不要整树 render）

| 变化 | 做法 |
| --- | --- |
| 字段跟着状态变 | 绑定：`.label(sig)`、`.bind(\|c\| …)`、`text!("{count}")` |
| 消息架构里改一个控件 | 挂载时用 `entity_ref` / `with_refs` 拿到句柄，之后 `update_component(entity, \|v, _\| …)` |
| 增删子项 | `each` / `when`；命令式的 `cx.mount(parent, …)` 仍可用 |
| 虚拟窗口 | `each_virtual`，底层是 `materialize_virtual_*` |

`RuntimeProgram::update` 和点击 handler 不得卸载再重新挂载整棵树。那不是 Nana 的 `notify`。

## 现有 API

| API | 命运 |
| --- | --- |
| 声明式视图（`view!` / Rust 写法） | 产品默认作者面 |
| `create_component` / `append_child` / `on` | 底层、测试、单节点补丁 |
| `create_detached_component` | 保留；产品教程不再教 |
| `mount` | 命令式的 keyed 装配，组合控件的 `assemble_*` 用它 |
| `bind_component` | Vue / 已有 id 的宿主专用 |
| `RuntimeProgram` | 不改成 `cx.new(\|cx\| View)`；消息架构的应用照样用视图建界面 |

## 性能

一次挂载等于一次 `world.commit`。里面是全部的 create、project 和 insert。

子节点在同一批里 insert 进父节点。它们不会先作为文档根单独提交。挂载不做布局，不做抽取，也不做 GPU。

命令式的 `create_component` 加 `append_child` 仍然每次调用 commit 一次。它适合单节点补丁，不适合整页挂载。

### 相对 Vue 的成本

两条路写同一棵 `UiWorld`。布局、文字、命中、抽取和绘制只有一份实现。所以绘制稳态没有差别。60 s、21 万帧的纯 Rust 帧，CPU prepare 的 P95 是 0.0105 ms。见 [高刷新性能记录](../../archive/docs-notes/high-refresh-performance.md)。

差别只在两处：

- **挂载与变更**：Vue 多出 V8 patch，然后每个 DOM op 一次同步跨界调用，再然后是 CSS 级联。5,000 节点构造的 P95 是 25.5 ms。同规模 Runtime 布局的 P95 是 11.6–29.7 ms。同量级，偏高，不是数量级。两条路在 5 k 以上都要分批挂载。
- **每个指针事件**：两边都是常数，不随树增长。L3 约 0.0003 ms。Vue 约 0.062 ms。见 [输入成本](input-cost.md)。

选型标准见 [README](../../README.md#rust-还是-vue)。

## 和 GPUI 的差别

GPUI 的现代面是这样的：`Entity` 保留状态，每帧 `render()` 返回嵌套的 `div().child(...)`。

Nana 的视图也是嵌套表达式，事件就近绑定。但它只在挂载时求值一次。运行时是保留的 `UiWorld`。Vue L1/L2 和 Rust 写同一棵树，增量 flush，稳定的 `StableNodeId`。变化靠细粒度绑定定点更新。

不要抄这些：`div()` 或 Tailwind 式的链式样式 API。那和 Style Model 冲突。要写 CSS 就写 CSS。L3 视图的 `<style>`、`stylesheet!`、`css!` 在构建时由 `nana-ui-css` 编译成 Style Model 数据，见 [reactive-view](reactive-view.md)。也不要抄每帧 `impl Render`，不要抄 `cx.notify()` 重投影整份 View，不要把闭包塞进 `Button` 字段。`ComponentView` 要 `Clone`。

handler 仍在 `AppContext` 表里。只是写在元素旁边。
