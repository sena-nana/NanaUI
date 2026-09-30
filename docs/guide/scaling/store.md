# Store

`Signal<App>` 里任何一处变了，读过它的地方都要重跑。`store(value)` 把值放在一处，每条路径各自追踪。路径句柄可以直接当属性。`#[derive(Store)]` 在 `view-macro` 后面，只支持具名字段、不带泛型的结构体。派生、`keyed` 和写入没有第二种写法。

```rust
use nana_ui::runtime::view::{Store, StoreList, StorePath, store};

#[derive(Clone, Store)]
struct Todo {
    id: u64,
    title: String,
    done: bool,
}

#[derive(Store)]
struct App {
    todos: Vec<Todo>,
    filter: String,
}

let app = store(App {
    todos: Vec::new(),
    filter: String::new(),
});
let todos = app.todos().keyed(|todo| todo.id);
```

:::api

```rust view
use nana_ui::runtime::view;

todos.each(|todo| view! {
    <Row gap=4>
        <Text>{todo.title()}</Text>
        <Checkbox checked={todo.done()}>"完成"</Checkbox>
    </Row>
});
```

```rust rust
use nana_ui::runtime::view::{checkbox, row, text};

todos.each(|todo| {
    row().gap(4).children((
        text(todo.title()),
        checkbox("完成").checked(todo.done()),
    ))
});
```

:::

```rust
todos.at(&7).done().set(true);
app.todos().push(todo);
app.filter().set("done".into());
```

派生生成 `TodoStoreFields`、`AppStoreFields`。用到 `.title()`、`.todos()` 时，对应的 trait 要在作用域里。同一模块里派生就够了。列表方法来自 `StoreList`，读写来自 `StorePath`：`get`、`with`、`try_with`、`set`、`update`。

`keyed` 的参数是 `fn(&T) -> K`。不捕获环境的闭包可以当成这个函数指针。句柄和信号一样是 `Copy` 的 id。

`todos.at(&7).done().set(true)` 只更新读了这一格的绑定。`app.todos().push(todo)` 给列表加一行，已有行的内容不会被重读。`app.filter().set(…)` 只影响读 `filter` 的地方。

## 深追踪和浅追踪

每条路径有两个触发器，都在第一次被追踪读取时才创建。没人读过的路径不占信号。

读值（`get` / `with`）追踪 deep。遍历列表（`keyed(…).each`、`items()`、`len()`）只追踪 shallow。写一条路径，会触发它自己和它下面已经存在的路径，以及它上面各路径的 deep。

`push`、`insert`、`retain`、`swap`、`sort_by_key` 不改任何一项的内容，所以只触发列表本身和上层的 deep，不触发各行。对整个列表 `set` / `update` 会触发所有行。行内绑定重跑之后按字段比较，值没变就停。

行按 key 的 64 位哈希定位。重排之后仍指向同一项。同一列表里两个 key 的哈希不能相同。行被删掉后，它的触发器在下一次按 key 查找时释放。对已删除的行 `get` 会 panic，`try_with` 返回 `None`。

## 撤销

`store_with_history(value, 上限)` 记住之前的值。`undo()`、`redo()`、`travel(±n)` 前后移动。`can_undo()`、`can_redo()`、`steps()` 是被追踪的，可以直接绑到按钮上。`steps()` 给出每一步写在哪一行。

一次事件处理里的所有写入算一步，界限是两次 flush。开始一步时复制整个值，所以 `T: Clone`，成本和值的大小成正比。上限至少是 1。撤销和重做通过整值写回，只有值真的变了的绑定会更新，行按 key 保留。撤销之后再写入，可以重做的步骤就丢掉。

`.vue` 里 `let x = store(…)`，以及类型是 `Store`、`Subfield`、`Item` 的 prop，都按 store 处理：用到它的绑定在运行时追踪，不会折成常量。单独写这个名字（`:checked="done"`，`done` 是一个 `Subfield` prop）则直接绑定。

字段追踪的表和触发规则在 [声明式视图](../../reference/reactive-view.md)。

## 接下来

<div class="next-steps">
  <a class="next-step" href="/guide/scaling/packaging">
    <p class="next-step-link">打包</p>
    <p class="next-step-caption">把资源和可执行文件放进安装布局。</p>
  </a>
  <a class="next-step" href="/guide/essentials/reactivity">
    <p class="next-step-link">响应式</p>
    <p class="next-step-caption">信号、作用域，以及 store 和 Signal 的差别。</p>
  </a>
  <a class="next-step" href="/guide/essentials/list">
    <p class="next-step-link">列表</p>
    <p class="next-step-caption">keyed().each 保留的行怎样对上 key。</p>
  </a>
</div>
