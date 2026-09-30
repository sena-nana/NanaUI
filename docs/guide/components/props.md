# 属性

一个字段接受三种值：常量、信号（或计算值）、闭包。常量在建节点时写一次。信号和闭包之后跟着变。

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <Button disabled={move || draft.with(|text| text.is_empty())} loading={pending}>
        "保存"
    </Button>
}
```

```rust rust
use nana_ui::runtime::view::button;

button("保存")
    .disabled(move || draft.with(|text| text.is_empty()))
    .loading(pending)
```

:::

`pending` 是 `Signal<bool>` 或 `Computed<bool>`，直接交进去，只记下「信号 id + 这个字段的写入函数」，没有闭包。`disabled` 那段每次要看 `draft` 的内容，所以是闭包，装箱一次。写成 `loading={false}` 或 `.loading(false)` 就是常量，建完之后没有成本。

模板里能静态分开的只有这三类。字面量是常量。路径和字段（`pending`、`row.title`）原样传入。其他表达式包成 `move || 表达式`，读到的信号变了就重算。你自己写的 `disabled={|| …}` 会原样保留。

## 构造时读出来的是常量

在视图函数里写 `count.get()`，得到的是建树那一刻的值，不会跟着变。这和 Vue 在 `setup` 里读 `.value` 一样。要跟着变，把信号本身或闭包交给属性，不要先读出来。

:::api

```rust view
use nana_ui::runtime::view;

let count = signal(0u64);
view! {
    <Text>"{count}"</Text>
}
```

```rust rust
let count = signal(0u64);
// 这一行永远是建树时的数字
text(count.get().to_string());
// 这一行会跟着 count 变
text(move || count.get().to_string());
```

:::

`text!("{count}")` 是同一件事：插值读到的信号成为依赖。

## 控件表之外的字段

`button`、`slider`、`text_input` 这些函数的 setter 由控件表生成，每个对应一个 `FieldWrite`。表里没有的控件写成 `widget(组件)`，再用 `.bind(|组件| …)` 改它，或在你实现了 `FieldWrite` 之后用 `.prop::<T, W>(值)`。

`.bind` 看不出改了哪个字段，所以每次都按「有改动」处理，走复制和投影。能用单个 setter 时就用 setter：同一个节点上的动态字段共用一个副作用，逐字段比较，全部相等就停，不复制、不投影、不提交。

`Computed<T>` 和信号走同一条直接绑定。计算值只在结果和上次不同时，才让读它的字段重跑。

每个闭包装箱一次。函数写法里同一个节点上的两个闭包就是两次装箱。整段模板能看见时，同一节点的闭包才有机会合成一个副作用。

## 接下来

<div class="next-steps">
  <a class="next-step" href="/guide/components/events">
    <p class="next-step-link">事件</p>
    <p class="next-step-caption">激活、输入和提交分别怎么接。</p>
  </a>
  <a class="next-step" href="/guide/components/v-model">
    <p class="next-step-link">双向绑定</p>
    <p class="next-step-caption">哪些字段会写回信号。</p>
  </a>
  <a class="next-step" href="/guide/essentials/computed">
    <p class="next-step-link">计算值</p>
    <p class="next-step-caption">从信号派生一个可绑定的值。</p>
  </a>
</div>
