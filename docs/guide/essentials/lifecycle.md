# 挂载与卸载

`mount_view` 和 `mount_view_root` 接收一个闭包。视图表达式在这个闭包里求值，一次提交建完。闭包里创建的信号、`computed`、`watch_effect` 和 `provide`，都归这次挂载的作用域。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::view::{node_ref, on_cleanup, on_mount, IntoView};
use nana_ui::runtime::DocumentId;

fn search(document: DocumentId) -> impl IntoView {
    let input = node_ref();
    on_mount(move |cx| {
        if let Some(id) = input.get_untracked() {
            let _ = cx.focus_node(document, id);
        }
    });
    on_cleanup(|| {
        // 作用域回收时运行。此时不要再读这个作用域里的信号。
    });
    view! {
        <Column gap=8>
            <Text>"搜索"</Text>
            <TextInput placeholder="关键字" ref={input} />
        </Column>
    }
}
```

```rust rust
use nana_ui::runtime::view::{column, node_ref, on_cleanup, on_mount, text, text_input, IntoView};
use nana_ui::runtime::DocumentId;

fn search(document: DocumentId) -> impl IntoView {
    let input = node_ref();
    on_mount(move |cx| {
        if let Some(id) = input.get_untracked() {
            let _ = cx.focus_node(document, id);
        }
    });
    on_cleanup(|| {
        // 作用域回收时运行。此时不要再读这个作用域里的信号。
    });
    column().gap(8).children((
        text("搜索"),
        text_input().placeholder("关键字").node_ref(input),
    ))
}
```

:::

挂到父节点下用 `mount_view(parent, || search(document))`。根就是文档根时用 `mount_view_root(document, || search(document))`。先建出来、先不放进树，用 `mount_view_detached`。三种都把闭包放进同一个作用域规则里。

## 挂载没有留下半棵树

建树自己报错时，变更队列不会提交，作用域随即回收。根插到父节点下面失败时，先回收作用域，再把已经建出的根拆掉。放置槽位失败，或者 `with_refs` 里有句柄没有对上这次建出的元素时，树已经立着，于是走 `unmount`：先回收作用域，再拆掉各个根。后一种返回 `FrameworkError::InvalidInput`。

插到某个父节点下面的根不带 key。父节点自己按 key 排好的子节点不会因此重新组装。

父节点已经不在树上时，`mount_view` 返回 `FrameworkError::MissingView`，闭包不会运行。`mount_view_root` 和 `mount_view_detached` 则把根放在你传入的那份 `DocumentId` 上。

`on_mount` 在视图进树之后跑一次：挂载的那次提交之后，或者 `each` 的一行、`when` 的一支被放进去之后。它不追踪读取。参数是放下这棵视图的 `&mut AppContext`。上面的 `node_ref` 到这时已经有节点 id。`focus_node` 在节点不可聚焦时返回 `Ok(false)`，不会 panic。挂载失败、作用域先被回收时，这次回调直接丢掉，不会对着半棵树跑。

`on_cleanup` 登记在当前作用域上，作用域被回收时运行。闭包是 `FnOnce()`。

::: warning
在作用域之外调用 `on_cleanup` 什么也不做。事件处理器里不在作用域中，在那里登记的清理不会跑。`on_mount` 要写在构建视图的时候。
:::

## 谁和谁一起回收

| 时机 | 回收什么 |
| --- | --- |
| `MountedView::unmount` | 这个挂载的作用域，然后拆掉各个根 |
| 拆掉根节点 | 绑在它上面的绑定、结构副作用，以及锚定在它身上的作用域 |
| `each` 删掉一行、`when` 切走一支 | 只回收那一行、那一支的子作用域 |
| `AppContext` 被丢弃 | 还留在这个上下文上的视图作用域 |

丢掉 `MountedView` 这个句柄不会卸载。树还在，绑定也还在。要拆掉，调用 `mounted.unmount(&mut cx)`。它先 `dispose` 作用域，再 despawn 各个根。

一行或一支自己的 `on_cleanup` 在它被拿掉时就跑，不等整页卸载。离场动画还在播的节点，作用域先回收，节点播完才销毁。清理里不要再 `get` 这个作用域的信号。

作用域之外创建的信号没有归属，会留到线程结束。事件里临时 `signal(…)` 就是这种。列表每一行要用的信号，写在行视图里，让那一行的子作用域带着走。

`provide` 同样挂在当前作用域。行和分支看得到父链上的值。作用域回收之后，这个提供者也不在了。

## 建树和更新是两回事

挂载闭包只跑一次。里面的 `if`、`for`、`.with` 决定的是这一次的结构。之后改信号，只会跑读过它的绑定、`watch_effect`，以及 `each` / `when` 的结构更新。不要在点击里再 `mount_view` 一遍同一块界面。

`flush_reactive` 由输入路由在事件末尾调用，每帧开头也会再来一次。你不用在 `on_mount` 里手动刷。`on_mount` 里改信号，和其他写信号一样，排到下一次 flush。

作用域回收之后再读写其中的信号，会报 `runtime.reactive.disposed_access`，同时 panic。清理和稍后才回来的任务，用 `try_update`：信号还在才改，已经不在就返回 `false`。

## 接下来

<div class="next-steps">
  <a class="next-step" href="/guide/components/registry">
    <p class="next-step-link">扩展控件</p>
    <p class="next-step-caption">内置标签不够时，怎样登记自己的控件。</p>
  </a>
  <a class="next-step" href="/reference/reactive-view">
    <p class="next-step-link">声明式视图</p>
    <p class="next-step-caption">作用域、刷新和卸载的完整合同。</p>
  </a>
</div>
