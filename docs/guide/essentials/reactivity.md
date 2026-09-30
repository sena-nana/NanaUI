# 响应式

状态放在信号里。树在挂载时建一次，之后每个绑定只改它自己写的那个字段。不整树重绘，也不做 diff。

两种都调用 `signal`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::view::{IntoView, signal};

fn counter() -> impl IntoView {
    let count = signal(0u64);
    view! {
        <Column gap=8>
            <Text>"计数 {count}"</Text>
            <Button @activate={count.update(|c| *c += 1)}>"增加"</Button>
        </Column>
    }
}
```

```rust rust
use nana_ui::runtime::view::*;

fn counter() -> impl IntoView {
    let count = signal(0u64);
    column().gap(8).children((
        text!("计数 {count}"),
        button("增加").on_activate(move || count.update(|c| *c += 1)),
    ))
}
```

:::

`signal(0u64)` 得到 `Signal<u64>`。它是 `Copy` 的 id，值放在创建它的那个线程上。拿到别的线程上用会 panic。

## 读和写

`get` 克隆当前值，并记下这次读取。`with` 只借出引用，适合 `list.with(Vec::is_empty)` 这种不需要克隆的判断。`T` 不是 `Clone` 时没有 `get`，用 `with`。

`set` 换上新值并通知，就算新值和旧值相等也通知。`update` 就地改，然后通知。传给 `update` 的闭包不要再读同一个信号。

`get_untracked` 和 `with_untracked` 读，但不建立依赖。`untrack(|| …)` 包住一整段，里面的读取都不记。作用域已经回收后再读写，会报 `runtime.reactive.disposed_access`，同时 panic。`try_update` 在信号已经不在时返回 `false`，不 panic。

`set` 和 `update` 只把订阅者排进队列，不立刻改树。输入路由在每个事件末尾调用 `flush_reactive`，每帧开头也会再刷一次。你在 `RuntimeProgram::update` 里改信号，有待应用绑定的窗口也会被请求重绘。

属性可以接常量、`Signal<T>`、`Computed<T>`，或一个 `Fn() -> T`。常量在建节点时写进去。信号直接绑定，不装箱闭包。单独写在插值里的信号，例如 `"计数 {count}"`，读的时候自动成为依赖。

::: warning
在构造视图时直接 `count.get()`，读到的是当时的常量，之后不再跟着变。要跟着变，把信号本身传进去，或者传一个闭包。
:::

函数写法里 `.label(count.get())` 就是这种常量。模板里的 `{count.get()}` 是一段表达式，会包成 `move || count.get()`，那才会跟着变。能直接传信号时就传信号。

信号在挂载闭包、`each` 的一行或 `when` 的一个分支里创建，就归那个作用域。作用域回收时一起丢掉。在这些作用域之外创建的信号没有归属，会留到线程结束。列表里每一行要用的信号，写在行视图里。

## 按字段追踪

`Signal<App>` 里任何一处变了，读过它的地方都要重跑。嵌套状态用 `store`。值还是一份，但每条路径分开追踪。`#[derive(Store)]` 在 `view-macro` 后面，只支持具名字段、不带泛型的结构体。它生成 `AppStoreFields` 这样的 trait，用到字段访问器时要导入这个 trait。

```rust
use nana_ui::runtime::view::{Store, store};

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
        <Checkbox checked={todo.done()}></Checkbox>
    </Row>
});
```

```rust rust
use nana_ui::runtime::view::{checkbox, row, text};

todos.each(|todo| {
    row().gap(4).children((
        text(todo.title()),
        checkbox("").checked(todo.done()),
    ))
});
```

:::

`keyed` 的参数是函数指针 `fn(&T) -> K`，不能捕获环境。路径句柄和信号一样是 `Copy` 的 id，可以直接当属性。读写来自 `StorePath`：`get`、`with`、`try_with`、`set`、`update`。

`todos.at(&7).done().set(true)` 只更新读了这一格的绑定。`app.todos().push(todo)` 给列表加一行，已有行的内容不会被重读。对已经删掉的行 `get` 会 panic，`try_with` 返回 `None`。同一列表里两个 key 的 64 位哈希不能相同。

`store_with_history(value, 上限)` 才有 `undo` 和 `redo`。时间旅行的步进规则见 [声明式视图](../../reference/reactive-view.md)。

## provide 与 use_context

`provide(value)` 把一个值挂在当前作用域上。下层作用域，包括之后才建出来的行和分支，用 `use_context::<T>()` 沿父链读取，最近的提供者优先。同一个作用域里再 `provide` 同一个类型，会换掉上一个。`T` 必须是 `Clone + 'static`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::view::{use_context, IntoView, provide, signal};

fn page() -> impl IntoView {
    let user = signal(String::from("你"));
    provide(user);
    view! {
        <Column gap=8>
            {greeting()}
        </Column>
    }
}

fn greeting() -> impl IntoView {
    let user = use_context::<nana_ui::runtime::view::Signal<String>>().expect("page provides user");
    view! {
        <Text>"你好，{user}"</Text>
    }
}
```

```rust rust
use nana_ui::runtime::view::{use_context, IntoView, column, provide, signal, text};

fn page() -> impl IntoView {
    let user = signal(String::from("你"));
    provide(user);
    column().gap(8).children(greeting())
}

fn greeting() -> impl IntoView {
    let user = use_context::<nana_ui::runtime::view::Signal<String>>().expect("page provides user");
    text!("你好，{user}")
}
```

:::

只能在构建视图时读。事件处理器运行时不在任何作用域里，`use_context` 得到 `None`。

::: warning
在作用域之外调用 `provide` 什么也不做。事件处理里调用 `use_context` 也读不到。把信号放进 `provide`，子视图拿到的仍是那一个 `Signal`。
:::

从信号派生、以及会重复运行的 `watch_effect`，写在下一章。

## 接下来

<div class="next-steps">
  <a class="next-step" href="/guide/essentials/computed">
    <p class="next-step-link">计算值</p>
    <p class="next-step-caption">用 computed 派生，用 watch_effect 做副作用。</p>
  </a>
  <a class="next-step" href="/reference/reactive-view">
    <p class="next-step-link">声明式视图</p>
    <p class="next-step-caption">信号、Store 和刷新的完整合同。</p>
  </a>
</div>
