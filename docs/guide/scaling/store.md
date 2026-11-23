# Store

`Signal<App>` 里任何一处变了，读过它的地方都要重跑。`store(value)` 把值放在一处，每条路径各自追踪。路径句柄可以直接当属性。`#[derive(Store)]` 在 `view-macro` 后面，支持具名字段的结构体（可以带泛型）和枚举。派生、`keyed` 和写入没有第二种写法。

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

## 泛型与枚举

派生带上结构体的泛型参数，包括生命周期、内联和 `where` 里的约束、const 参数。trait 也带同样的参数：`Page<'a, T>` 派生出 `PageStoreFields<'a, T>`。store 里的值是 `'static` 的，所以每个参数都要满足 `'static`，例如 `Store<Page<'static, u32>>`。

枚举每个变体生成一个 `is_<变体>()`，每个字段生成一条守卫路径 `Variant`。具名字段叫 `<变体>_<字段>()`，只有一个无名字段的叫 `<变体>()`，多个无名字段的叫 `<变体>_<序号>()`。变体名转成 snake_case，撞上关键字时用原始标识符（`r#type()`）。

```rust
#[derive(Clone, Store)]
enum Load<T> {
    Idle,
    Loading { progress: f32 },
    Ready(T),
    Failed { error: String },
}

let state = store(Load::<Todo>::Loading { progress: 0.0 });
state.is_loading();                   // 追踪；只有整值被写时才重跑
state.loading_progress().set(0.5);    // 只影响读 progress 的地方
state.ready().title().try_with(Clone::clone); // 不是 Ready 时为 None
state.ready().set(todo);              // 不是 Ready 时什么也不做
```

`is_<变体>()` 只在这条路径或它上面的路径被整体写入时重跑。变体字段里的写入不会让它重跑。守卫路径只在值是这个变体时有落点：`try_with` 返回 `None`，`get` / `with` 会 panic，`set` / `update` / 双向绑定什么也不做。载荷自己也派生了 `Store` 时，可以继续往下走：`state.ready().title()`。

## 深追踪和浅追踪

每条路径有两个触发器，都在第一次被追踪读取时才创建。没人读过的路径不占信号。

读值（`get` / `with`）追踪 deep。遍历列表（`keyed(…).each`、`items()`、`len()`）只追踪 shallow。写一条路径，会触发它自己和它下面已经存在的路径，以及它上面各路径的 deep。

`push`、`insert`、`retain`、`swap`、`sort_by_key` 不改任何一项的内容，所以只触发列表本身和上层的 deep，不触发各行。对整个列表 `set` / `update` 会触发所有行。行内绑定重跑之后按字段比较，值没变就停。

行按 key 定位：先看 key 的 64 位哈希，再用 `Eq` 比较 key 本身，所以 `keyed` 的 key 要 `Hash + Eq`。两个 key 哈希相同时各有各的行：先来的 key 用哈希当行 id，后来的 key 沿探测序列拿一个空闲 id。哈希碰撞不会让一个 key 的绑定或 `each` 行读到另一个 key 的项。没有碰撞时，查找和重建索引不额外分配堆内存。重排后行仍指向同一项。离开列表的 key 的行再保留一次 flush，在那之前别的 key 拿不到它的 id，同一个 key 回来就接着用原来的行。行被删掉后，它的触发器随删除释放。对已删除的行 `get` 会 panic，`try_with` 返回 `None`。`at(&key)` 对不在列表里的 key 返回它此刻会得到的 id。key 加进来后句柄就指向它，除非另一个哈希相同的 key 先加进来；那时要重新 `at` 一次。

## 撤销

`store_with_history(value, 上限)` 记住之前的值。`undo()`、`redo()`、`travel(±n)` 前后移动。`can_undo()`、`can_redo()`、`steps()` 是被追踪的，可以直接绑到按钮上。`steps()` 给出每一步写在哪一行。

一次事件处理里的所有写入算一步，界限是两次 flush。开始一步时复制整个值，所以 `T: Clone`，成本和值的大小成正比。上限至少是 1。撤销和重做通过整值写回，只有值真的变了的绑定会更新，行按 key 保留。撤销之后再写入，可以重做的步骤就丢掉。

`.vue` 里 `let x = store(…)`，以及类型是 `Store`、`Subfield`、`Variant`、`Item` 的 prop，都按 store 处理：用到它的绑定在运行时追踪，不会折成常量。单独写这个名字（`:checked="done"`，`done` 是一个 `Subfield` prop）则直接绑定。

字段追踪的表和触发规则在 [声明式视图](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/reactive-view.md)。

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
