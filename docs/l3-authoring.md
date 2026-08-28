# L3 组成式建树

Rust 产品入口是 [`AppContext::build`](../crates/nana-ui-runtime/src/framework/build.rs)：嵌套描述一棵子树，一次 commit 进保留 `UiWorld`，事件写在 child 旁边。底层仍是 `create_component` / `append_child` / `on`；不要把 `build` 当成每帧 `Render`。

对照与取舍见文末。第一扇窗口见 [开始](start.md)。

## 写法

```rust
let start = cx.build(document_id, |ui| {
    ui.column(12.0, |ui| {
        ui.child("title", Text::new("你好"));
        let start = ui.child("start", Button::new("开始"));
        ui.on(start, |_, _: &Activate, cx| {
            cx.dispatch_program(Start);
        });
        ui.row(8.0, |ui| {
            ui.child("open", Button::new("打开"));
            ui.child("float", Button::new("浮窗"));
        });
        start
    })
})?;
```

内层闭包不返回 `Result`。错误记在 builder 上，只在 `build()` 出口变成 `Result<R, FrameworkError>`。

| 方法 | 作用 |
| --- | --- |
| `child(key, component)` | 在当前父节点下创建或复用 keyed 控件，返回 `Entity<C>` |
| `on(entity, handler)` | 本批 commit 之后注册，签名与 `AppContext::on` 相同 |
| `column` / `row` | 造 `Stack` 并开嵌套；自动 key（`#column-0` 这类）。其它 `Stack` 变体用 `with` |
| `with(key, component, \|ui\| …)` | keyed 容器 + 嵌套，返回闭包的值 |
| `nest(entity, \|ui\| …)` | 已有实体设为当前父节点 |
| `build_child(parent, \|ui\| …)` | 往已有父节点建 keyed 子树；与日后 `mount` 共用 key 表 |
| `build_detached` | 顶层 `child` 保持 parked，给随后 assemble 的子树（shell / dock / overlay） |
| `leaf(component)` | 创建但不插入当前父节点，**始终 parked**；给需要先拿 id 的槽位 |
| `adopt(entity)` | 把已有节点插入当前父节点 |

`Stack` 仍然只是样式容器，**子列表不进 `Stack` 字段**。

## 身份

- 静态 chrome **强制 key**。`child` / `with` 接受 `&'static str`。
- 容器快捷方式（`column` 等）自动分配 `#column-0` 形式的 key，同一父节点下按调用顺序稳定。
- 动态列表继续 `mount` / `materialize_virtual_*`，不要每帧 `build`。
- 同一 parent 下 key 重复 → `DuplicateAssemblyKey`，且不 commit。
- 类型变化 → despawn 再建（与 `mount` 相同）。

## 更新（不要整树 render）

| 变化 | API |
| --- | --- |
| 按钮文案 / loading | `update_component(entity, \|v, _\| …)` |
| 某区增删子项 | `cx.mount(parent, \|ui\| { ui.child(…) })` |
| 初次整页 | `cx.build(…)` |
| 虚拟窗口 | `materialize_virtual_*` |

`RuntimeProgram::update` 和点击 handler **不得**拆树再 `build`。那不是 Nana 的 `notify`。

## 现有 API

| API | 命运 |
| --- | --- |
| `build` / `build_child` | 产品默认作者面 |
| `create_component` / `append_child` / `on` | 底层、测试、单节点补丁 |
| `create_detached_component` | 保留；产品教程不再教 |
| `mount` / `child` | 动态区的默认 |
| `bind_component` | Vue / 已有 id 的宿主专用 |
| `RuntimeProgram` | 不改成 `cx.new(\|cx\| View)` |

## 性能

一次 `build` = 一次 `world.commit`（全部 create / project / insert）。子节点在同一批里 insert 进父节点，不会先作为文档根单独提交。`build` 路径不做布局、抽取或 GPU。

命令式 `create_component` + `append_child` 仍然每次调用 commit 一次，适合单节点补丁，不适合整页挂载。

## 和 GPUI 的差别

GPUI 的现代面是：`Entity` 保留状态，每帧 `render()` 返回嵌套的 `div().child(...)`。Nana 只借作者层的嵌套 `.child` / 事件就近绑定，运行时仍是保留 `UiWorld`（Vue L1/L2 与 Rust 写同一棵树、增量 flush、稳定 `StableNodeId`）。

不抄这些：`div()` / Tailwind 式样式（和 Style Model 冲突；CSS 子集只属于 L1）、每帧 `impl Render`、`cx.notify()` 重投影整 View、把闭包塞进 `Button` 字段（`ComponentView` 要 `Clone`）。handler 仍在 `AppContext` 表里，只是写在 child 旁边。
