# Progress

`progress(max)` 创建一条确定进度。`max` 是 `f64`，在构造时给定。模板里的标签是 `<Progress>`。函数注释要你再绑 `.value(…)`。

字段只有 `value: f64` 和 `label: Option<Arc<str>>`。没有事件，也没有 `model`。轨道是 Subtle 底、Accent 填充。

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <Progress max=100 value={done} label="导出" />
}
```

```rust rust
use nana_ui::runtime::view::progress;

progress(100.0).value(done).label("导出")
```

:::

模板里的 `max=100` 展开成 `100_f64`。手写时参数是 `f64`，写成 `100.0`。`value` 可以是常量、`Signal<f64>` 或闭包。

显示的值夹在 `0` 和 `max` 之间。非有限的 `value` 按 `0`。`max` 不是有限正数时，组件把它收成一个极小的正数，不拿它做除零。无障碍同时给出百分比、最小、最大和当前数值。

有标签时，标签在轨道上方。标签可以不给。没有标签、也不可取消时，高度就是轨道本身。轨道厚度是 6。进度不自己跑表，数值由应用写。

无障碍的值是四舍五入后的百分比，同时带上最小、最大和当前数值。填充比例是夹紧后的 `value / max`。

可取消不是字段。组件 `Progress::cancellable(true)` 才接收指针、才可聚焦。激活时如果 `cancellable` 仍为真，发 `ProgressCancelled`，进度本身不删、不停表。听这个事件用 `.on(|_: &ProgressCancelled| …)`。`<Progress>` 没有对应的 `on_` 方法。进度控件不拥有计时器。

`label` 的类型是 `Option<Arc<str>>`。不写就是没有标题。有标题时字重走 medium。

`value` 可以跟着信号变。轨道只显示你写进去的数，自己不加动画相位。

`max` 只是构造参数，不是之后能绑的字段。视图函数没有 `.max`。天花板要变，用 `.bind(|progress| progress.max = next)`，或重新构造。

[总览](index.md) 和 [控件合同](../reference/components.md)
