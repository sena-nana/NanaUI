# 声明式视图：两种一级写法、token 样式表、具名 slot

`view!` 模板和 Rust 写法都是正式写法，`view!` 展开出来就是 Rust 写法的调用；`.vue` 退为高级用法。视图层转正，默认编译进来。说明见 [声明式视图](reactive-view.md)。

## 需要改的地方

- **`reactive-view` feature 删除。** 视图层默认编译进来。`nana-ui`、`nana-ui-runtime`、`nana-ui-devtools` 依赖里的 `"reactive-view"` 要去掉，否则 Cargo 报 feature 不存在。`view-macro`（`view!` / `css!` / `stylesheet!`）和 `reactive-trace` 仍是可选 feature；`nana-ui-devtools` 的视图检查（`inspect` / `set_field`）随 `runtime-agent` 一起提供。

| 旧 | 新 |
| --- | --- |
| `column(8.0, (a, b))`、`row(4.0, …)` | `column().gap(8).children((a, b))`；子节点多、要写 `for` / `if` 时用 `column().gap(8).with(\|c\| { c.add(a); … })`。`gap` 接受整数 |
| `view!` 里的 `style = ".a { … }";` | 模板开头写 `<style> .a { … } </style>`，里面是 CSS token，不是字符串。`<style>"…"</style>` 和 `style = …` 会报错并给出新写法 |
| `css!("padding: 4px 8px")` | `css! { padding: 4px 8px; }`；传字符串会报错 |
| `El::styles(&SITE, …)`（生成代码用的隐藏入口） | 删除。样式表写成 `stylesheet! { mod s; .card { … } }`，元素上写 `.class(s::card)` / `.class_when(s::done, cond)` |

**CSS token 的写法规则**：空格按源码位置还原，`.a.b` 和 `.a .b` 不会混。Rust 词法写不出的值放进双引号，编译时去掉引号拼接：`em` / `ex` 单位（`"1.5em"`；不加引号的 `1.5em` 会在宏展开前被 rustc 报 `expected at least one digit in exponent`）、数字后紧跟 `e` 的十六进制颜色（`"#9ecafe"`）。CSS 的单引号字符串改用双引号；`//` 只能写在引号里。`url("…")` 和选择器里的字符串保留引号。`.vue` 的 `<style>` 仍是原样 CSS 文本，不受这些限制。

## 行为变化

- **警告落在具体位置。** `view!`、`css!`、`stylesheet!` 的警告（样式、无障碍）以前都标在整个宏调用上，现在标在它说的那个 token 上：选择器、声明、元素、`class` 属性。数量没变；用 `#[allow(deprecated)]` 压警告的地方仍然有效。
- **模板的样式走 `Sheet`。** `<style>` 编译成一张 `view::Sheet`，`class` / `class:x` 展开成 `.class` / `.class_when`，和手写 Rust 是同一条路径。每种"固定类 + 条件类"组合在第一次出现时挑一次规则，之后复用；得到的布局和以前相同。一个元素上的类必须来自同一张表。
- **组合控件在它收的内容之后装配。** 一个视图里，放进 slot 的组合控件（例如 `DesktopShell` 的 `.primary(widget(SettingsPage…))`）先装配，收它的控件后装配，所以外层对内容的补丁（主区域的圆角等）不再被内容自己的装配覆盖。子节点仍在父节点之前装配。
- **类覆盖元素上已有的布局。** `.class` 的规则施加在元素建好时的布局上（包括 `.css(..)` 写的），冲突时类里的声明生效。

- **新的内置标签会遮蔽同名的自定义组件。** `view!` 里不带路径的标签先查内置控件表，查不到才调用同名的 snake_case 函数；宏看不到作用域里有哪些函数，所以内置表新增一个名字，同名的自定义组件就改为解析成内置控件，属性名碰巧对得上时不报错。这次新增的内置标签：`Avatar`、`Chip`、`EmptyState`、`IconButton`、`StatusBadge`、`Texture`、`Thumbnail`。自定义组件用了这些名字的，改成带路径的标签（`<kit::EmptyState …>`，同模块里写 `<self::EmptyState …>`），带路径的标签一律调用函数。

## 新增

- `list.each(key, row)` 等于 `each(list, key, row)`；`cond.then_show(|| v).otherwise(|| w)` 等于 `when(cond, || v).otherwise(|| w)`（`EachExt`、`WhenExt`）。
- `El::with(|c| …)` 和 `Children`：用普通 Rust 语句加子节点。
- 和手写代码配合：`entity_ref::<C>()` / `El::entity_ref` / `EntityRef::get`（类型化句柄）、`El::on_cx`（处理器拿到组件和 `ViewContext`，模板里是三个参数的 `on:E={…}`）、`AppContext::mount_view_detached`、`MountedView::root`、`with_refs`（挂载闭包返回 `with_refs(视图, 句柄)`，挂载返回 `(MountedView, 解析好的句柄)`）。`ListItem` / `SidebarRow` 的 `.leading` / `.content` / `.trailing`。
- `stylesheet!`、`Class`、`Sheet`、`El::class`、`El::class_when`。
- `detached(view)`：在视图里建出节点但不放进任何地方，配 `entity_ref` / `with_refs` 拿到 id，交给 `set_modal_slots` 这类按 id 收内容的接口。和其他根放在同一个 tuple 里，一次挂载、一次 commit。
- `view!` 的具名 slot：`<template #navigation>…</template>` 展开成元素上的 `.navigation(view)`，`#title-trailing` 是 `.title_trailing(…)`，`#default` 是普通子节点。`<Suspense>` 的 `#fallback` 在 `view!` 里也能写了。
- 组合控件在视图里自己装配：`DesktopShell`、`AppTitleBar`、`SettingsRow`、`SegmentedControl`、`SidebarSection`、`AppShell`、`Workspace`、`Dock`、`SplitPane`（新增无内容的 `SplitPane::new(&model)`，内容用 `.first` / `.second`）、`PaneSection`（`.header` / `.tabs` / `.body`）、`GraphCanvas`、`DatePicker`、`NativeMarkdown`、`ConfirmDialog`、设置页的 `AppearanceSection` / `AboutSection` / `SettingsSidebar` / `SettingsCollapsibleCard`（`.summary` / `.details` / `.accessory`）/ `SettingsPage`（`.content`）登记了 `TypeBehavior::slot_assembler`（`SidebarSection` 自己建表头和 body，见 `AppContext::assemble_sidebar_section`；`SidebarFrame` 有 `.top` / `.body` / `.footer`；`AppShell` 有 `.title_bar` / `.body` / `.overlay`，`Workspace` 有 `.region(id, view)`；Dock 的面板是 key 等于面板 id 的子节点，见 `AppContext::assemble_dock_panels`）；`Chip`、`ColorField`、`PathField`、`FileTab`、`DiffView`、`MediaTransportBar` 在视图建好时运行自己的装配。视图里不再调用 `assemble_*`。
