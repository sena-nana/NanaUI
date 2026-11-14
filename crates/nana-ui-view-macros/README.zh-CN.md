# nana-ui-view-macros

[English](README.md) · 简体中文

NanaUI 声明式视图的过程宏：`view!`、`css!`、`stylesheet!`、`#[component]` 和 `#[derive(Store)]`。

`view!` 接受类 Vue 的模板，展开成普通的视图函数调用，运行时不多出任何东西。`css!` 和 `stylesheet!` 在编译期经 CSS 引擎编译，`stylesheet!` 为每个 class 生成一个 `view::Class`。`#[component]` 让模板按参数名传参并支持 `#[prop(default)]`，`#[derive(Store)]` 为结构体生成按字段追踪的访问器。

请经 `nana_ui::runtime`（或 `nana_ui_runtime`）使用这些宏，打开 `nana-ui` 或 `nana-ui-runtime` 的 `view-macro` feature 即可，不要直接依赖本 crate。代码生成本身在 [`nana-ui-view-codegen`](../nana-ui-view-codegen)。

## 示例

```rust
view! {
    <Column gap=12>
        <Text>"计数 {count}"</Text>
        <Button @activate={count.update(|c| *c += 1)}>"加一"</Button>
        <Text v-if={loading}>"加载中"</Text>
        <Text v-else>"完成"</Text>
        <Slider min=0 max=1 step=0.05 v-model={volume} />
    </Column>
}
```

字面量是常量，`{path}` 原样传入，其他 `{表达式}` 变为 `move || 表达式`。

## 测试

```bash
cargo test -p nana-ui-runtime --features view-macro --test contracts view_macro
```

这条测试检查模板与等价的手写函数挂出同一棵树、以同样方式更新。其余检查见 [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml)。

更多内容见 [声明式视图](../../docs/reference/reactive-view.md) 和 [组合视图](../../docs/guide/essentials/view-functions.md)。
