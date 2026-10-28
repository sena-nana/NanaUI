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

`.hint(text)` 给选项配一段次要文字，例如译名后面的原文：`SelectOption::new("x", "头部左右转").hint("FaceAngleX")`。菜单里它在这一行的末尾，比标签小一号、用次要色；收起的框里跟在所选项的标签后面。宽度先给标签，提示放不下时省略，连一小段都放不下就不画。读屏把它读在标签之后，画不下时也读。空字符串就是没有。

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

## 按选项定宽

下拉框默认占满一行。要它像浏览器原生的 `<select>` 那样按最长的选项定宽，打开 `fit_options`：

:::api

```rust view
view! {
    <Select fit_options={true} options={repos} v-model={repo} />
}
```

```rust rust
select().fit_options(true).options(repos).model(repo)
```

:::

这时框里的内容宽取所有选项里最宽的一项（没有选中、显示占位文字时，还有占位文字），再加上箭头位；框宽是它加上内边距和边框。带提示的选项连提示一起量，量时按标签的字号算，宁宽勿窄。换一个选项框宽不变，选项增删时跟着变。

`fit_options` 只决定下拉框自己的内容宽，宽度由内容决定时才用得上：样式里写 `width: max-content`（函数 API 是 `.css(css! { width: max-content; })`），直接改 `Select` 的样式时是 `LengthSpec::Shrink`。默认的宽度照旧占满一行，写了确定的宽度时照旧按那个宽度；`min_width`、`max_width` 照常生效，例如再写 `min_width` 保住一个最小宽度。不写 `fit_options` 时和原来一样，内容宽只看当前显示的那一项，也不给箭头留位。

框比最长的选项窄时（写了确定的宽度，或被 `max_width` 截住），打开的菜单仍按最长的一项展开，选项不被截断。菜单从框的左边开始；右边超出菜单看得见的范围（见下文“菜单”）时向左移，刚好收在右边界上；比整个范围还宽时截成范围的宽度。菜单永远不比框窄。不写 `fit_options` 时菜单和框一样宽。

```rust
let mut select = Select::new(Some(current)).options(options).fit_options(true);
let layout = Arc::make_mut(&mut select.style.layout);
layout.width = Some(LengthSpec::Shrink);
layout.min_width = Some(LengthSpec::Px(122.0));
```

## 菜单

菜单从框下方展开。下方放不下、上方放得下时翻到上方；两边都放不下时开在空间大的一边，高度截到那一边为止。空间指菜单看得见的地方：窗口，再扣掉框外层每个会裁剪它的容器（滚动视图、`overflow` 裁剪的盒子、对话框卡片），按它们此刻的滚动位置算。哪些容器裁剪它，和画框、点框时一样按 clip 链算：`absolute` 的框不受它逃出的容器裁剪，`fixed` 的框（或框外层有一个 `fixed` 的盒子）只算到那里为止，所以 `fixed` 的框打开的菜单可以用整个窗口。所以卡片里的下拉框不会把菜单开到卡片裁掉的地方，页面滚过之后也不会按没滚时的位置把菜单开到窗口外。菜单开着时页面再滚动，菜单跟着框一起走；等菜单下一次变化（滚动、移动高亮）或下一趟布局时再按新位置摆放。窗口改变大小、框或外层的容器被布局挪动时，菜单开着也会按新的空间重新翻面、截高，不用等框重新投影；画出来的、指针点到的和命中测试用的始终是同一份摆放。

菜单画在页面的最上层，和 Popover 的面板一样：框后面的块、卡片里排在后面的设置行都不会盖住它，指针落在菜单上时也归菜单。只有框外层的裁剪仍然有效，菜单不会画到裁掉的地方。

菜单里只有一个高亮：打开时在当前选中的一项上，指针移到哪一项（禁用的除外）它就到哪一项，方向键也移动它，回车选中它。指针不在任何一项上时高亮留在原处，菜单也不因高亮跟随指针而滚动。

选项多到装不下时，它们在菜单里滚动，裁在菜单的边框以内，不会画到菜单外面：

- 滚轮在菜单上滚动选项，滚到头也不交给下面的页面。菜单装得下全部选项时，滚轮照旧滚页面。按住 Ctrl 时滚轮不归菜单。
- 方向键移动高亮时，把高亮的一项滚进来，只滚必要的距离。菜单打开时，当前选中的一项尽量放在中间。
- 右侧有一条滚动条，尺寸和配色与 [ScrollView](scroll-view.md) 的一样，取自主题的 `ScrollbarMetrics` 和 `ScrollbarRecipe`。菜单开着、装不下时一直显示；主题的显示策略是 `Always` 时带常驻轨道，是 `Hidden` 时不画，滚轮和键盘照常。指针移到滚动条上时滑块变粗、换成悬停色；拖动滑块滚动选项；按在空轨道上时滑块先对准按下的位置，再接着拖；拖动被取消时回到按下前的位置。选项让出滚动条那一条，滑块不会压住文字和高亮。
- 点到的就是画在指针下的那一项：只有在菜单里看得见的部分接受点击，滚动条那一条不算选项。页面滚过之后也一样。

滚动位置和高亮一样是交互状态（`Select::menu_scroll`，类型 `SelectMenuScroll`）：菜单每次打开都重新开始；应用刷新时值和选项没变就保留。[Dropdown](dropdown.md) 和 [SearchDropdown](search-dropdown.md) 用的是同一个菜单，表现相同；SearchDropdown 的查询词变了时，回到第一个匹配项。

## 读屏

字段是组合框（`ComboBox`），读屏说它展开还是收起。菜单打开时，字段下面多一个列表框（`ListBox`），菜单里每一行是其中的一个选项（`ListBoxOption`），和网页上 `role="combobox"` 控制一个 `role="listbox"` 一样：

- 选项读它的标签，提示读在后面（`description`）。当前选中的那一项是「已选」，其余是「未选」；禁用的选项读作禁用。每个选项带它在整份列表里的位置和总数（`aria-posinset` / `aria-setsize`），所以只露出几行时，读屏仍说「第 12 项，共 40 项」。
- 焦点留在字段上。高亮的那一项是字段的活动后代（`aria-activedescendant`），方向键移动高亮时读屏跟着读。
- 选项的范围就是菜单画它、指针点它的那一块：同一份菜单几何，经字段的滚动和变换换到窗口里。滚出菜单的行、被字段外层的裁剪整个裁掉的行没有节点；裁掉一部分的行，范围是露出来的那部分。菜单翻面、截高、滚动时，选项跟着走。
- 选项按值认，值不变节点就不变。菜单收起时列表框和选项一起去掉。
- 读屏激活一个选项（AccessKit 的 `Click`，Runtime 的 `ActivateMenuItem`），和指针按在这一行上一样：选中它、收起菜单、发 `SelectChanged`。禁用的选项不响应。

## 否决这次选择

`SelectChanged` 是指针、键盘或无障碍提交的选中。`value` 为 `None` 时还没有选中项。要否决用户的选择，在处理函数里把你要的值写回去。

`.on_change(|event: &SelectChanged| …)` 自己听提交。模板里是 `on:SelectChanged={|event: &SelectChanged| …}`。

`@change={save}` 在 `save` 是函数时原样传入，参数必须是 `&SelectChanged`。其它表达式展开成 `move |_| { … }`，事件被忽略。

## 菜单开关

菜单每打开一次、每收起一次，都各发一次 `SelectToggled { open }`。不管是怎么收起的都会发：点框、选中一项（包括再选一次已显示的那一项）、回车、Escape、点到别处、焦点离开（Tab 或应用改了焦点），或者应用刷新时删了选项、禁用了控件、置为加载中。写入的状态和原来一样时不发，比如对开着的菜单再要求打开一次，或者刷新时值和选项都没变。同一次操作里先发 `SelectChanged`，再发 `SelectToggled`。

`open` 说的是菜单有没有显示出来：控件禁用或加载中时菜单不显示，算收起。应用直接把 `disabled` 或 `loading` 写回 `false` 而 `opened` 还是 `true` 时，菜单重新显示，也会发一次 `open: true`。

焦点离开时菜单随之收起，和浏览器原生的 `<select>` 一样。

`.on_toggle(|event: &SelectToggled| …)` 自己听开关，模板里是 `@toggle` 或 `on:SelectToggled`。

## 属性

| 属性 | 类型 | 说明 |
| --- | --- | --- |
| `value` | `Option<Arc<str>>` | 当前选中。`model` 在选中后写成 `Some(event.value)` |
| `options` | `Vec<SelectOption>` | 也可以是这个类型的信号。选项可带 `.hint`，见上文 |
| `label` | `Option<Arc<str>>` | 读屏名字，不画出来。不写时由 `labelled_by` 或设置行起名，再没有就读显示的选项 |
| `placeholder` | `Option<Arc<str>>` | 不写就是 `None` |
| `disabled` | `bool` | 常量、信号或闭包 |
| `loading` | `bool` | 常量、信号或闭包 |
| `fit_options` | `bool` | `true` 时内容宽取最长的选项再加箭头位，像原生 `<select>`。配合内容决定的宽度（`Shrink`）用。框更窄时菜单仍按最长的选项展开，收在看得见的范围内。默认 `false` |

## 事件

| 事件 | 载荷 | 说明 |
| --- | --- | --- |
| `on_change`，模板 `on:SelectChanged` 或 `@change` | `&SelectChanged` | 指针、键盘或无障碍提交。函数必须接收这个引用；其它表达式会忽略事件 |
| `on_toggle`，模板 `on:SelectToggled` 或 `@toggle` | `&SelectToggled` | 菜单打开或收起，`open` 是之后的状态。每次变化发一次，见上文“菜单开关” |

## 插槽

没有插槽。选项是 `options` 里的数据，不是子节点。

## 参见

[总览](index.md) · [控件](../reference/components.md)
