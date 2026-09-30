# Teleport

有些内容要写在按钮旁边，却画在浮层里。`teleport` 在声明处构建这段视图，树里的父节点换成你给的目标。布局、绘制、命中、焦点顺序和无障碍都按目标的位置。作用域仍归声明处，声明处销毁，内容一起销毁。

`layer` 是一个 `node_ref()`，先交给目标节点。侧边栏顶部的开关只换被传走的那一段。

```rust
use nana_ui::runtime::view::{column, node_ref, widget};
use nana_ui::runtime::Stack;

let layer = node_ref();
```

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <Column gap=8>
        <Widget of={Stack::column(0.0)} ref={layer} />
        <Teleport to={layer}>
            <Text>"在浮层里"</Text>
        </Teleport>
    </Column>
}
```

```rust rust
use nana_ui::runtime::view::{column, teleport, text, widget};
use nana_ui::runtime::Stack;

column().gap(8).children((
    widget(Stack::column(0.0)).node_ref(layer),
    teleport(layer, text("在浮层里")),
))
```

:::

`to` 可以是 `NodeRef`、节点 id，或选出 `Option<StableNodeId>` 的闭包。它变了，内容跟着搬。值为 `None`，或目标不在树上，内容就留在声明处的锚点里。锚点是一列 `Stack::column(0)`。底层是 `place_assembled`。

模板里记下目标用 `ref={layer}`，展开成 `.node_ref(layer)`。`layer` 在挂载之后才有 id。传送发生在这段内容进树的时候。

锚点接受 `.class`、`.class_when` 和 `.css`。模板写在 `<Teleport to={layer} class="fill">` 上，让还没传走的内容撑满高度。目标后来出现，内容再搬过去。

内容的作用域归声明处。声明处销毁，当时画在目标下面的内容一起销毁。窗口仍由宿主打开。传送只改这棵树里的父节点。

## 锚点上的类

`.class`、`.class_when`、`.css` 写在 `teleport(…)` 的返回值上，或写在 `<Teleport>` 标签上。它们装饰内容建在里面的那一列。内容已经挂到目标下面之后，这列锚点可以是空的。

`to` 用闭包时，闭包选出的 id 变了就再搬一次。这次求值时目标已经不在树上，内容回到锚点。

## 接下来

<div class="next-steps">
  <a class="next-step" href="/guide/built-ins/suspense">
    <p class="next-step-link">Suspense</p>
    <p class="next-step-caption">第一次加载完成前显示的回退。</p>
  </a>
  <a class="next-step" href="/guide/components/slots">
    <p class="next-step-link">具名 slot</p>
    <p class="next-step-caption">对话框自己的槽，不是传送到别处。</p>
  </a>
  <a class="next-step" href="/guide/essentials/refs">
    <p class="next-step-link">句柄</p>
    <p class="next-step-caption">node_ref 在挂载时记下哪个节点。</p>
  </a>
</div>
