# 侦听

要在依赖变化时做一件事，而不是派生一个值，用 `watch_effect`。派生用 [计算值](computed.md)。

它马上跑一遍，记下读到的信号。这些信号以后变了，在下一次任意上下文的 flush 里再跑。返回值是 `Effect`，不是单独的 `Watch` 类型。它不接收旧值和新值，要比较就自己留一份。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::view::{IntoView, signal, watch_effect};

fn echoed_label() -> impl IntoView {
    let count = signal(0u64);
    let echoed = signal(String::new());
    watch_effect(move || {
        echoed.set(count.get().to_string());
    });
    view! {
        <Column gap=8>
            <Text>{echoed}</Text>
            <Button @activate={count.update(|c| *c += 1)}>"增加"</Button>
        </Column>
    }
}
```

```rust rust
use nana_ui::runtime::view::*;

fn echoed_label() -> impl IntoView {
    let count = signal(0u64);
    let echoed = signal(String::new());
    watch_effect(move || {
        echoed.set(count.get().to_string());
    });
    column().gap(8).children((
        text(echoed),
        button("增加").on_activate(move || count.update(|c| *c += 1)),
    ))
}
```

:::

这个副作用读 `count`，写的是另一个信号 `echoed`。它写在挂载闭包、某一行或某个分支里时，归那个作用域，作用域回收就停。想提前停，调用 `effect.dispose()`。回收作用域也会停掉它。

副作用互相触发，超过 64 轮还不停，会报 `runtime.reactive.did_not_settle`，剩下的队列被丢掉。不要在 `watch_effect` 里写它自己读的那个信号。怎么改见 [诊断码](../../reference/errors.md)。

`watch_effect` 的闭包是 `FnMut`，可以改自己捕获的状态。

## 和绑定的关系

节点上的动态字段共用一个副作用。任何一个输入变了，先按字段和保留的视图比较。全部相等就停，不复制，也不提交。`computed` 算出同一个值时，连这一步都不会开始。

刷新发生在读的时候，不在写的时候。没人读的 `computed` 就停在脏标记上。

## 接下来

<div class="next-steps">
  <a class="next-step" href="/guide/essentials/class-and-style">
    <p class="next-step-link">类与样式</p>
    <p class="next-step-caption">类名、构建时样式表，以及跟着主题走的表面。</p>
  </a>
  <a class="next-step" href="/guide/essentials/computed">
    <p class="next-step-link">计算值</p>
    <p class="next-step-caption">由别的信号派生一个值。</p>
  </a>
</div>
