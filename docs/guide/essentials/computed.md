# 计算值

一个值完全由别的信号算出来时，用 `computed`。它归当前作用域，创建时就算一遍。之后依赖变了，它只标脏，下次被读时才重算。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::view::{IntoView, computed, signal};

fn summary() -> impl IntoView {
    let count = signal(0u64);
    let label = computed(move || match count.get() {
        0 => "还没有".to_string(),
        n => format!("共 {n} 项"),
    });
    view! {
        <Column gap=8>
            <Text>{label}</Text>
            <Button @activate={count.update(|c| *c += 1)}>"增加"</Button>
        </Column>
    }
}
```

```rust rust
use nana_ui::runtime::view::*;

fn summary() -> impl IntoView {
    let count = signal(0u64);
    let label = computed(move || match count.get() {
        0 => "还没有".to_string(),
        n => format!("共 {n} 项"),
    });
    column().gap(8).children((
        text(label),
        button("增加").on_activate(move || count.update(|c| *c += 1)),
    ))
}
```

:::

`computed` 要求 `T: PartialEq + 'static`。重算结果和上一次相等时，读它的绑定不会再跑。所以 `computed(|| count.get() % 2)` 从 `1` 变到 `3`，显示这个奇偶的地方不动。

`Computed<T>` 也是 `Copy`。读取用 `get` 或 `with`，和信号一样会追踪。它可以再传给属性，也可以再被别的 `computed` 读。闭包里用 `untrack` 包住的读取不算依赖。

只是把信号格式化一下、不必单独缓存时，用 `Signal::map`。`text(count.map(|n| format!("共 {n} 项")))` 得到一个闭包属性，每次绑定运行时现算，没有 `PartialEq` 那一层截断。

::: warning
在构造视图时直接 `label.get()`，写进节点的是当时的字符串，之后不再跟着变。把 `label` 本身传给 `text`，或者传闭包。
:::

## watch_effect

要在依赖变化时做一件事，而不是派生一个值，用 `watch_effect`。它马上跑一遍，记下读到的信号；这些信号以后变了，在下一次任意上下文的 flush 里再跑。返回值是 `Effect`，不是单独的 `Watch` 类型。

```rust
use nana_ui::runtime::view::{signal, watch_effect};

let count = signal(0u64);
let echoed = signal(String::new());
let effect = watch_effect(move || {
    echoed.set(count.get().to_string());
});
```

这个副作用读 `count`，写的是另一个信号 `echoed`。它写在挂载闭包、某一行或某个分支里时，归那个作用域，作用域回收就停。想提前停，调用 `effect.dispose()`。回收作用域也会停掉它。

副作用互相触发，超过 64 轮还不停，会报 `runtime.reactive.did_not_settle`，剩下的队列被丢掉。不要在 `watch_effect` 里写它自己读的那个信号。

`watch_effect` 的闭包是 `FnMut`，可以改自己捕获的状态。它不接收旧值和新值，你要比较就自己留一份。

## 和绑定的关系

节点上的动态字段共用一个副作用。任何一个输入变了，先按字段和保留的视图比较。全部相等就停，不复制，也不提交。`computed` 算出同一个值时，连这一步都不会开始。

刷新发生在读的时候，不在写的时候。没人读的 `computed` 就停在脏标记上。

## 接下来

<div class="next-steps">
  <a class="next-step" href="/guide/essentials/class-and-style">
    <p class="next-step-link">类与样式</p>
    <p class="next-step-caption">类名、构建时样式表，以及跟着主题走的表面。</p>
  </a>
  <a class="next-step" href="/guide/essentials/reactivity">
    <p class="next-step-link">响应式</p>
    <p class="next-step-caption">信号本身怎么读、怎么写。</p>
  </a>
</div>
