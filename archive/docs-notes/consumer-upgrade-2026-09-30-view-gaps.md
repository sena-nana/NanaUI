# 视图迁移补缺：写回默认值的类、结构块的容器、虚拟列表句柄

LiliaBilibili 迁到声明式视图时发现的一批框架缺口。说明见 [声明式视图](../../docs/reference/reactive-view.md)、[控件](../../docs/reference/components.md)、[布局（Rust）](../../docs/reference/rust-layout.md)。

## 需要改的地方

- **`UiMutation` 多了一个变体 `SetLabelledBy`。** 穷举匹配 `UiMutation` 的代码要加一条分支。
- **`SettingsPage` 多了公开字段 `title_size`、`title_weight`。** 用结构体字面量构造 `SettingsPage` 的代码要补上（`None` 是原来的 18 / 600）；用 `SettingsPage::new` 的不受影响。
- 应用里为绕开缺口写的代码可以删掉：包住 `each` / `when` 的有尺寸盒子（改用 `<Block class>` / `.class(..)`）、为了防 debug 栈溢出加的 `.into_any()`、`hide_header(true)` 加自己画的页标题（改用 `title_size` / `title_weight`）、`height: 100%` 代替 `height: auto` 的拉伸。

## 行为变化

- **类会写回默认值。** 补丁里是声明写了的字段，不再是"和默认值不同"的字段：`align-items: flex-start`、`position: static`、`flex-wrap: nowrap` 这样的声明会覆盖元素建出来时的非默认值。以前被静默丢掉的这类声明现在生效。
- **行里的子项按分到的宽度量高。** `Fill` 列在定宽侧栏旁收窄后，里面 16:9 的盒子、换行文字按收窄后的宽度算高，这一列和这一行不再比内容高。
- **高度按内容算的脱流盒子**（`position: absolute`，没写高度、也没同时写 `top` 和 `bottom`）里，`Fill` / 百分比高度按内容处理，不再取包含块的高度。
- **`height: auto`（行里）/ `width: auto`（列里）在 `align-items: stretch` 下拉伸**，和没写一样；`Shrink` 等按内容的关键字仍保持内容尺寸。
- **`word-break: break-word`** 在样式表里生效（以前被丢掉）。
- **设置行的标签是控件的无障碍名字**：控件自己没有名字时读屏读行标签。`Select` 的无障碍名字不再是它显示的选项（选项是它的值）；没有任何节点给它起名时仍按显示的文字命名。
- **虚拟列表保持阅读位置**：在视口顶部那一项上面插入或删除项时，它留在原位；`follow_end` 的列表照常跟到底。
- **child slot 的根保留自己的 key**（以前改名成 `#adopt-N`）；和同一父节点下别的子节点 key 冲突时报 `DuplicateAssemblyKey`。

## 新增

- `each` / `when` / `dynamic` / `each_virtual` 的 `.class(..)`、`.class_when(..)`、`.css(..)`：给结构块的容器；模板里 `<Block class="…">` 包住 `v-for` 元素或 `v-if` 链，`<Virtual>`、`<Transition>`、`<TransitionGroup>`、`<KeepAlive>` 上的 `class` 同样给容器。
- `virtual_list_ref::<K>()` + `EachVirtual::list_ref(..)`（模板 `<Virtual list-ref={..}>`）：`item(&key)`、`row(&key)`、`first_visible()`、`scroll_to(&key, VirtualAlignment)`、`scroll_to_inset(&key, inset)`。`VirtualAlignment` 从 `view` 模块导出。
- `SettingsPage::title_size(f32)`、`title_weight(u16)`。
- `MutationQueue::set_labelled_by(control, label)`、`UiWorld::labelled_by(id)`。
- `nana_ui_css::written_layout` / `WrittenLayout`：一组声明写了哪些 Style Model 字段。
