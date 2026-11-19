# KeepAlive

`when` 默认会拆掉没显示的分支。`.keep_alive()` 把它连同节点和状态留下，切回来时原样出现。列表行不需要这个：`each` 已经按 key 保留行。

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <KeepAlive>
        <TextInput v-if={editing} v-model={draft} />
        <Text v-else>{draft}</Text>
    </KeepAlive>
}
```

```rust rust
use nana_ui::runtime::view::{text, text_input, when};

when(editing, || text_input().model(draft))
    .otherwise(|| text(draft))
    .keep_alive()
```

:::

`<KeepAlive>` 包住一条 `v-if` / `v-else-if` / `v-else` 链。除了 `class`，它没有属性。藏起来的分支移进一个脱离父容器子节点列表的停靠隐藏 `Stack`：不占用宿主子节点位置，也不参与布局、绘制和命中，焦点会移走。声明宿主销毁时，留下的分支和停靠节点一起销毁。

和 `<Transition>` 套在一起时，保活的分支切走不播离场，切回来时播进场。见 [Transition](https://github.com/sena-nana/NanaUI/blob/main/docs/guide/built-ins/transition.md)。

## 按 key 留住几页

页签不是真和假两条分支。`dynamic(key, 渲染)` 按 key 显示一个视图，对应 Vue 的 `<component :is>`。`.keep_alive()` 保留之前每个 key 的视图，`.max(n)` 最多留 n 个没在显示的，先丢掉最久没显示的。

```rust
use nana_ui::runtime::view::dynamic;

dynamic(tab, |tab| match tab {
    Tab::Notes => notes(),
    Tab::Preview => preview(),
})
.keep_alive()
.max(2)
```

上限只在函数写法里。模板的 `<KeepAlive>` 没有 `max`。包住 `v-for` 也会报错：列表行由自己的 key 保留，不是由这条链保留。

留下的分支作用域不回收，节点停在那个隐藏的停靠 `Stack` 里。切回来会插回声明位置显示原来的节点。

`when(…).keep_alive()` 最多留一条没在显示的分支。`v-if` 配 `v-else` 正好是这一条。链上还有 `v-else-if` 时，更早离开的分支会被丢掉，只留最近藏起来的那一支。要按 key 留多页，用 `dynamic`。

`<KeepAlive class="panel">` 上的类会为这条链显式建出一个包装容器并落到它上面；函数写法在 `keep_alive()` 后面写 `.class`。没有容器样式或 `.key` 等选项时，它本身无盒，不占额外 `gap`。

## 接下来

<div class="next-steps">
  <a class="next-step" href="/guide/built-ins/teleport">
    <p class="next-step-link">Teleport</p>
    <p class="next-step-caption">内容留在声明处，画到另一层去。</p>
  </a>
  <a class="next-step" href="/guide/built-ins/suspense">
    <p class="next-step-link">Suspense</p>
    <p class="next-step-caption">等待第一次加载时显示的回退。</p>
  </a>
  <a class="next-step" href="/guide/built-ins/transition">
    <p class="next-step-link">Transition</p>
    <p class="next-step-caption">切回来时怎样播进场。</p>
  </a>
</div>
