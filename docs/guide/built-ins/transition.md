# Transition

`when` 和 `each` 可以带一段进出场。新的分支或新行从给定的透明度和变换播到节点自己的值。被拿掉的不立刻销毁。这就是 Vue 的 `<Transition>` 和 `<TransitionGroup>`。`TransitionGroup` 不另开一页。

挂载时已经在树上的行不播进场。

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <Transition name="fade" duration="150">
        <Text v-if={open}>"已打开"</Text>
    </Transition>
}
```

```rust rust
use std::time::Duration;

use nana_ui::runtime::view::{text, when, Transition};

when(open, || text("已打开")).transition(Transition::fade(Duration::from_millis(150)))
```

:::

`<Transition>` 包住一条 `v-if` / `v-else-if` / `v-else` 链。`name` 可以是 `fade`、`slide-up`、`slide-down`、`slide-left`、`slide-right`、`scale`，不写就是 `fade`。`duration` 的单位是毫秒，不写是 150。`transition={值}` 直接传入一个 `Transition`，这时不要再写 `name`。

预设对应的函数是 `Transition::fade`、`slide(dx, dy, 时长)`、`scale(比例, 时长)`。`slide-up` 是 `slide(0.0, 12.0, 时长)`，`scale` 是 `scale(0.95, 时长)`。自己拼用 `Transition::new().enter(Presence::new(时长).opacity(0.0).translate(0.0, 8.0)).leave(…)`。`.ease(缓动)` 统一这一段的缓动，默认是 `Easing::EaseOutCubic`。只要一半时用 `without_enter()` 或 `without_leave()`。

## 离场留在原地

被删掉的行或分支马上回收作用域，不再跟随数据。节点留在原位，不参与命中，焦点移走，播完才销毁。

`when` 的新旧分支会同时存在：旧的在原位离场，新的接在后面进场。加上 `.moves` 后，旧分支消失时新分支平滑上移。Vue 的 `mode="out-in"`（先离场再进场）目前没有。

动画走节点的合成器轨道，不写回逻辑样式。离场结束由 `advance_animations` 的完成事件驱动。移动在布局之后、同一帧提取之前开始，所以不会先闪到新位置。

## TransitionGroup

列表用 `<TransitionGroup>`，或在 `each` 上加同一个 `.transition`。`move` 是位置变化时的滑动时长（FLIP）。插入、删除、重排都算，离场的行最终销毁、后面的行补位时也算。

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <TransitionGroup duration="180" move="200">
        <Text v-for={item in items} key={item.id}>{item.title}</Text>
    </TransitionGroup>
}
```

```rust rust
use std::time::Duration;

use nana_ui::runtime::view::{each, text, Transition};

each(items, |item| item.id, |item| text(item.title.clone())).transition(
    Transition::fade(Duration::from_millis(180)).moves(Duration::from_millis(200)),
)
```

:::

`<Transition>` 和 `<TransitionGroup>` 可以互换：一个包链，一个包 `v-for`，生成的是 `.transition`，有 `move` 再加 `.moves`。`Store` 的 `keyed(..).each` 同样可以接 `.transition`。

虚拟列表（`each_virtual`）不支持。滚出视口的行本来就要立刻回收，包上 `<Transition>` 会在编译模板时报错。

类名可以写在这两个标签上，落在它们包住的那一列容器上，不是每一行。见 [声明式视图](../../reference/reactive-view.md) 的进出场一节。

## 接下来

<div class="next-steps">
  <a class="next-step" href="/guide/built-ins/keep-alive">
    <p class="next-step-link">KeepAlive</p>
    <p class="next-step-caption">切走的分支如何留着，而不是播完就销毁。</p>
  </a>
  <a class="next-step" href="/guide/built-ins/teleport">
    <p class="next-step-link">Teleport</p>
    <p class="next-step-caption">把一段视图挂到树的另一处。</p>
  </a>
  <a class="next-step" href="/guide/essentials/conditional">
    <p class="next-step-link">条件</p>
    <p class="next-step-caption">v-if 怎样展开成 when。</p>
  </a>
</div>
