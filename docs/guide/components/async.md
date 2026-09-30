# 异步

信号只能在创建它的线程上用。要写信号的异步代码就留在 UI 线程上：阻塞的那一段放到 `spawn_blocking`，结果交给 `resource`。第一次完成前要显示别的内容，用 `suspense`。

`load_user` 是你的函数，不在框架里。它要在 `suspense` 的内容构建时才调用，所以放在内容会执行的函数里。写在 `suspense` 外面的 `resource` 不会让这份回退出现。

```rust
use nana_ui::runtime::view::{resource, spawn_blocking, text, IntoView, Signal};

fn profile(id: Signal<u64>) -> impl IntoView {
    let user = resource(move || id.get(), |id| spawn_blocking(move || load_user(id)));
    text(move || user.with(|user| user.map_or(String::new(), |user| user.name.clone())))
}
```

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

`<Profile id={id} />` 展开成 `profile(id)`。内容立刻建好，先藏起来，直到里面创建的 `resource` 第一次完成。之后的重新加载不再切回「加载中…」。模板的 `fallback` 和 `.vue` 里的 `<template #fallback>` 见 [Suspense](../built-ins/suspense.md)。

## `resource` 记住最近一次结果

`source` 像副作用一样被追踪。它每次变化都调用 `fetch(source)`，还没完成的上一次会被丢掉。

- `get()` / `with()` 读最近一次结果。第一次完成前是 `None`。重新加载时保留旧值。
- `loading()` 表示是不是正在加载。
- `refetch()` 用来源的当前值再加载一次。
- `set(值)` 直接改结果，用来做乐观更新。已经发出的那次 fetch 完成后仍会写上它的值。

`spawn_blocking` 把闭包放到新线程上，返回的 future 带着结果。闭包必须是 `Send`。`spawn_local` 在 UI 线程的执行器上跑一个 future，不要求 `Send`，可以直接读写信号。它返回的 `Task` 可以 `abort()`。建树时创建的任务归这个视图，卸载时一起丢掉。

在别的线程上使用信号会 panic。句柄是 `Copy` 的 id，可以放进 `Send` 的处理器，但使用必须回到创建它的线程。

## 宿主要轮询

waker 在任何线程上被唤醒，都会通过宿主的唤醒钩子把事件循环叫起来，再到 UI 线程上 `poll_tasks()`。每帧开头也会轮询一次。`nana-ui` 的窗口宿主在启动时调用 `set_task_wake`，并在处理宿主工作时轮询、给有待应用绑定的窗口请求重绘。

没有宿主时（测试、嵌入）要自己调用 `poll_tasks()`。没人轮询，future 就停在那里。

## 接下来

<div class="next-steps">
  <a class="next-step" href="/guide/built-ins/suspense">
    <p class="next-step-link">Suspense</p>
    <p class="next-step-caption">回退显示到什么时候，重新加载会不会再切回去。</p>
  </a>
  <a class="next-step" href="/guide/scaling/fetch">
    <p class="next-step-link">Fetch</p>
    <p class="next-step-caption">页面请求和 url() 共用的宿主白名单。</p>
  </a>
  <a class="next-step" href="/guide/built-ins/transition">
    <p class="next-step-link">Transition</p>
    <p class="next-step-caption">加载完的内容怎么进场。</p>
  </a>
</div>
