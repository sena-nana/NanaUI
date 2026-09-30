# Suspense

`suspense` 在内容第一次准备好之前显示回退。内容会立刻建好，只是先藏起来。里面创建的每一个 `resource` 都完成了第一次加载，回退才撤掉。之后的重新加载留着旧内容，不再切回去。

加载本身见 [异步](../components/async.md)。`profile` 在内容构建时创建 `resource`。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::view::text;

view! {
    <Suspense fallback={text("加载中…")}>
        <Profile id={id} />
    </Suspense>
}
```

```rust rust
use nana_ui::runtime::view::{suspense, text};

suspense(|| text("加载中…"), move || profile(id))
```

:::

回退是内容构建期间 `pending` 还大于 0 时显示的视图。模板里用 `fallback={…}`。`.vue` 里写成 `<template #fallback>`，那是插槽，展开出来不是一个叫 `fallback` 的方法。`<Suspense>` 只有这一个属性。

`pending` 在构建内容时，每创建一个 `resource` 加一，那一次 fetch 第一次完成时减一。`refetch` 不再加。内容里一个 `resource` 都没创建，回退不会出现，所以 `resource` 要写在内容会执行的函数里。几个 `resource` 写在同一份内容里时，要等它们全部第一次完成。

内容闭包是 `FnOnce`。回退闭包要 `Send`。中途卸载这段视图，还没完成的任务会 `abort`，还占着的 `pending` 也减掉，回退一起消失。

## 回退和内容同时在树上

内容放进一列，用 `visible` 藏到 `pending` 回到 0。回退是旁边的 `when`，条件是 `pending > 0`。第一次完成后回退拆掉，内容不用重建，只是从隐藏变成可见。

后来的 `refetch` 不再增加 `pending`，所以这次 `when` 也不会再打开。

`provide(SuspenseContext)` 发生在内容闭包里面。写在 `suspense` 前面的 `resource` 看不到这份上下文，也不会让这份回退出现。要让回退等它，把 `resource` 放进内容会调用的函数。

## 重新加载不回到回退

`resource` 在重新 fetch 时保留上一次的值，并用 `loading()` 表示正在加载。`suspense` 只看「有没有过第一次结果」。你要在刷新时换成另一段界面，就读 `loading()`，自己写 `when`，不要指望边界再显示回退。

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <Text v-if={move || user.loading()}>"正在刷新…"</Text>
    <Text v-else>{name}</Text>
}
```

```rust rust
use nana_ui::runtime::view::{text, when};

when(move || user.loading(), || text("正在刷新…")).otherwise(|| text(name))
```

:::

乐观更新用 `user.set(新值)`。已经发出的 fetch 完成后仍会覆盖它。这和回退无关：内容本来就在显示。

没有宿主轮询时，第一次加载不会完成，回退会一直留着。窗口宿主会调用 `set_task_wake` 并 `poll_tasks()`。测试和嵌入要自己轮询。见 [异步](../components/async.md)。

## 接下来

<div class="next-steps">
  <a class="next-step" href="/guide/components/async">
    <p class="next-step-link">异步</p>
    <p class="next-step-caption">resource、spawn_blocking 和线程。</p>
  </a>
  <a class="next-step" href="/guide/scaling/sfc">
    <p class="next-step-link">.vue 方言</p>
    <p class="next-step-caption">同一套调用，写成单文件组件。</p>
  </a>
  <a class="next-step" href="/guide/built-ins/keep-alive">
    <p class="next-step-link">KeepAlive</p>
    <p class="next-step-caption">切走页面时把已经加载的状态留下。</p>
  </a>
</div>
