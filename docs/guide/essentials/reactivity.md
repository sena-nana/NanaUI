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

`T: PartialEq` 时，`set_if_changed(value)` 先不追踪地比较，值相等就什么都不做，不同才 `set`，返回是否写了。后台结果、尺寸回调这类经常带回同一个值的写入用它，绑定不会白跑一轮；在副作用里调用也不会让副作用订阅这个信号。`try_set_if_changed` 在信号已经不在时返回 `false`，不 panic，适合写回可能已被回收的行。

`get_untracked` 和 `with_untracked` 读，但不建立依赖。`untrack(|| …)` 包住一整段，里面的读取都不记。作用域已经回收后再读写，会报 `runtime.reactive.disposed_access`，同时 panic。`try_update` 在信号已经不在时返回 `false`，不 panic。

`set` 和 `update` 只把订阅者排进队列，不立刻改树。输入路由在每个事件末尾调用 `flush_reactive`，每帧开头也会再刷一次。你在 `RuntimeProgram::update` 里改信号，有待应用绑定的窗口也会被请求重绘。

属性可以接常量、`Signal<T>`、`Computed<T>`，或一个 `Fn() -> T`。常量在建节点时写进去。信号直接绑定，不装箱闭包。单独写在插值里的信号，例如 `"计数 {count}"`，读的时候自动成为依赖。

::: warning
在构造视图时直接 `count.get()`，读到的是当时的常量，之后不再跟着变。要跟着变，把信号本身传进去，或者传一个闭包。
:::

函数写法里 `.label(count.get())` 就是这种常量。模板里的 `{count.get()}` 是一段表达式，会包成 `move || count.get()`，那才会跟着变。能直接传信号时就传信号。

信号在挂载闭包、`each` 的一行或 `when` 的一个分支里创建，就归那个作用域。作用域回收时一起丢掉。在这些作用域之外创建的信号没有归属，会留到线程结束。列表里每一行要用的信号，写在行视图里。

## 嵌套状态

`Signal<App>` 里任何一处变了，读过它的地方都要重跑。嵌套状态用 `store`，每条路径分开追踪。写法在 [Store](../scaling/store.md)。

把一个值传给下层视图，用 [依赖提供](../components/provide.md)。从信号派生用 [计算值](computed.md)。依赖变化时做一件事，用 [侦听](watch.md)。

## 接下来

<div class="next-steps">
  <a class="next-step" href="/guide/essentials/computed">
    <p class="next-step-link">计算值</p>
    <p class="next-step-caption">用 computed 从信号派生一个值。</p>
  </a>
  <a class="next-step" href="/guide/essentials/watch">
    <p class="next-step-link">侦听</p>
    <p class="next-step-caption">依赖变化时做一件事。</p>
  </a>
  <a class="next-step" href="/reference/reactive-view">
    <p class="next-step-link">声明式视图</p>
    <p class="next-step-caption">信号、Store 和刷新的完整合同。</p>
  </a>
</div>
