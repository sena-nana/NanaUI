# 快速开始

这篇带你写出第一段界面。能打开的整段程序在 [创建应用](essentials/application.md)。先看过 [介绍](introduction.md) 会更容易跟上。

::: tip 预备知识
需要 Rust 1.98+。这个仓库目前用 path 或 git 引用，还没有发布到 crates.io。
:::

## 添加依赖

`nana-ui` 的默认 feature 是空的：不启用 `hosted` 就没有 `run_runtime`，不启用 `gpu` 就没有 painter。一个桌面应用至少写成这样：

```toml
[dependencies]
nana-ui = { path = "../NanaUI/crates/nana-ui", features = ["hosted", "bundled-fonts", "view-macro"] }
```

`hosted` 会带上 `gpu`、winit 和 AccessKit。`view-macro` 用来编译 `view!`、`css!` 和 `stylesheet!`。你如果只写函数调用，可以去掉它。更多控件族见 [应用 API](../reference/application-api.md)。

## 先看成品

在仓库根目录运行：

```bash
cargo run -p component-gallery
cargo run -p nana-ui --example gpu-view-demo --features hosted,bundled-fonts
```

Gallery 是控件目录。最小的宿主对照是 `examples/runtime-host-fixture`。

## 第一段界面

一个应用做三件事：建一棵 `RuntimeDocument`，实现 `RuntimeProgram`，然后调用 `run_runtime`。界面本身写在挂载闭包里。

下面两段是同一个界面，展开成同一次挂载。`start` 是一个没有参数的函数。侧边栏顶部的开关决定你看到哪一种。

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <Column gap=12>
        <Text>"你好"</Text>
        <Button @activate={start}>"开始"</Button>
    </Column>
}
```

```rust rust
use nana_ui::runtime::view::{button, column, text};

column().gap(12).children((
    text("你好"),
    button("开始").on_activate(start),
))
```

:::

要点击时开窗或更换纹理，用 `.on_cx(|_button, _event: &Activate, cx| cx.dispatch_program(msg))`。模板里把 `on:Activate={…}` 写成三个参数的闭包，展开的就是这一个方法。那条消息在 `update` 里处理。

把这段视图放进窗口，需要一棵 `RuntimeDocument` 和一份 `RuntimeProgram`。能编译运行的整段在 [创建应用](essentials/application.md#整段程序)。

打开 `bundled-fonts` 之后，宿主会注册 Noto Sans SC，并把它设为界面的默认字体。关掉这个 feature，就回落到系统字体。控件从 `nana_ui::runtime` 引入。

## 状态放在哪里

| 东西 | 放在哪里 |
| --- | --- |
| 按钮是否 loading、输入框当前值 | 对应控件，或视图里的信号 |
| 打开了哪个文档、登录态、设置值 | 应用自己的结构 |
| 侧栏宽度、Region 折叠 | `WorkspaceModel`，见 [工作区](../reference/workspace.md) |
| 窗口位置和尺寸 | `WindowDescriptor::persist_key`，见 [窗口](../reference/window.md) |
| 这一帧的实时画面 | 你的 GPU 资源加上 `HostTextureRegistry`，见 [实时画面](../reference/gpu.md) |

`RuntimeProgram::Message` 用来传递跨窗口、GPU 和持久化这类宿主消息。

## 接下来

<div class="next-steps">
  <a class="next-step" href="/guide/essentials/application">
    <p class="next-step-link">创建应用</p>
    <p class="next-step-caption">把上面的视图放进能打开的窗口。</p>
  </a>
  <a class="next-step" href="/guide/essentials/view">
    <p class="next-step-link">视图写法</p>
    <p class="next-step-caption">同一个界面，两种写法怎么展开。</p>
  </a>
  <a class="next-step" href="/components/">
    <p class="next-step-link">组件</p>
    <p class="next-step-caption">按族查看已经有的控件。</p>
  </a>
  <a class="next-step" href="/architecture/">
    <p class="next-step-link">架构</p>
    <p class="next-step-caption">从这棵树到窗口上的一帧。</p>
  </a>
</div>
