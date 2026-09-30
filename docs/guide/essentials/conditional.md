# 条件

要跟着数据出现或消失的一块界面，用 `v-if`。它展开成 `when`，不是建树时的那个 `if`。建树时的 `if` 只决定那一刻有没有这个节点。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::view::IntoView;

fn tasks(empty: nana_ui::runtime::view::Signal<bool>) -> impl IntoView {
    view! {
        <Column gap=8>
            <Text v-if={empty}>"还没有任务"</Text>
            <Text v-else>"有任务"</Text>
        </Column>
    }
}
```

```rust rust
use nana_ui::runtime::view::*;

fn tasks(empty: Signal<bool>) -> impl IntoView {
    column().gap(8).children(
        empty
            .then_show(|| text("还没有任务"))
            .otherwise(|| text("有任务")),
    )
}
```

:::

`v-if` / `v-else` 展开成 `when(条件, || 分支).otherwise(|| 另一支)`。`empty.then_show(|| …)` 就是 `when(empty, || …)`，名字不叫 `show`。`v-else-if` 嵌在 `otherwise` 里面：

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <Text v-if={loading}>"加载中"</Text>
    <Text v-else-if={failed}>"失败"</Text>
    <Text v-else>"完成"</Text>
}
```

```rust rust
use nana_ui::runtime::view::{text, when};

when(loading, || text("加载中"))
    .otherwise(|| when(failed, || text("失败")).otherwise(|| text("完成")))
```

:::

模板里写成相邻的三行：`v-if`、`v-else-if`、`v-else`。`v-else` 前面没有 `v-if` 是模板错误。

条件可以是 `Signal<bool>`、`Computed<bool>`，或一个返回 `bool` 的闭包。`list.with(Vec::is_empty)` 这种表达式在模板里会包成闭包。函数写法里写成 `move || list.with(Vec::is_empty)`，再 `.then_show`。

没显示的那一支默认不存在：节点拆掉，它自己的作用域回收，里面创建的信号和副作用一起丢掉。条件再变回来，会新建一支。挂载时已经显示的那一支不播进场。

没有 `otherwise` 时，条件为假就什么也不建。

## 藏起来，但留着节点

`v-show` 不拆节点，只切换 `layout.hidden`。节点留在树上，不参与布局、绘制和命中。

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <Text v-show={shown}>"仍然在树上"</Text>
}
```

```rust rust
use nana_ui::runtime::view::text;

text("仍然在树上").visible(shown)
```

:::

频繁开关、而且内部状态要留着的，用 `v-show`。整块换掉的，用 `v-if`。

`.keep_alive()` 走第三条路：没显示的分支连同节点留着，只是放进一个不参与布局、绘制和命中的容器，切回来还是原来的状态。模板里用 `<KeepAlive>` 包住一条 `v-if` 链。

::: warning
同一个元素不能同时写 `v-for` 和 `v-if`，这是模板错误。要滤掉几行，在 `each` 的数据里滤，或者让行视图自己 `v-if`。列表见下一章。
:::

`when` 和 `each` 各自带一个容器，默认是间距为 0 的一列 `Stack`。给这条链的容器加类，把链包在 `<Block class="…">` 里，或在函数写法里写 `when(…).class(…)`。类不会自动落到两个分支上。分支根自己的 `class` 仍然只属于那一支。

`when(…).transition(…)` 给进场和离场。离场的节点先留在原位，不参与命中，播完才销毁。那是进场离场，不是 `v-show`。

## 接下来

<div class="next-steps">
  <a class="next-step" href="/guide/essentials/list">
    <p class="next-step-link">列表</p>
    <p class="next-step-caption">v-for、each，以及每一行的 key。</p>
  </a>
  <a class="next-step" href="/guide/essentials/class-and-style">
    <p class="next-step-link">类与样式</p>
    <p class="next-step-caption">条件类 class:empty 怎样绑到同一张样式表。</p>
  </a>
</div>
