# 列表

跟着 `Vec` 增删的行用 `v-for`。它展开成 `each`。key 还在的行留着，不重建；节点 id 和控件上的交互状态都不变。

```rust
#[derive(Clone)]
struct Todo {
    id: u64,
    title: String,
}
```

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::view::IntoView;

fn task_list(list: nana_ui::runtime::view::Signal<Vec<Todo>>) -> impl IntoView {
    view! {
        <Column gap=8>
            <Text v-for={todo in list} key={todo.id}>{todo.title}</Text>
        </Column>
    }
}
```

```rust rust
use nana_ui::runtime::view::*;

fn task_list(list: Signal<Vec<Todo>>) -> impl IntoView {
    column()
        .gap(8)
        .children(list.each(|todo| todo.id, |todo| text(todo.title)))
}
```

:::

`list.each(key, row)` 就是 `each(list, key, row)`。`list` 是 `Readable<Vec<T>>`，`Signal<Vec<T>>` 满足它。`T` 必须 `Clone`。`key` 是 `Fn(&T) -> K`，`K` 必须 `Eq + Hash + Clone + Send`。`row` 收到的是克隆出来的那一项，返回 `impl IntoView`。

模板里 `v-for` 必须写 `key`。不写是模板错误。函数写法里 key 也要给：没写 key 的位置名是每次建行时各数各的，不能用来找回那一行。

key 还在时，行内的字段变化应写在行自己的信号或 [Store](reactivity.md) 上，这样不会把整张列表重算一遍。行被删掉时，只回收这一行的作用域，并销毁它的节点。新行在一次单独的构建里挂上。

重复的 key 只保留第一个，后面的跳过。

::: warning
不要把 `v-for` 和 `v-if` 写在同一个元素上。那是模板错误，不会变成「先过滤再循环」。
:::

## 容器

`each` 的行放在一个容器里，默认是间距为 0 的一列。行距用 `.gap(8)`。要横着排，用 `.horizontal(8)`。

```rust
use nana_ui::runtime::view::each;

each(list, |todo| todo.id, |todo| text(todo.title)).gap(8)
```

模板里给这个容器加类，用 `<Block class="strip">` 包住那一个 `v-for` 元素。`v-for` 元素自己的 `class` 落在每一行上，不落在容器上。

静态的几个子节点不要用 `each`。个数写死就用 tuple。建树时的 `for` 写在 `.with(|c| { … })` 里，它只跑一遍，不跟着后来的 `push` 变。

## key 还用在查找上

静态节点不写 key 时按位置命名，`#v0`、`#v1` 这样。需要 `resolve_assembly_path` 按名字找到它时，才在节点上写 `.key("title")`。`each` 里每一行是自己的范围：不同行可以写同样的 key，互不覆盖。挂载之后才加进来的行，同样登记在这个容器下面。

`Store` 的列表用 `keyed(|item| item.id).each(|item| …)`，key 已经在 `keyed` 里，行闭包只负责视图。见 [响应式](reactivity.md)。

很长的列表不要一次建出每一行。`each_virtual` 只建视口盖到的行，合同在 [声明式视图](../../reference/reactive-view.md)。这一章的 `each` 会把现有的行都留在树上。

## 接下来

<div class="next-steps">
  <a class="next-step" href="/guide/essentials/events">
    <p class="next-step-link">事件</p>
    <p class="next-step-caption">无参的 on_activate，以及带事件的 on_input。</p>
  </a>
  <a class="next-step" href="/guide/essentials/conditional">
    <p class="next-step-link">条件</p>
    <p class="next-step-caption">空列表时用 when 换一块提示，而不是写在 v-for 上。</p>
  </a>
</div>
