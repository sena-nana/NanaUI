# 视图写法

`view!` 和函数调用是两种一级写法。它们展开成同一套调用，例如 `column().gap(8)`、`.class(..)` 和 `each(..)`。树在挂载时建一次，之后每个绑定只更新它自己写的那个字段。

视图层默认就会编译进来。`view!` 放在 `view-macro` 这个 feature 后面。宏的路径是 `nana_ui::runtime::view`。它和同名模块一个在宏命名空间，一个在类型命名空间，所以下面两行可以同时写：

```rust
use nana_ui::runtime::view;
use nana_ui::runtime::view::{button, column, text};
```

## 一个计数器

下面是同一个计数器。

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

`<Text>"计数 {count}"</Text>` 展开成 `text!("计数 {count}")`。没有插值的字符串展开成 `text("…")`。

`@activate={表达式}` 展开成 `.on_activate(move || { 表达式; })`。如果 `add` 已经是一个函数，`@activate={add}` 会直接把这个函数传进去。

## 子节点

个数写死的几个子节点，用一个 tuple：`.children((a, b))`。这一组子节点装箱一次。

如果建树的时候要用 `for`、`if` 或 `let`，写成 `.with(|c| { c.add(a); })`。这个块在建树时只跑一次，里面的分支决定的是那一刻的结构。要跟着数据变的结构，用 `each` 或 `when`。

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <Column gap=8>
        <TextInput placeholder="新任务" v-model={draft} />
        <Button @activate={add}>"添加"</Button>
        <Text v-for={t in list} key={t.id}>{t.title}</Text>
        <Text v-if={list.with(Vec::is_empty)}>"还没有任务"</Text>
        <Text v-else>"有任务"</Text>
    </Column>
}
```

```rust rust
use nana_ui::runtime::view::*;

column().gap(8).with(|c| {
    c.add(text_input().placeholder("新任务").model(draft));
    c.add(button("添加").on_activate(add));
    c.add(list.each(|t| t.id, |t| text(t.title)));
    c.add(
        list.with(Vec::is_empty)
            .then_show(|| text("还没有任务"))
            .otherwise(|| text("有任务")),
    );
})
```

:::

## 属性

属性可以是常量、`Signal<T>`、`Computed<T>`，或 `Fn() -> T` 闭包。常量在建节点时写进去。信号直接绑定。闭包装箱一次。

::: warning
在构造视图时直接 `count.get()`，读到的是当时的常量，之后不再跟着变。要跟着变，把信号本身传进去，或者传一个闭包。
:::

类名写错，在 `view!` 里是编译错误。函数写法里，类名同样在编译期对上样式表里的名字。

## 对照

宏展开出来的就是右边这一列。数字字面量在宏里会带上类型后缀。手写时 `gap` 接受任何 `Px`，写成 `gap(8)` 即可。

| 模板 | 函数写法 |
| --- | --- |
| `<Column gap=8>` / `<Row>` | `column().gap(8).children((…))` / `row()` |
| `<Text>"计数 {count}"</Text>` | `text!("计数 {count}")` |
| `<Button>"保存"</Button>` | `button("保存")` |
| `<TextInput/>` | `text_input()` |
| `@activate={save}` | `.on_activate(save)` |
| `v-model={sig}` | `.model(sig)` |
| `v-for={t in list} key={t.id}` | `list.each(\|t\| t.id, \|t\| …)` |
| `v-if` / `v-else` | `when(…)`，或 `cond.then_show(…).otherwise(…)` |
| `v-show={x}` | `.visible(x)` |
| `class="a"` / `class:empty={c}` | `.class(s::a)` / `.class_when(s::a, c)` |

`v-for` 和 `v-if` 不能写在同一个元素上，这是模板错误。`v-for` 要带 `key`。不认识的标签会按 snake_case 去调用同名函数。

信号、Store、样式表和 `.vue` 方言的全文在 [声明式视图](../../reference/reactive-view.md)。计算值、条件和列表会在后面的章节分开写。
