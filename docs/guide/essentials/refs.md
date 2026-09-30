# 句柄

视图建完之后，有时你要按类型改那一个控件，或者把节点 id 交给别的接口。用 `entity_ref` 记下 `Entity<控件>`，用 `node_ref` 记下节点 id。两者都是信号，在元素建成时写入。

```rust
use nana_ui::runtime::view::{button, column, entity_ref, text, with_refs};
use nana_ui::runtime::{Button, Text};

let (mounted, (label, action)) = cx.mount_view_root(document_id, || {
    let label = entity_ref::<Text>();
    let action = entity_ref::<Button>();
    with_refs(
        column().gap(8).children((
            text("就绪").entity_ref(label),
            button("开始").entity_ref(action),
        )),
        (label, action),
    )
})?;
```

`entity_ref::<Text>()` 得到 `EntityRef<Text>`。`.entity_ref(label)` 在这个 `Text` 建成时把实体写进去。`with_refs(视图, 句柄)` 让挂载的返回值变成 `(MountedView, 解析后的句柄)`：`EntityRef<C>` 变成 `Entity<C>`，`NodeRef` 变成 `StableNodeId`。元组、数组和 `Vec` 按原样逐项解析。

句柄要在挂载闭包里面创建，才归这次挂载。上面的 `label` 和 `action` 在闭包里，卸载时和别的信号一起回收。

类型必须对上。`El<Text>` 只接受 `EntityRef<Text>`。写成 `EntityRef<Button>` 编译不过。一个元素可以同时填几个句柄，谁也不覆盖谁：`.entity_ref(label).node_ref(slot)`。

::: warning
`with_refs` 里的每个句柄都必须指向这次建出来的元素。指到当时没有建出的分支（例如条件为假的 `when`）时，挂载返回 `FrameworkError::InvalidInput`，这次什么也不留。
:::

## 不经过 with_refs

`EntityRef::get` 在建成之后返回 `Some(Entity<C>)`，还没建是 `None`。这次读取不追踪，不会变成绑定。`NodeRef` 就是 `Signal<Option<StableNodeId>>`。要 id 时用 `get_untracked`，避免在别的绑定里订上它。

模板属性 `ref={input}` 展开成 `.node_ref(input)`。`ref="input"` 是 `.vue` 里按名字取同一个 `NodeRef`。`entity_ref` 没有模板属性。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::view::{node_ref, IntoView};

fn field() -> impl IntoView {
    let input = node_ref();
    view! {
        <TextInput placeholder="关键字" ref={input} />
    }
}
```

```rust rust
use nana_ui::runtime::view::{node_ref, text_input, IntoView};

fn field() -> impl IntoView {
    let input = node_ref();
    text_input().placeholder("关键字").node_ref(input)
}
```

:::

`EntityRef::node_ref()` 把同一个引用再看成 `NodeRef`，给只收节点 id 的接口，例如传送的目标。

挂载返回的 `MountedView` 还可以 `root::<C>()`。视图只有一个根、而且你知道它的类型时，直接得到 `Entity<C>`。多个根用 `roots()` 拿 `&[StableNodeId]`。

## 拿到之后

`Entity<C>` 交给 `AppContext::update_component`。上面的 `cx` 就是 `RuntimeDocument::context_mut()` 拿到的这个上下文。闭包收到 `&mut C` 和 `ViewContext`。组件和原来相等、又没有发出事件或改动树时，这次更新直接返回，不投影也不提交。

```rust
cx.update_component(label, |text, _| {
    text.value = "已开始".to_string();
})?;
```

不要在这个闭包里写 `*text = Text::new("已开始")`。那会连交互状态一起盖掉。整份换上用 `set_component(label, Text::new("已开始"))`，由控件自己的 `reconcile` 决定什么留下。

视图已经在用信号绑定的字段，优先改信号。句柄是给挂载之后仍要按实体操作的代码：程序保存的标签、传送目标、焦点。点击里整页再挂载一次，见 [创建应用](application.md) 里的那条合同。

## 接下来

<div class="next-steps">
  <a class="next-step" href="/guide/essentials/view-functions">
    <p class="next-step-link">视图函数</p>
    <p class="next-step-caption">把一块界面收成返回 impl IntoView 的函数。</p>
  </a>
  <a class="next-step" href="/guide/essentials/lifecycle">
    <p class="next-step-link">挂载与卸载</p>
    <p class="next-step-caption">句柄属于哪个作用域，卸载时怎样回收。</p>
  </a>
</div>
