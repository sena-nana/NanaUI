# nana-ui-runtime

NanaUI 的保留式运行时，与渲染后端无关。

`UiWorld` 是界面树的唯一权威：节点身份、拓扑和逻辑状态都在这里，对外只暴露 `StableNodeId`。修改以 `UiMutation` 排进 `MutationQueue`，在帧边界整批校验、原子提交。在这棵树上，本 crate 实现了布局、输入路由、焦点、IME、无障碍、动效，以及全部内置控件和 Shell、Workspace、Dock、浮层等桌面组件。`CustomRenderNode`、`GpuView` 这类 GPU 内容节点也在这里声明，和其他节点一样参与布局和命中测试。

`view` 模块提供声明式写法：信号直接更新它绑定的节点字段，不重新渲染，也不做 diff。

本 crate 不持有窗口、Surface 或 GPU 设备，也不解析 CSS。场景抽取在 [`nana-ui-scene`](../nana-ui-scene)，绘制和宿主在 [`nana-ui`](../nana-ui)，应用通常经 `nana_ui::runtime` 使用这里的类型。

## 示例

```rust
use nana_ui_runtime::view::{IntoView, button, column, signal};
use nana_ui_runtime::{AppContext, DocumentId};

fn counter() -> impl IntoView {
    let count = signal(0u64);
    column().gap(12.0).children((
        nana_ui_runtime::text!("计数 {count}"),
        button("加一").on_activate(move || count.update(|c| *c += 1)),
    ))
}

let mut cx = AppContext::new();
let document = DocumentId::new(1).unwrap();
let view = cx.mount_view_root(document, counter).unwrap();
```

## Features

- `icu`（默认）：用 ICU4X 和编译进来的 CLDR 数据格式化数字、日期和复数；关闭后用 `PlainFormatter`。
- `calendar`、`charts`、`controls`、`graph-canvas`、`image-viewer`、`rich-text`：可选的控件族，`components` 一次打开全部。`rich-text` 包含 Markdown、Mermaid 图和数学公式。
- `snippet-transforms`（默认）：snippet 里的正则变换；关闭后变换原样返回，不链接正则引擎。
- `syntax-highlighting`：代码块语法高亮。
- `view-macro`：`view!` 模板宏。
- `reactive-trace`：记录响应式更新的因果链，回答“为什么更新”。

`layout-verify` 和 `benchmark` 只用于测试与基准。

## 测试

```bash
cargo test -p nana-ui-runtime
cargo test -p nana-ui-runtime --features view-macro --test contracts --test perf_contracts -- view_macro view_stack theme_layout
```

`view_macro`、`view_stack` 和 `theme_layout` 不带 `view-macro` 时会编译为空并显示通过。基准需要 `benchmark` feature：

```bash
cargo run --release -p nana-ui-runtime --features benchmark --bin nana-runtime-benchmark
```

其余命令见 [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml)。

更多内容见 [运行时保留树](../../docs/learn/runtime-tree.md)、[响应式视图](../../docs/reference/reactive-view.md) 和 [Runtime 与 Scene 合同](../../docs/reference/runtime-scene.md)。
