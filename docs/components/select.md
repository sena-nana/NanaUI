# Select

单值下拉。选项身份归应用，框架不另编一份 id。要多选，或要和菜单共用字段表面，用 [Dropdown](dropdown.md)。边输入边查，用 [SearchDropdown](search-dropdown.md)。

模板里的标签是 `<Select>`。它没有构造参数。选项用 `.options(…)` 给。

## 基本用法

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

## 选项

选项用 `SelectOption::new(value, label)`，两个参数都进 `Arc<str>`。禁用某一项是 `SelectOption::new("rust", "Rust").disabled(true)`。禁用的选项仍留在打开的菜单里。

`options` 可以是 `Vec<SelectOption>`，也可以是这个类型的信号。这是单值字段，不是多选。

## 加载、禁用和名字

`loading` 和 `disabled` 可以是常量、信号或闭包。

下拉框画出来的是它的值，不是名字。`label` 是它的名字：读屏说它是做什么的，不画出来（`aria-label`）。什么都没给时，读屏只能拿显示的选项当名字，读出「升序」而不是「排序方向」。显示出来的选项始终是它的值。

:::api

```rust view
view! {
    <Select label="排序方向" options={directions} v-model={direction} />
}
```

```rust rust
select().label("排序方向").options(directions).model(direction)
```

:::

旁边已经有可见的字段标题时，不必再写一遍名字：`labelled_by` 指向那段标题（`aria-labelledby`），读屏用标题的文字，标题改了跟着改。标题写在下拉框前后都行，模板里写标题的 `ref`：

:::api

```rust view
view! {
    <Text ref={caption}>"排序方向"</Text>
    <Select labelled_by={caption} options={directions} v-model={direction} />
}
```

```rust rust
let caption = node_ref();
(
    text("排序方向").node_ref(caption),
    select().labelled_by(caption).options(directions).model(direction),
)
```

:::

放进设置行时，行标签同样给它起名。自己的 `label` 不为空时，用自己的。

不写 `placeholder` 就是 `None`。类型是 `Option<Arc<str>>`。

## 否决这次选择

`SelectChanged` 是指针、键盘或无障碍提交的选中。`value` 为 `None` 时还没有选中项。要否决用户的选择，在处理函数里把你要的值写回去。

`.on_change(|event: &SelectChanged| …)` 自己听提交。模板里是 `on:SelectChanged={|event: &SelectChanged| …}`。

`@change={save}` 在 `save` 是函数时原样传入，参数必须是 `&SelectChanged`。其它表达式展开成 `move |_| { … }`，事件被忽略。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `value` | `Option<Arc<str>>` | 当前选中。`model` 在选中后写成 `Some(event.value)` |
| `options` | `Vec<SelectOption>` | 也可以是这个类型的信号 |
| `label` | `Option<Arc<str>>` | 读屏名字，不画出来。不写时由 `labelled_by` 或设置行起名，再没有就读显示的选项 |
| `placeholder` | `Option<Arc<str>>` | 不写就是 `None` |
| `disabled` | `bool` | 常量、信号或闭包 |
| `loading` | `bool` | 常量、信号或闭包 |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `on_change`，模板 `on:SelectChanged` 或 `@change` | `&SelectChanged` | 指针、键盘或无障碍提交。函数必须接收这个引用；其它表达式会忽略事件 |

## 插槽

没有插槽。选项是 `options` 里的数据，不是子节点。

## 参见

[总览](index.md) · [控件](../reference/components.md)
