# Select

`select()` 创建一个单值下拉。模板里的标签是 `<Select>`。它没有构造参数。选项用 `.options(…)` 给。

字段是 `value: Option<Arc<str>>`、`options: Vec<SelectOption>`、`placeholder: Option<Arc<str>>`、`disabled: bool`、`loading: bool`。事件是 `on_change`，类型为 `SelectChanged`。处理器接收 `&SelectChanged`。`model` 绑定 `value`，选中后写成 `Some(event.value)`。

:::api

```rust view
use nana_ui::runtime::view;

view! {
    <Select placeholder="语言" options={options} v-model={language} />
}
```

```rust rust
use nana_ui::runtime::view::select;

select().placeholder("语言").options(options).model(language)
```

:::

`v-model={language}` 展开成 `.model(language)`。`language` 是 `Signal<Option<Arc<str>>>`。用户选中一项后，控件自己写入选中值并收起菜单。应用不用再写一遍才能看见。事件里的值是 `Arc<str>`，模型始终包成 `Some`。

选项用 `SelectOption::new(value, label)`，两个参数都进 `Arc<str>`。禁用某一项是 `SelectOption::new("rust", "Rust").disabled(true)`。禁用的选项仍留在打开的菜单里。

`.on_change(|event: &SelectChanged| …)` 自己听提交。模板里是 `on:SelectChanged={|event: &SelectChanged| …}`。

`loading` 和 `disabled` 可以是常量、信号或闭包。没有单独的 `label` 字段。放进设置行、又没人给它起名时，读屏仍按显示的文字命名；显示出来的选项是它的值。行标签存在时，无障碍名字用行标签，不再拿选项文字当名字。

`SelectChanged` 是指针、键盘或无障碍提交的选中。`value` 为 `None` 时还没有选中项。要否决用户的选择，在处理函数里把你要的值写回去。

`@change={save}` 在 `save` 是函数时原样传入，参数必须是 `&SelectChanged`。其它表达式展开成 `move |_| { … }`，事件被忽略。`options` 可以是 `Vec<SelectOption>`，也可以是这个类型的信号。

选项身份归应用。框架不另编一份 id。这是单值字段，不是多选。

不写 `placeholder` 就是 `None`。类型是 `Option<Arc<str>>`。

[总览](index.md) 和 [控件合同](../reference/components.md)
