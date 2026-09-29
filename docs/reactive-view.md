# 声明式视图

`nana_ui::runtime::view`：视图写成一个表达式，动态部分是信号或闭包，树只建一次，之后每个绑定只更新它写的那个节点字段。不整树 render，不做 diff。它和 `build` / `mount` 写同一棵 `UiWorld`、同一张 assembly key 表，最后落到的还是 `create` / `insert` / `project` / `commit`。

**开关**：视图层默认编译进来，不需要 feature；模板宏 `view!`、`css!`、`stylesheet!` 在 `view-macro` 后面（独立的 proc-macro crate，不用就不参与编译）；因果追踪在 `reactive-trace` 后面。示例：`crates/nana-ui/examples/reactive-counter.rs`（用 `view!` 写），`examples/reactive-sfc`（用 `.vue` 文件写）。

**两种一级写法，一条路径。** `view!` 模板和 Rust 函数写法都是正式写法，能力对齐：`view!` 展开出来就是 Rust 写法的那些调用（`column().gap(8)`、`.class(..)`、`each(..)`……），两者没有各自的实现。每个模板构造都有同名的 Rust 方法，对应表见 [`view!` 与 Rust 写法](#view-与-rust-写法)。`tests/view_macro.rs` 把同一个页面分别写成模板、tuple 子节点和 `.with` 块三种形式，挂载后逐节点相同，信号变化后也相同；Transition、KeepAlive、Suspense、Teleport、ErrorBoundary、虚拟列表和具名 slot 各有一组同样的对照；样式同样一份写成 `<style>`、一份写成 `stylesheet!`，布局相同。模板贴近 Vue，适合成段的界面；Rust 写法是普通 Rust，能写 `for` / `if`，有完整的补全和类型检查，类名写错是编译错误。`.vue` 文件属于[高级用法](#高级用法vue-方言)。

## 写法

```rust
use nana_ui::runtime::view::*;
use nana_ui::runtime::text;

fn counter() -> impl IntoView {
    let count = signal(0u64);                                   // 组件局部状态
    row().gap(8).children((
        text!("计数 {count}"),                                   // 插值，读到的信号自动成为依赖
        button("增加").on_activate(move || count.update(|c| *c += 1)),
    ))
}

fn todos(list: Signal<Vec<Todo>>, draft: Signal<String>) -> impl IntoView {
    column().gap(8).with(|c| {
        c.add(text_input().placeholder("新任务").model(draft));   // v-model
        c.add(button("添加").disabled(move || draft.with(|d| d.is_empty())));
        c.add(list.each(|t| t.id, |t| text(t.title)));           // v-for + :key
        c.add((move || list.with(Vec::is_empty))
            .then_show(|| text("还没有任务"))
            .otherwise(|| text("有任务")));                       // v-if / v-else
    })
}

let view = cx.mount_view(parent, counter)?;                    // 或 mount_view_root(document, …)
```

同一个 `todos` 写成模板（展开结果和上面相同）：

```rust
view! {
    <Column gap=8>
        <TextInput placeholder="新任务" v-model={draft} />
        <Button disabled={draft.with(|d| d.is_empty())}>"添加"</Button>
        <Text v-for={t in list} key={t.id}>{t.title}</Text>
        <Text v-if={list.with(Vec::is_empty)}>"还没有任务"</Text>
        <Text v-else>"有任务"</Text>
    </Column>
}
```

- **子节点**：写死的几个用 tuple，`.children((a, b))`，零成本；需要 `for`、`if`、`let` 时用块，`.with(|c| { c.add(a); … })`，每个子节点装箱一次。块在建树时只跑一次，里面的 `for` / `if` 决定的是建树那一刻的结构，不跟着数据变；要跟着数据变，用 `each` / `when`。
- **和手写代码配合**：消息驱动的应用（程序保存控件句柄、事件发消息给 `RuntimeProgram`）也可以用视图建界面。`entity_ref::<C>()` 配 `.entity_ref(r)` 记下元素建成的 `Entity<C>`；挂载闭包返回 `with_refs(视图, 句柄)` 时，挂载直接返回 `(MountedView, 解析好的句柄)`（`EntityRef<C>` 变成 `Entity<C>`，`NodeRef` 变成节点 id，元组、数组、`Vec` 逐项解析；有句柄的元素没建出来时挂载报 `InvalidInput`、什么也不留），之后照常 `update_component`。`El<C>` 只收 `EntityRef<C>`，类型写错编译不过。句柄是信号，在挂载闭包里创建，才归这个挂载所有。`.on_cx::<E>(|组件, 事件, cx| …)` 的处理器拿到组件和 `ViewContext`，可以原地改组件、`cx.emit(..)`、`cx.dispatch_program(..)`；模板里把 `on:E={…}` 写成三个参数的闭包就是它。要先建好再交给别人放置（组合控件按 id 收的 slot），用 `mount_view_detached`；只有一个根的视图，`view.root::<C>()` 直接拿到它。
- **从数据出发**：`list.each(key, row)` 就是 `each(list, key, row)`，`cond.then_show(|| v).otherwise(|| w)` 就是 `when(cond, || v).otherwise(|| w)`（不叫 `show`：Vue 的 `v-show` 保留节点，对应的是 `.visible(..)`）。

- **属性**接受常量、`Signal<T>` / `Computed<T>`、`Fn() -> T` 闭包三种。常量在建节点时写进去，之后没有任何成本。信号直接绑定，只存一条"信号 id + 字段写入函数"的记录，没有闭包。闭包装箱一次。
- **控件属性**由 `view/controls.rs` 里的 `props!` 宏按字段生成 setter（`button(..).disabled(..)`、`slider(..).value(..)`），每个 setter 对应一个 `FieldWrite`。任意控件都能用 `widget(component).bind(|c| …)` 和 `.on::<E>(|e| …)`，自定义字段写 `FieldWrite` 后用 `.prop::<T, W>(..)`。
- **key** 只在 `each` 的元素和需要按路径查找（`resolve_assembly_path`）的节点上写。静态结构只建一次，没写 key 的节点按位置命名（`#v0`、`#v1`…）。
- **`mount_view` 接收闭包。** 视图表达式在挂载作用域里求值，组件函数里创建的信号归这个挂载，`unmount`、销毁根节点、或 `AppContext` 被丢弃时一起回收。`each` 的每一行、`when` 的每个分支各有一个子作用域，行被删掉、分支被切走时只回收它自己的信号和副作用。
- 在任何作用域之外创建的信号（例如应用启动时的全局状态、事件处理器里新建的信号）不归任何挂载，会一直存在到线程结束。列表数据里每行需要的信号，应该在行视图里创建，或者由持有它的挂载创建。
- `each` 里 key 重复时，只保留第一个，后面的重复项跳过。
- 在视图构造阶段直接 `count.get()` 读出来的值是常量，不会跟着变。要跟着变，就传信号本身或者传闭包。这和 Vue `setup` 里读 `.value` 的规则一样。

## Store：按字段追踪的嵌套状态

`Signal<App>` 里任何一处变了，读它的地方全部重跑。`store(value)` 把值放在一处，但每条路径各自追踪：

```rust
#[derive(Clone, Store)]            // Store 派生在 view-macro feature 下
struct Todo { id: u64, title: String, done: bool }
#[derive(Store)]
struct App { todos: Vec<Todo>, filter: String }

let app = store(App { todos, filter: String::new() });
let todos = app.todos().keyed(|t| t.id);            // 列表按 key 定位行
todos.each(|t| row().gap(4).children((text(t.title()), checkbox("").checked(t.done()))));

todos.at(&7).done().set(true);     // 只有第 7 行的复选框更新
app.todos().push(todo);            // 列表加一行，已有的行一个都不重读
app.filter().set("done".into());   // 只影响读 filter 的地方
```

- `#[derive(Store)]` 生成 `<Name>StoreFields` trait，每个字段一个访问器，返回 `Subfield`；用到访问器的地方要导入这个 trait。只支持具名字段、不带泛型的结构体。
- 路径句柄（`Store`、`Subfield`、`Item`）和信号一样是 `Copy` 的 id，可以直接当属性值：`text(t.title())`、`.checked(t.done())`。读写方法 `get` / `with` / `try_with` / `set` / `update` 来自 `StorePath` trait。
- 每条路径有两个触发器，都在第一次被追踪读取时才创建，没被读过的路径不占任何信号。读值（`get` / `with`）追踪 deep；遍历列表（`keyed(..).each` / `items()`、`len()`）只追踪 shallow。写一条路径触发它自己和它下面已存在路径的两个触发器，以及它上面各路径的 deep。
- 列表自己的 `push`、`insert`、`retain`、`swap`、`sort_by_key` 不改任何一项的内容，所以只触发列表本身和上层的 deep，不触发各行。整体 `set` / `update` 列表会触发所有行，行内绑定重跑后按字段比较，值没变就到此为止。
- 行按 key 的 64 位哈希定位，重排后仍指向同一项；同一列表里两个 key 的哈希不能相同。行被删掉后，它的触发器在下一次按 key 查找时释放；对已删除行的 `get` 会 panic，`try_with` 返回 `None`。
- **时间旅行**：`store_with_history(value, 上限)` 记住之前的值，`undo()` / `redo()` / `travel(±n)` 前后移动，`can_undo()` / `can_redo()` / `steps()`（每一步写在哪一行）是被追踪的，可以直接绑到按钮上。一次事件处理（两次 flush 之间）里的所有写入算一步；开始一步时复制整个值（要求 `T: Clone`，成本和值的大小成正比）。撤销和重做通过整值写回，所以只有值真的变了的绑定会更新，行按 key 保留；撤销之后再写入，就丢掉可以重做的步骤。
- `.vue` 里 `let x = store(…)` 和 `Store` / `Subfield` / `Item` 类型的 prop 被当作 store：用到它的绑定一律在运行时追踪，不会被折叠成常量；单独写这个名字（例如 `:checked="done"`，`done` 是一个 `Subfield` prop）则直接绑定。

## 异步：任务、`resource` 与 `suspense`

信号只能在创建它的线程上用，所以要写信号的异步代码也跑在 UI 线程上：

```rust
let user = resource(move || id.get(), |id| spawn_blocking(move || load_user(id)));
suspense(
    || text("加载中…"),
    move || text(move || user.with(|u| u.map_or(String::new(), |u| u.name.clone()))),
)
```

- `spawn_local(future)`：在 UI 线程的执行器上运行一个 future（不要求 `Send`，可以直接读写信号）。它的 waker 在任何线程上被唤醒，都会通过宿主的唤醒钩子把事件循环叫起来，在 UI 线程上 `poll_tasks()`；每帧开头（`take_system_work`）也会轮询一次。构建视图时创建的任务归这个视图的作用域，视图卸载时一起丢弃。返回的 `Task` 可以 `abort()`。
- `spawn_blocking(f)`：把阻塞工作放到新线程上，返回它结果的 future。
- `resource(source, fetch)`：`source` 像副作用一样被追踪，每次变化都调用 `fetch(source)` 并丢弃上一次还没完成的 fetch。`get()` / `with()` 读最近一次结果（第一次完成前是 `None`，重新加载期间保留旧值），`loading()` 表示是否在加载，`refetch()` 用当前来源重新加载，`set(v)` 直接改值（乐观更新）。
- `suspense(fallback, content)`：`content` 立刻构建但隐藏，直到在它里面创建的所有 `resource` 都完成了第一次加载，期间显示 `fallback`；之后的重新加载不再切回 `fallback`。模板里写 `<Suspense fallback={..}>`，`.vue` 里用 `<template #fallback>`。
- 宿主接线：`nana-ui` 的窗口宿主在启动时调用 `set_task_wake`，并在处理宿主工作时轮询任务、给有待应用绑定的窗口请求重绘。没有宿主（测试、嵌入）时自己调用 `poll_tasks()`。

## 样式表

样式表就是 CSS，用的是 Vue 路径同一个 CSS 子集（[布局](layout.md)），由同一个引擎 `nana-ui-css` 处理。区别在于什么时候做：Vue 路径在运行时解析和级联，L3 视图在**构建时**解析和匹配，运行时只拿到结果。

```rust
view! {
    <style>
        .todos { opacity: 1; transition: opacity 120ms ease-out; }
        .todos.empty { opacity: 0.6; font-size: "1.1em"; }
    </style>
    <Column class="todos" class:empty={list.with(Vec::is_empty)} gap=8>…</Column>
}

// Rust 写法：同一份 CSS 放进 stylesheet!，类是常量
stylesheet! {
    mod todo_styles;
    .todos { opacity: 1; transition: opacity 120ms ease-out; }
    .todos.empty { opacity: 0.6; font-size: "1.1em"; }
}
column()
    .gap(8)
    .class(todo_styles::todos)
    .class_when(todo_styles::empty, move || list.with(Vec::is_empty))
```

- **一条路径**：模板的 `<style>` 编译成一张 `Sheet`，`class="a"`、`class:a={条件}` 展开成 `.class(..)`、`.class_when(..)`，和 `stylesheet!` 加手写调用完全一样。`stylesheet!` 为每个类生成一个 `Class` 常量（`-` 换成 `_`，`.todo-item` 是 `todo_styles::todo_item`），类名写错就是编译错误。`mod x;` 的常量只给父模块用，`pub mod x;` 是公开的。一个元素的类来自同一张表。

- **`view!` 里的 CSS 是 Rust token**：`<style>` 放在模板最前面，里面直接写 CSS。空格按 token 在源码里的位置还原，所以 `.a.b`（复合）和 `.a .b`（后代）不会混。Rust 词法写不出的值放进双引号，编译时去掉引号原样拼接：`em` / `ex` 单位（`"1.5em"`；`1em` 会被 rustc 当成缺指数的浮点数，在宏展开之前就报 `expected at least one digit in exponent`）、数字后紧跟 `e` 的十六进制颜色（`"#9ecafe"`）。CSS 里的单引号字符串改用双引号；`//` 只能写在引号里。`url("…")` 里的字符串和选择器里的字符串保留引号。
- **检查**：每条警告都落在它说的那段 CSS 上：不支持的选择器和 at-rule 落在选择器上，Style Model 没有对应字段的声明落在那条声明上，没有规则用到的类落在元素的 `class` 属性上。`view!` 里是那个 token（编辑器里的波浪线就在那里），`.vue` 里是文件的行列。
- `.vue` 的 `<style>` 是原样的 CSS 文本，不受上面的词法限制；两种写法编译出同样的补丁。

- **写法**：`class="a b"` 是固定的类；`class:名字="条件"` 是条件为真时才有的类（Rust 表达式写不出 Vue 的 `{ active: x }` 对象语法，所以用 Svelte 的写法）。函数 API 用 `.css(css! { padding: 12px; opacity: 0.8 })` 给单个元素写一段声明，写法规则和 `<style>` 相同。
- **编译**：`.a` 和 `.a.b` 这样的类选择器，按 `!important`、特异性、源码顺序排好级联。每条规则用 `nana-ui-css` 把声明施加到一份默认布局上，改动了的 Style Model 字段就是这条规则的补丁，以 JSON 数据嵌进程序；`var()` 按样式表自己的自定义属性在构建时求值。样式表成为一张按级联顺序排好的"需要哪些类 → 补丁"的表。
- **运行时**：不解析 CSS，`nana-ui-runtime` 里也没有 CSS 代码。一个元素第一次以某组"固定类 + 条件类"出现时，样式表从表里挑出这组类可能命中的规则（比较的是类的编号），之后同一组类直接复用。再按"基础布局 + 当前生效的条件类"合成一次，结果是一份共享的布局，同一组类的所有实例（包括 `v-for` 的每一行）都只拿它的引用。条件类的条件是普通绑定，变化时换一份合成结果。
- **`transition`**：编成隐式动画（`El::animate`）。绑定改变了 `opacity`、`transform`、`width`、`height` 或 `background` 时，在合成器轨道上从当前显示的值播到新值，逻辑样式直接取新值。只认元素固定类上的 `transition`。
- **不编译、会报警告的**：其他选择器（标签、id、组合器、属性）、`:hover` / `:focus` / `:active`、`@media`、`@keyframes` 和 `animation`、`@font-face`、伪元素、Style Model 里没有对应字段的声明、元素上没有规则用到的类、绑定式的 `:class`。
- **主题色**：样式表的颜色在构建时按亮色主题求值，所以随主题变化的颜色写成语义角色：每个元素都有 `.foreground(..)`、`.background(..)`、`.border(..)`（`SemanticColorRole`）和 `.radius(..)`（`RadiusTier`），可绑定；模板里是同名属性，例如 `<Text foreground={SemanticColorRole::Muted}>`。
- **已知取舍**：补丁只记录"和默认值不同"的字段，所以把属性写回默认值（例如 `position: static`）不会覆盖元素原来的非默认值；颜色在构建时按亮色主题求值，跟随主题切换的颜色请用组件自带的语义色。

## 进出场与移动动画

`when(..)` 和 `each(..)`（包括 `Store` 的 `keyed(..).each`）可以加 `.transition(t)`，对应 Vue 的 `<Transition>` 和 `<TransitionGroup>`：

```rust
when(open, panel).transition(Transition::fade(ms(150)));
each(items, |t| t.id, row).transition(Transition::slide(0.0, 12.0, ms(180)).moves(ms(200)));
```

- **进场**：新行、新分支从给定的透明度和变换播到节点自己的值。挂载时已经存在的行不播。
- **离场**：被删掉的行或分支不立刻销毁。它的作用域马上回收（不再跟随数据），节点留在原位，不参与命中测试，焦点移走，播完离场后才销毁。
- **移动**（`.moves(时长)`）：位置变了的行从原来的位置滑过去（FLIP）。插入、删除、重排都算，离场的行最终销毁、后面的行补位时也算。
- 预设：`Transition::fade`、`slide(dx, dy, 时长)`、`scale(比例, 时长)`；自定义用 `Transition::new().enter(Presence::new(时长).opacity(0.0).translate(0.0, 8.0)).leave(..)`；`.ease(e)` 统一缓动，`without_enter()` / `without_leave()` 只要一半。
- 全部走节点的合成器轨道，不写回逻辑样式；离场结束由 `advance_animations` 的完成事件驱动，移动在布局阶段之后、同一帧提取之前开始，所以不会先闪到新位置。
- `when` 的新旧分支同时存在：旧分支原位离场，新分支接在它后面进场；加 `.moves` 后，旧分支消失时新分支平滑上移。Vue 的 `mode="out-in"`（先离场再进场）目前没有。
- 虚拟列表（`each_virtual`）不支持：滚出视口的行本来就要立刻回收。
- 模板：`<Transition name="fade" duration="150">` 包住一条 `v-if` / `v-else-if` / `v-else` 链，`<TransitionGroup duration="150" move="200">` 包住一个 `v-for` 元素（两个标签可以互换）。`name` 可选 `fade`、`slide-up`、`slide-down`、`slide-left`、`slide-right`、`scale`，默认 `fade`；时长单位是毫秒；`:transition="值"` 直接传一个 `Transition`。

## 组合控件的 slot

`DesktopShell`、`SidebarSection` 这类组合控件按节点 id 接收应用内容。视图用 slot 把一段视图交给它们，不用先建节点再拿 id：

```rust
widget(DesktopShell::from_model(model).title("Gallery"))
    .title_trailing(row().gap(6).children((search, theme)))   // 具名 slot，底下是 .slot(..)
    .navigation(sidebar())
    .primary(page())
```

模板里写成 `<template #name>`，展开成同样的方法调用：

```rust
view! {
    <Widget of={DesktopShell::from_model(model).title("Gallery")}>
        <template #title-trailing><Row gap=6>…</Row></template>
        <template #navigation>{sidebar()}</template>
        <template #primary>{page()}</template>
    </Widget>
}
```

- **`.slot(view, |c, id| c.xxx(id))`**：slot 由控件自己放置（shell 的区域、标题栏的三列）。先建 slot 视图，不插到任何地方，再把根节点 id 写进组件，最后建组件本身。
- **`.child_slot(view, write)`**：slot 是本节点自己的子节点（section 的 header、list item 的图标），按 slot 的声明顺序插在 `.children(..)` 前面。
- slot 必须恰好有一个根节点，否则挂载失败，返回 `InvalidInput`，这次什么也不提交。slot 和组件在同一次提交里建好，slot 里创建的信号归同一个挂载作用域，卸载时一起回收。
- **装配**：需要装配的控件在 `TypeBehavior::slot_assembler` 里登记自己的 `assemble_*`。视图层在建好这类节点之后，以及绑定改了它之后，自动运行装配，所以视图里不写 `assemble_desktop_shell`。builder 仍然由调用方自己调用 `assemble_*`：在每次写入时都装配会弄脏空闲的 world，见 `TypeBehavior::assembler`。只由自身属性决定子节点的叶子组合控件（`Chip`、`ColorField`、`PathField`、`FileTab`、`DiffView`、`MediaTransportBar`）登记的是 `TypeBehavior::assembler`：它本来就在每次写入后运行，视图层在建好这类节点时也运行一次，所以它们不必再登记 `slot_assembler`。
- slot 的节点在视图的整个生命周期里不换：里面的内容要变，就在 slot 里用 `when` / `each` / `dynamic`；区域显示还是隐藏，由 `WorkspaceModel` 决定。
- 目前登记了装配的有 `DesktopShell`、`AppShell`、`AppTitleBar`、`Workspace`、`Dock`、`SplitPane`、`PaneSection`、`GraphCanvas`、`DatePicker`、`NativeMarkdown`、`ConfirmDialog`、`SettingsRow`、`SegmentedControl`（及其 `SegmentedOption`）、`SidebarSection`，以及设置页的 `AppearanceSection`、`AboutSection`、`SettingsSidebar`、`SettingsCollapsibleCard`（`.summary` / `.details` / `.accessory`）和 `SettingsPage`（`.content`）；叶子组合控件（`Chip`、`ColorField` 等）的 `assembler` 在视图建好时也会运行。具名 slot 方法：`SplitPane` 的 `.first` / `.second`、`PaneSection` 的 `.header` / `.tabs` / `.body`（`view/panes.rs`）；`DesktopShell` 的各区域、`AppShell` 的 `.title_bar` / `.body` / `.overlay`、`Workspace` 的 `.region(id, view)`、`AppTitleBar` 的 `.leading` / `.center` / `.trailing`（`view/shell.rs`）；`SidebarSection` 的 `.tools`、`SidebarFrame` 的 `.top` / `.body` / `.footer`（`view/sidebar.rs`）；设置行的 `.control`（`view/settings.rs`）；`ListItem` 与 `SidebarRow` 的 `.leading` / `.content` / `.trailing`（作为它们自己的子节点，按这个顺序写）；模态面板 `Dialog`、`Drawer` 的 `.body` / `.footer`，以及 `MediaTransportBar` 的 `.leading` / `.trailing` / `.secondary`：把应用的控件放进它自己的按钮组和第二行（`view/composites.rs`）。其他组合控件可以先用通用的 `.slot` / `.child_slot`，并自己调用 `assemble_*`。
- **Dock 面板**：面板按布局里的 id 对应，数量由数据决定，所以不用具名 slot，而是用子节点的 key：`widget(Dock::new(layout)).children((files().key("files"), preview().key("preview")))`，模板里写 `<Widget of={dock}><FilesPanel key="files"/>…</Widget>`。装配时每个面板 id 找同名 key 的子节点作为内容；布局被绑定换掉后按 key 重新对应，面板节点不重建。key 对不上任何面板的子节点会被挂起。
- **侧栏分组**：`widget(SidebarSection::new("资源").count(2).collapsible(true)).tools(…).children(行)`。分组自己建表头（展开按钮、标题、计数）和 body，应用的行（包括 `each` 的列表）被挪进 body，`tools` 放在表头里悬停时显示；建出的结构和手工拼出的分组逐节点相同。`SidebarFrame` 的 `.body(..)` 自动把内容放进它的纵向滚动区域；slot 按 top、body、footer 的顺序写。
- **分段选择**：`segmented().children((segmented_option("面捕").selected(..).on_select(..), …))`（`view/selection.rs`）。选项就是控件的子节点，`label`、`disabled`、`selected` 可绑定；控件按子节点和 `selected` 标记登记选项与选中项（`assemble_segmented_control`），任一选项的绑定变了也会重新登记。用户选中一项时，控件先发 `SegmentedSelectionRequested`，再在那一项上发 `SegmentedOptionChosen`，`.on_select` 听的是后者。要隐藏的选项仍留作子节点，同时把它禁用。
- **设置行**：`settings_row(label).hint(..).divided(..).control(switch(""))`。行自己建标签和说明（`assemble_settings_row`），结构和 `mount_settings_leaf_row` 建出的行逐节点相同；`label`、`hint`、`divided`、`stacked`、`first_in_group`、`last_in_group` 都可绑定，`.visible(..)` 控制显隐。分组里哪一行是首行、末行，由应用按自己的数据算好再绑定。

## `view!` 与 Rust 写法

`view-macro` feature 提供 `view!`（`nana_ui::runtime::view!`）。它是 Vue 模板写法的对应，展开结果就是 Rust 写法的调用，不增加任何运行时概念。宏在独立的 proc-macro crate `nana-ui-view-macros` 里，不开 feature 就不参与编译。

```rust
fn todos() -> impl IntoView {
    let draft = signal(String::new());
    let list: Signal<Vec<Todo>> = signal(Vec::new());
    let add = move || { /* … */ };
    view! {
        <Column gap=8>
            <TextInput placeholder="新任务" v-model={draft} />
            <Button disabled={draft.with(|d| d.trim().is_empty())} @activate={add}>"添加"</Button>
            <TodoRow v-for={todo in list} key={todo.id} todo={todo} list={list} />
            <Text v-if={list.with(Vec::is_empty)}>"还没有任务"</Text>
            <Text v-else>{format!("共 {} 项", list.with(Vec::len))}</Text>
        </Column>
    }
}
```

| 模板 | Rust 写法（也就是展开结果） |
| --- | --- |
| `<Column gap=8>…</Column>` / `<Row>` | `column().gap(8_f32).children((…))` / `row()…`；数字字面量带上类型后缀，表达式原样传入 |
| `<Text>"计数 {count}"</Text>` | `text!("计数 {count}")`；没有 `{…}` 的字符串是 `text("…")` |
| `<Button>"加一"</Button>`、`<Checkbox>` | `button("加一")`、`checkbox(…)` |
| `<Slider min=0 max=1 step=0.05/>`、`<TextInput/>` | `slider(0_f64, 1_f64, 0.05_f64)`、`text_input()` |
| `<Widget of={component}>…</Widget>` | `widget(component).children((…))` |
| `<TodoRow todo={t} list={list}/>`（其他标签） | `todo_row(t, list)`：标签名转成 snake_case，属性值按书写顺序作为参数，子节点作为最后一个参数 |
| `name="x"`、`name=3`、`name={x}`、`name={a.b}` | 原样传入：常量，或者信号本身 |
| `name={其他表达式}` | `move \|\| 表达式`：读到的信号变了就重算 |
| `name={\|\| …}` | 闭包原样传入 |
| `@activate={表达式}` | `.on_activate(move \|\| { 表达式; })`；`@activate={add}` 直接传函数值 |
| `on:RangeChanged={\|e: &RangeChanged\| …}` | `.on::<RangeChanged>(…)`；写成 `\|组件, 事件, cx\|` 三个参数时是 `.on_cx::<…>(…)` |
| `v-if` / `v-else-if` / `v-else`（兄弟节点） | `when(…).otherwise(…)`，`v-else-if` 嵌套在 `otherwise` 里；手写也可以 `cond.then_show(…)` |
| `v-for={pat in items} key={…}` | `each(items, move \|item\| { let pat = item; key }, move \|pat\| 元素)`；手写也可以 `items.each(key, row)` |
| `v-show={x}`、`v-model={sig}`、`key="x"` | `.visible(x)`、`.model(sig)`、`.key("x")` |
| `<style>…</style>`、`class="a"`、`class:a={c}` | `stylesheet! { mod s; … }`、`.class(s::a)`、`.class_when(s::a, c)` |
| `<template #navigation>…</template>`（具名 slot） | `.navigation(…)`：元素上同名的方法，`#title-trailing` 是 `.title_trailing(…)`；`#default` 就是普通子节点 |

多于 12 个子节点时，宏会嵌套成多层 tuple。写错的地方会被准确指出，例如：未闭合的标签、`</Row>` 关了 `<Column>`、`v-else` 前面没有 `v-if`、`v-for` 没写 `key`、未知指令、`v-if` 和 `v-for` 写在同一个元素上。`tests/view_macro.rs` 把同一个页面用模板和手写函数调用各写一遍，挂载后的保留树逐节点相同，改完信号 flush 之后也相同。

在宏里能静态区分的只有三类：常量（字面量）、直接值或信号（路径、字段）、闭包（其他表达式）。"静态依赖"那一类需要知道哪个标识符是信号，只有能看到整段脚本的 [`.vue` 方言](#高级用法vue-方言)编译器才能做到。

## 高级用法：`.vue` 方言

日常写视图用 `view!` 或 Rust 写法。`.vue` 文件适合两种情况：从 Vue 迁移、想逐文件对照着改；或者希望模板、脚本、样式分在独立文件里。它编译出来的代码和 `view!` 走同一个代码生成器、同一套 Rust 调用，能力上没有多出来的东西；多出来的是编译器能看到整段脚本，所以能做下面的依赖分析和常量折叠，代价是脚本和模板表达式没有 rust-analyzer 支持。

视图也可以写成 `.vue` 文件：模板是 Vue 语法，`<script setup lang="rust">` 里写 Rust。`nana-ui-sfc` 在构建期把它们编译成普通的 Rust 函数，模板部分和 `view!` 共用 `nana-ui-view-codegen` 这一个代码生成器。

```vue
<!-- views/TodoList.vue -->
<script setup lang="rust">
let draft = signal(String::new());
let list: Signal<Vec<Todo>> = signal(Vec::new());
let add = move || { /* … */ };
</script>

<template>
  <Column :gap="8">
    <TextInput placeholder="新任务" v-model="draft" />
    <Button :disabled="draft.with(|d| d.trim().is_empty())" @activate="add">添加</Button>
    <TodoItem v-for="todo in list" :key="todo.id" :todo="todo.clone()"
              @remove="list.update(|l| l.retain(|t| t.id != todo.id))" />
    <Text v-if="list.with(Vec::is_empty)">还没有任务</Text>
    <Text v-else>共 {{ list.with(Vec::len) }} 项</Text>
  </Column>
</template>
```

```rust
// build.rs
fn main() {
    if let Err(error) = nana_ui_sfc::Compiler::new("::nana_ui::runtime").build("views") {
        panic!("{error}"); // views/TodoList.vue:20:34: …
    }
}

// src/lib.rs
pub mod views {
    use crate::model::Todo;
    include!(concat!(env!("OUT_DIR"), "/nana_views.rs"));
}
```

- `TodoList.vue` 编译成 `pub fn todo_list(…) -> impl IntoView`。`defineProps!(todo: Todo, on_remove: impl Fn() + Send + 'static)` 声明参数。
- 属性写法和 Vue 一致：`name="x"` 是字符串，`:name="表达式"` 是 Rust 表达式，`{{ 表达式 }}` 用 `Display` 插值。
- 同一批编译的组件之间按 prop 名匹配参数，不按书写顺序：`@remove` 对应 `on_remove` 回调，子节点对应最后一个名为 `children` 的 prop。缺少的参数、多余的参数都是编译错误。
- 具名插槽：子组件声明 `header: impl IntoView` 这样的视图参数，模板里用 `<slot name="header"/>` 放置（`<slot/>` 放 `children`）；父组件写 `<template #header>…</template>`，`#default` 等于其余子节点。插槽内容是按值传入的视图，只能放一次，也没有后备内容。
- `ref="name"` 把元素的节点 id 写进脚本里的 `let name = node_ref();`；`on_mount(move |cx| …)` 在视图进树之后执行，可以拿它聚焦、读布局。
- 组件上的 `key` 落在组件的第一个根节点上（`keyed`）。不在这一批里的标签，按 `view!` 的规则调用同名的 Rust 函数。
- `<style>`（写不写 `scoped` 都一样，总是只作用于本组件）在构建时编译，见下文"样式表"。模板里仍然要遵守 Rust 的所有权规则，例如同一个值既要传给组件又要被事件闭包使用时，得写 `todo.clone()`。
- 生成的代码用 prettyplease 排版后写进 `$OUT_DIR/nana_views.rs`，rustc 的报错会指向可读的代码。模板和脚本本身的错误（语法、标签不配对、缺 `key`、缺参数、computed 成环）在构建时报出，带文件、行、列。

开发期热重载：构建脚本写 `Compiler::new(..).hot(debug)`，只改 `.vue` 里的静态文字时，`nana-ui-dev` 的 `watch_templates` 把新文字送进正在运行的窗口，不重建；改了别的就照常重建。见 `docs/hot-reload.md`。

### 编译器能看到什么

编译器能看到整个组件，所以知道哪些名字是信号、每个信号怎么被使用。依赖分析在脚本和模板上扫描 token：

- 调用 `get` / `with` 算读，`set` / `update` 算写，其他任何用法（作为参数传出、被闭包按值取走、调用别的方法）都算"传出"；
- 看不见内部的调用（自由函数、`format!` 之外的宏、在不认识的接收者上调用 `get`）会让这个表达式变成"动态"。

扫描看不到变量遮蔽，所以所有不确定的情况都往"不优化"那边算。

| 分析结果 | 编译器怎么做 |
| --- | --- |
| 信号从未被写入、也没有传出 | 折叠成 `constant(…)`：`Copy` 句柄和读取 API 都不变，但读取不建立依赖，作为属性时只写一次 |
| computed 只读折叠后的常量 | 也折叠：`constant((f)())`，只计算一次；可以连锁折叠 |
| 绑定只读常量或普通值 | 包成 `Fixed(值)`，建节点时写一次，不创建副作用 |
| 绑定通过编译器看得见的代码只读若干信号（静态依赖） | 包成 `__checked(位置, [依赖…], 闭包)`：debug 构建里每次执行都核对"实际读到的信号 ⊆ 声明的依赖"，不一致就报 `runtime.reactive.static_deps_mismatch` fault，并计入 `ReactiveStats::static_deps_mismatches`；release 构建里原样返回闭包 |
| 其他 | 动态：运行时追踪 |
| computed 之间成环 | 编译错误 |
| 信号有写入但没人读 / `watch_effect` 写了自己读的信号 | 通过 `cargo:warning` 报警告 |

每个组件的信号（读、写、传出、是否折叠）和每个绑定的位置、分类、依赖，都写进 `$OUT_DIR/nana_views.deps.md`。

静态依赖目前**只在 debug 里校验，不用来跳过运行时追踪**：一次"值没变"的节点重跑总共约 70 ns，依赖追踪只是其中一部分；release 的正确性不押在 token 扫描上。`examples/reactive-sfc` 的测试断言校验失败数为 0。

## 控件级摇树：`BuiltinComponents::Typed`

`AppContext::new()` 会注册全部 91 个内置控件，每条注册都带着"从标签和属性构造这个控件"的绑定器和重投影函数，所以不管程序用没用到，全部控件的代码都会被链接进来。只从 Rust 类型创建控件的程序（`build`、`mount`、声明式视图、`.vue`）用不到这些绑定器，可以改用精简模式：

```rust
impl ApplicationState for App {
    const BUILTINS: BuiltinComponents = BuiltinComponents::Typed;
    // …
}
// 不经过宿主时：AppContext::typed()、RuntimeDocument::typed(document)
```

- 内置控件只注册**标识**（类型名、标签、TypeId）。节点照常带上组件类型标记，按标签也能解析出类型名。
- 每个类型的行为写在 `ComponentView::BEHAVIOR`（`TypeBehavior`：激活、组装器、在某一点上激活、关闭弹出的选项、写入后的维护、输入路由钩子 `TypeHooks`），在该类型第一个节点被创建时安装。所有创建路径都经过这一步（`stamp_component_type`、复合控件的 `install_view`、按标签绑定的 `finish_semantic_binding`），程序没创建过的类型，这些代码一处都不被引用。输入路由经 `hooked!` 调用子系统，钩子没装时的结果和文档里没有这种节点时一样。
- 模式必须在编译期确定：`ApplicationState::BUILTINS` 是关联常量，宿主按常量选择，另一个分支在 release 里被删掉；在运行时二选一会把两种模式都链接进来。
- 精简模式下，按标签构造内置控件（`bind_semantic`，也就是 Vue 路径）返回 `FrameworkError::InvalidComponentType`。Vue 宿主继续用完整模式。

实测（release、strip 之后）：

| 程序 | 完整 | 精简 | 省下 |
| --- | --- | --- | --- |
| 最小无头程序（一列文字和一个按钮） | 5.40 MB | 2.28 MB | 3.12 MB（-58%） |
| 托管窗口应用 `examples/reactive-sfc` | 35.48 MB | 33.42 MB | 2.06 MB（-5.8%） |

托管应用的体积主要来自 GPU、文字和内置字体，控件只是其中一小部分。

验证：`NANA_TEST_TYPED_BUILTINS=1 cargo test -p nana-ui-runtime` 让本 crate 单元测试里的 `AppContext::new()` 返回精简上下文（只存在于 `cfg(test)`），需要按标签构造的测试会跳过。创建路径漏装行为时，对应的测试在这个模式下失败。

## 语义

| 环节 | 行为 |
| --- | --- |
| 挂载 | 整棵声明树在一次 `build_detached` 里建完；挂到父节点下再多一次插入 commit。根节点无 key 插入，不影响父节点自己的 keyed 子节点 |
| 写信号 | `set` / `update` 只把订阅它的副作用排进队列，不碰树；`computed` 只标脏，下次被读时才重算 |
| 等值截断 | 写入只把直接读者标脏，更下游标"待查"。待查的 computed 或副作用先把它依赖的 computed 更新一遍，只有其中某个算出了**不同的值**才重跑（`computed` 要求 `T: PartialEq`）。所以 `computed(\|\| n.get() % 2)` 在 1 → 3 时，读它的绑定一个都不跑 |
| flush | `AppContext::flush_reactive`。按轮执行：先跑 `watch_effect`，再跑 `each` / `when` 的结构更新，最后把这一轮所有需要改的节点各暂存一次、合进**一次** commit。输入路由在每个事件末尾调用它（所以 `InputRouteOutcome::invalidated_work` 会反映绑定的变化），`take_system_work` 在每帧开头调用它 |
| 节点绑定 | 同一个节点的所有动态字段共用一个副作用。任何一个输入变了，先按字段逐个与保留的视图比较（`FieldWrite::differs`），全部相等就到此为止：不复制、不投影、不提交。只要有一个字段不同，才复制一份、写入、投影一次 |
| `each` | 用 key 对照：保留的行不重建，节点 id 和控件的交互状态都不变；删掉的行回收作用域并销毁节点；新行在一次 detached build 里建好。重排只移动最长递增子序列之外的节点：插入或删除一行不移动任何已有节点，整体反转移动 n−1 个。一行里的字段变化应该用行内信号或 `Store`，这样不会触发列表重算 |
| `each_virtual` | `each_virtual(items, key, 行高, row)`：行放在一个纵向 `ScrollView` 里，只建视口（加 overscan）盖到的行，底层是 Runtime 已有的保留式虚拟列表。滚走的行连同作用域一起回收，持有焦点或输入法组合的行保留。数据变化、滚动（`ScrollChanged`）和视口尺寸变化（`ScrollViewportChanged`，布局后发出）都会移动窗口。5 万行：挂载加布局加出窗口 2.4 ms，`each` 要 175 ms、每行常驻约 2.3 KB。行高默认固定；`.measured()` 让行按内容量高，给定的行高只作估计：新行出现后，下一次布局（`ScrollLaidOut`）量出真实高度并重新放置，视口顶部那一行保持不动。模板里写 `v-for` 加 `v-virtual="行高"`，按内容量高写 `v-virtual.measured`；要定滚动区域的尺寸，把 `v-for` 元素包进 `<Virtual row-height="24" height="400" measured>`（另有 `width`、`grow`、`overscan`，`:scroll` 传入一个自己配置的 `ScrollView`）。函数 API 对应 `.height()`、`.width()`、`.grow()`、`.scroll_view()`。列表在页面主滚动之内（评论区、页头下的分区）时用 `.within(页面的 NodeRef)`：不另建滚动区域，列表直接放在当前位置，窗口取祖先 `ScrollView` 视口里被列表盖住的那一段；列表上方的内容变高变矮时，下一次布局后窗口跟着移动。`.grid(最小列宽, 间距)` 把项目按列表宽度能放下的列数排成网格，给定的行高是一行网格的高度（不含间距），列等分宽度，最后一行不满时空出的列保持对齐；宽度变化时按新的列数重排。按内容量高时，数据变化（例如翻页追加）保留已量过的行高，不回到估计值。模板里 `<Virtual within={page} grid=220 gap=12>` |
| `when` | 条件变了才动：旧分支回收作用域并销毁，新分支建好后插入。`.visible(sig)` 则保留节点，只切 `layout.hidden`（对应 `v-show`） |
| 保活 | `when(..).keep_alive()`：没显示的那个分支连同节点和状态保留，切回来时原样出现（Vue `<KeepAlive>`）。保留的分支移进容器里一个隐藏的 `Stack`（不参与布局、绘制和命中测试，焦点会移走），容器销毁时一起销毁。`dynamic(key, render)` 按 key 显示一个视图（Vue `<component :is>`），`.keep_alive()` 保留之前 key 的视图，`.max(n)` 最多保留 n 个、先丢最久没显示的。模板里 `<KeepAlive>` 包住一条 `v-if` 链，可以和 `<Transition>` 互相嵌套；保活的分支切走时不播离场，切回来时播进场 |
| 传送 | `teleport(to, content)`：`content` 在声明处构建、归声明处的作用域，跟着声明处一起销毁，但在树里挂到 `to` 下面（Vue `<Teleport>`），所以布局、绘制、命中测试、焦点顺序和无障碍都按 `to` 的位置。`to` 可以是 `NodeRef`、节点 id 或选出节点的闭包，变化时跟着移动；为 `None` 或目标不在时留在原处。底层是 `place_assembled`。模板里写 `<Teleport :to="layer">` |
| 错误 | 错误是值：`Result<V, E>`（`E: Display`）本身就是视图，`Err(e)` 什么都不建，把 `e` 报给上面最近的 `error_boundary(fallback, content)`；`report_error(e)` 手动报。边界里只要还有错误，就隐藏内容、显示 `fallback(错误列表)`。错误跟着报它的作用域走：失败的分支或行因为数据变化被丢掉，错误也就撤掉，内容原样回来。没有边界时报 `runtime.view.error_unhandled` fault。panic 是 bug，不捕获。模板里写 `<ErrorBoundary :fallback='\|errors\| …'>` |
| 回收 | 节点被销毁时（不管从哪条路径），`commit_mutations` 的清理段会回收它的绑定、结构副作用和锚定在它身上的作用域 |
| 上下文 | `provide(value)` / `use_context::<T>()`：值挂在当前作用域上，下层作用域（包括之后才建出来的行和分支）沿父链读取，最近的提供者优先。只能在构建视图时读；事件处理器运行时不在任何作用域里 |
| 出帧 | 宿主每处理完一次更新（输入、程序消息、定时器），都会检查各窗口有没有待应用的绑定；有就请求重绘，所以在 `RuntimeProgram::update` 里改信号也会出帧 |
| 线程 | 每个线程一个信号运行时。句柄是 `Copy` 的 id，可以放进 `Send` 的事件处理器；在别的线程上使用会 panic。`AppContext` 要留在创建视图的那个线程上 |

## 和 Vue 的对应

| Vue | L3 视图 |
| --- | --- |
| `ref(0)` | `signal(0)`（`ref` 是 Rust 关键字） |
| `computed(() => …)` | `computed(move \|\| …)` |
| `reactive({ … })` | `store(value)` + `#[derive(Store)]`，按字段追踪 |
| `watchEffect` | `watch_effect(move \|\| …)` |
| `{{ x }}`、`:label="x"` | `text!("{x}")`、`.label(x)`，或者 `.bind(move \|c\| …)` |
| `@click` | `.on_activate(move \|\| …)`，其他事件用 `.on(move \|e: &E\| …)` |
| `v-if` / `v-else` | `when(cond, \|\| a).otherwise(\|\| b)` |
| `v-show` | `.visible(sig)` |
| `onErrorCaptured` / `<ErrorBoundary>` | `error_boundary(fallback, content)`，视图返回 `Result`；模板里 `<ErrorBoundary :fallback>` |
| `<Teleport to>` | `teleport(to, content)`；模板里 `<Teleport :to>` |
| `<style scoped>`、`:class` | `<style>` 在构建时编译；`class="a"`、`class:a="条件"`；函数 API 用 `.css(css! { … })` |
| `<KeepAlive>` / `<component :is>` | `when(..).keep_alive()` / `dynamic(key, render).keep_alive().max(n)`；模板里 `<KeepAlive>` |
| `<Transition>` / `<TransitionGroup>` | `when(..).transition(t)` / `each(..).transition(t.moves(..))`；模板里同名标签 |
| `v-for` + `:key` | `each(items, key, row)` |
| 长列表（虚拟滚动） | `each_virtual(items, key, 行高, row)`；模板里 `v-virtual="行高"` / `v-virtual.measured`，或 `<Virtual row-height height …>` 包住 `v-for` 元素 |
| `v-model` | `.model(sig)`（文本输入、滑块、复选框） |
| props / emits | 函数参数 / `impl Fn(T)` 回调参数 |
| slot / 具名 slot | `impl IntoView` 参数；`.vue` 里 `<slot name="x"/>` 与 `<template #x>` |
| 模板 `ref` | `node_ref()` + `.node_ref(r)`；`.vue` 里 `ref="r"` |
| `onMounted` | `on_mount(move \|cx\| …)` |
| `provide` / `inject` | `provide(value)` / `use_context::<T>()` |
| `onUnmounted` | `on_cleanup` |
| 异步 `setup` + `<Suspense>` | `resource(source, fetch)` + `suspense(fallback, content)`；模板里 `<Suspense>` |
| `defineProps` | `defineProps!(name: Type, …)`（`.vue`） |
| `defineEmits` + `emit('done')` | 回调 prop `on_done: impl Fn() + …`，父组件写 `@done="…"`（`.vue`） |

同一个界面的两种写法：

```vue
<script setup>
const count = ref(0)
const draft = ref('')
const todos = ref([])
</script>
<template>
  <div class="column">
    <span>计数 {{ count }}</span>
    <NanaButton @click="count++">增加</NanaButton>
    <NanaInput v-model="draft" placeholder="新任务" />
    <span v-for="t in todos" :key="t.id">{{ t.title }}</span>
    <span v-if="todos.length === 0">还没有任务</span>
  </div>
</template>
```

```rust
fn page() -> impl IntoView {
    let count = signal(0u64);
    let draft = signal(String::new());
    let todos = signal(Vec::<Todo>::new());
    view! {
        <Column>
            <Text>"计数 {count}"</Text>
            <Button @activate={count.update(|c| *c += 1)}>"增加"</Button>
            <TextInput v-model={draft} placeholder="新任务" />
            <Text v-for={t in todos} key={t.id}>{t.title}</Text>
            <Text v-if={todos.with(Vec::is_empty)}>"还没有任务"</Text>
        </Column>
    }
}
```

除了脚本逻辑（JS 要改成 Rust），模板、响应式和样式表都可以逐行对上；样式表在 L3 里构建时编译，写法见"样式表"。和 Vue 模板仍有三处不同：

- `each` 和 `when` 各自带一个容器 `Stack`，而 Vue 的 `v-for` / `v-if` 直接生成兄弟节点。
- 表达式里的 `.get()` / `.with()` 省不掉，因为 Rust 稳定版不能给信号实现 `Fn`。只有单独出现的信号可以省，例如 `"{count}"`、`disabled={busy}`。
- 组件函数不是惰性的，只在挂载、行、分支这三个作用域边界上才划分归属。

## 成本

测量命令：`cargo run --release -p nana-ui-runtime --features benchmark --bin nana-reactive-benchmark`。每种写法交替跑 15 轮，读最小值；测的是写法层加 commit，不含布局和绘制。2026-09-29 下午本机，负载 5–6。

| 场景 | 旧写法 | 新写法 |
| --- | --- | --- |
| 挂载 1,000 个文本 | `build` 1.36 ms | 常量 1.49 ms / 每个都绑定信号 1.62 ms |
| 挂载 5,000 个文本 | `build` 7.68 ms | 常量 8.41 ms / 全部绑定 9.43 ms |
| 挂载 1,000 行（行容器加按钮） | `build` 3.87 ms | 4.33 ms |
| 5,000 个里改 1 个 | `update_component` 0.42 µs | `set` + flush 0.62 µs |
| 5,000 个里改 100 个 | 逐个 `update_component` 44.9 µs | 100 个信号一次 flush 43.7 µs / 1 个信号绑 100 个节点 38.5 µs |
| 2,000 行里插入再删除 1 行 | 整段 `mount` 重写 8.31 ms | `each` 0.058 ms |
| 2,000 行里改 1 行的字段 | 手写 `update_component` 0.41 µs / 整段 `mount` 重写 6.59 ms | 行内信号 0.59 µs |
| 100 个按钮的绑定重跑但值不变 | — | 7.41 µs |

大列表的整体操作（5,000 行，每行一个行容器、一个文本、一个按钮；前后二进制交替各跑 4 次、每次 10 轮取最小值）：

| 操作 | 优化前 | 优化后 |
| --- | --- | --- |
| 挂载 | 49.5 ms | 39.7 ms |
| 清空 | 30.3 ms | 22.8 ms |
| 往空列表里一次放入 5,000 行 | 152.7 ms | 49.8 ms |

这组数字来自对挂载过程的采样：三分之二的时间在 world 的 commit，四分之一在构建视图。据此改了四处：

- 每次把一个子树移出文档，都会把累积的待删列表整个排序去重一次，放入 n 行就是 O(n² log n)。现在只在交出系统工作时排一次。
- 作用域从父作用域的子列表里移除时要线性查找，逐行删掉 n 行是 O(n²)。现在每个作用域记住自己的位置，移除是一次交换删除。
- 一次 commit 的暂存表按节点 id 用 SipHash 哈希；改用运行时已有的整数哈希。
- 比较两份布局是否"除变换和光标外相同"时，先各复制一份再比较；现在布局是同一份就直接相等，变换等字段相同就直接比较，都不行才复制一份。`depends_on_viewport` 跳过仍是进程默认值的三组逻辑边距。

按同样的证据决定不做的几项：

- **行模板克隆**：构建视图只占挂载时间的四分之一，其中组件构造和投影每行仍然都要做，克隆静态部分能省下的不到一成。
- **分帧挂载**：5,000 行挂载约 40 ms，是两三帧；但长列表的正确做法是 `each_virtual`（5 万行 2.4 ms），分帧只会让用户看到只挂了一半的列表。
- **静态文字改成 `Cow<'static, str>`**：一个文本节点常驻 1,804 B，空 `Stack` 是 1,778 B，文字本身只占约 26 B，不值得改所有控件的 API。
- 控件注册清单也不需要单独生成：`BuiltinComponents::Typed` 已经按实际用到的类型注册。

单点更新比手写 `update_component` 多出约 0.15 µs，这是信号簿记的成本；一次改很多节点时只 commit 一次，反而更快。

第二阶段两项优化各自做了 A/B（前后二进制交替各跑 3 次，读最小值）：

- 字段级比较：绑定重跑但值不变时，21.1 µs → 7.2 µs。值确实改变时基本持平，"1 个信号绑 100 个节点"慢约 4%，原因是被改的直接绑定要读两次信号单元格。
- `reconcile_child_order` 改成最长递增子序列：`each` 插入加删除一行，1.85 ms → 0.165 ms。它也是 `mount` 和 workspace 装配共用的重排路径。

内存（`tests/reactive_view_alloc.rs`，按线程计数的分配器）：

- 5,000 个节点里，挂载常量文本平均每个节点累计分配约 11.8 KB、25 次（这是分配流量，不扣除释放）。直接绑定信号的节点比它多约 945 B、4 次分配，其中包括哈希表和 Vec 扩容摊下来的部分。
- 常驻内存（分配减去释放，`tests/node_memory.rs`）：文本、`Stack` 约 1.8 KB，`Button` 约 2.4 KB，`TextInput` 约 3.7 KB。约 2 KB 的 `LayoutStyle` 不再每个节点一份：它的大块字段组（`paint`、三组逻辑边距、网格放置）是写时复制的 `Shared`，默认值全进程共用一份；内容相等的布局在组件创建时（`ComponentView::share_layouts`）和写进世界时（最近布局缓存）共用同一个分配。此前一个 `Button` 持有三份 4.8 KB 的布局（组件自己的、投影时复制的、按圆角档位解析的），常驻 11.7 KB。
- 同一个节点再加一个直接绑定，只多 1 次分配，就是新信号的订阅表。
- 依赖不变时重新执行副作用，0 次分配；绑定重跑但字段值没变的一次 flush，也是 0 次分配。

### `.vue` 与手写函数

`.vue` 编译出来就是 `fn … -> impl IntoView`，和手写函数调用同一套 API。两者只在编译器生成的代码和人写的代码不同的地方有差别：常量折叠、构建时编译的样式表、热重载模式下的静态文字。

测量用 `examples/reactive-sfc` 的 `sfc-benchmark`，同一套 UI 写成三份：

- `bench/` 里的 `.vue`；
- 惯用手写：静态的值直接传值，单个信号直接绑定；
- 朴素手写：所有信号都在闭包里读，和 `tests/views.rs` 里的 `counter_by_hand` 一样。

`tests/views.rs` 断言各份建出的树（节点、文字、布局）完全相同。

命令：`cargo run --release -p reactive-sfc --bin sfc-benchmark`。测量条件：2026-09-29 本机，负载 3–5；每个场景交替跑 15 轮，整个程序跑 3 次，时间取最小值。内存是在 2,000 行上按线程计数的常驻内存（分配减释放），测前先建一遍 n 行，把线程级的信号表撑大。

场景：

- A：每行一个 `Row`，里面一个文本和一个按钮，文字来自 prop 和一个从没被写过的信号。
- B：在 A 的基础上加 `<style scoped>` 设 `padding` / `flex-grow`，再加一个条件类 `class:active` 设 `opacity`。手写版分别用构建器（`padding_xy`、`with_layout`）和 `css!`；条件类用一个自定义的 `FieldWrite`，因为函数 API 里没有条件类。
- C：`v-for` 列表，每行显示一个行内信号。没有朴素版：`.vue` 把 `{{ title }}` 编成闭包，本身就是朴素写法；惯用手写直接绑定 `text(title)`，两者都是每行一个副作用。

| 场景 | `.vue` | 惯用手写 | 朴素手写 |
| --- | --- | --- | --- |
| A 挂载 1,000 行 | 6.86 ms | 6.78 ms | 7.08 ms |
| A 挂载 5,000 行 | 40.7 ms | 41.0 ms | 42.8 ms |
| A 每行常驻 / 副作用 | 4,763 B / 0 | 4,695 B / 0 | 5,522 B / 2 |
| B 挂载 5,000 行 | 42.6 ms | 构建器 43.6 ms / `css!` 41.6 ms | — |
| B 5,000 行里切换选中行 | 459 µs | 构建器 412 µs / `css!` 405 µs | — |
| B 每行常驻 | 5,089 B | 5,025 B | — |
| C 挂载 2,000 行 | 15.1 ms | 15.1 ms | — |
| C 插入或删除 1 行 | 120 µs | 120 µs | — |
| C 改 1 行的字段 | 0.60 µs | 0.60 µs | — |
| C 每行常驻 / 副作用 | 5,926 B / 1 | 5,950 B / 1 | — |

结论：

- 挂载和更新耗时：`.vue` 和惯用手写的差距在 3% 以内，也就是在噪声范围内。
- 常量折叠：朴素写法把不会变的值也写成绑定，每行多 2 个副作用、约 830 B 常驻，挂载慢约 5%。`.vue` 通过折叠自动得到惯用写法的结果。
- `constant(..)` 仍占一个信号槽，所以 `.vue` 比惯用手写每行多约 68 B。
- 条件类：`.vue` 在切换时比手写慢约 11%。每行重跑时要算类掩码、锁住 `StyleSite` 查表，手写的 `FieldWrite` 直接比较一个字段。
- 热重载：debug 构建默认 `hot(true)`，每个纯静态文字节点会变成一个读取替换表的副作用。在 release 里单独量这份生成代码：B、C 每行多 1 个副作用、约 315–330 B 常驻，C 挂载慢约 3%。
- 编译：build 脚本编译 9 个 `.vue` 文件（`views/` 编一次，`bench/` 编两次）约 0.25 s。

这次测量发现并修复了一个问题：带条件类的元素，绑定里保留着自己那份基础布局，没有和其他实例共用，因此每行多占约 2.2 KB（一份 `LayoutStyle`）；每次重跑时查缓存也找不到同一个指针，只能逐字段比较整份布局。修复后，`StyleSite::shared_base` 让同一个位置的所有实例共用一份基础布局。

| 项目 | 修复前 | 修复后 |
| --- | --- | --- |
| B 每行常驻 | 7,241 B | 5,089 B |
| 5,000 行切换选中行 | 817 µs | 459 µs |
| 1,000 行切换选中行 | 134 µs | 83 µs |

`tests/view_memory.rs` 断言编译版每行常驻不超过手写版 512 B；撤掉修复后这个测试会失败。

## 追踪与检查

- `cx.view_bindings(node)` 返回元素的声明位置，以及每个绑定字段和它的声明位置。一直可用。
- 开 `reactive-trace` 后，`cx.why_updated(node)` 返回这个节点最近一次被改的原因：哪次 `set` / `update` 在哪一行写了哪个信号（信号在哪一行创建）。记录只包含 id 和 `&'static Location`，写进每个线程预先分配好的环形缓冲；记录时不分配、不格式化、不做 I/O。
- `cx.inspect(node)` 返回节点在视图层里的样子：它是控件表里的哪个控件、每个可绑定字段的当前值（`Debug` 文本），以及驱动该字段的绑定声明在哪一行。`cx.set_field(node, 字段, 文本)` 按绑定的方式写一个字段（字符串原样、数字和 `true` / `false` 解析、空文本清空可选字段）；被绑定的字段下次绑定运行时会回到绑定的值。读写函数都由控件表生成，新加的控件自动支持。
- devtools：`nana-ui-devtools` 的运行时会话支持 `{"cmd":"inspect", …目标}` 和 `{"cmd":"set_field", …目标, "field":"label", "value":"另存"}`，回复里的 `inspect` 带控件、字段、绑定位置；再开 `reactive-trace`，还带 `causes`：最近一次改动它的信号写在哪一行、信号在哪一行创建。
- 诊断：`runtime.reactive.*` 这组 metric（flush 次数、信号写入、执行的副作用数、改动的节点数、commit 次数、flush 耗时），以及两个 fault：
  - `runtime.reactive.did_not_settle`：副作用互相触发超过 64 轮时报出，剩下的队列会被丢弃；
  - `runtime.reactive.disposed_access`：作用域回收之后又读写了其中的信号时报出，同时 panic。

## 已知限制

- 字段真的改变时，仍然会复制整个组件、完整投影一遍，再由 world 按字段比对标脏。只投影改动字段需要每个控件把投影按字段拆开，目前没做。
- `.bind(|c| …)` 看不出改了哪个字段，所以每次都按"有改动"处理，走复制路径。
- 闭包绑定每个各自装箱一次；只有 `view!` 能看到的整段模板，才有机会把同一节点的闭包合成一个。
- 按名字认识的内置控件只有 `nana-ui-view-schema` 控件表里的这些：`Text`、`Button`、`Checkbox`、`Switch`、`Slider`、`TextInput`、`TextArea`、`NumberInput`、`Select`、`ListItem`、`Progress`、`Spinner`、`Divider`、`Thumbnail`、`Avatar`、`Texture`（`GpuTextureView`）、`IconButton`、`Chip`、`StatusBadge`、`EmptyState`（`#action` slot），外加 `Column`、`Row`、`Widget`。其他控件用 `widget(C)` 加 `.bind` / `.on`。

**无障碍检查**：编译模板时，读屏器无法命名的控件会得到一条警告，不会编译失败。规则两条：`Button`、`Checkbox`、`Switch`、`ListItem` 没有文字（空的子节点或空的 `label`）；`TextInput`、`TextArea`、`NumberInput`、`Slider`、`Progress` 没写 `label`（它们的无障碍名字只来自 `label`，占位文字不算）。`.vue` 的警告经 `cargo:warning` 带行列打印；`view!` 在稳定版上没有警告接口，警告以"使用了已弃用常量"的形式出现在宏调用处，说明写在弃用提示里。

控件表是唯一来源：每条写明模板标签、元素函数及其参数、可绑定字段和类型、`v-model` 对应的字段和事件、事件方法。运行时由它生成 setter、`model` 和事件方法；`view!` 和 `.vue` 编译器由它得知哪些标签是内置的、接受哪些属性。写错的属性名、不存在的事件、对没有 `v-model` 的控件写 `v-model`，都在编译模板时报出行列。数值字段接受 `max="100"` 这样的字面量，生成带类型后缀的常量。没有数据的事件（`@activate`）接受 `|| …`；带数据的事件（`@change`、`@input`、`@submit`）接受 `|e| …`，`e` 的类型自动推断，不用标注。模板里的语句式写法两种都可以，编译器会生成对应的闭包。

## 不做的事

- 不做整树 render，也不做虚拟 DOM diff。
- 不把 JS 的响应式桥接到 Rust 信号上。
- `view!` 只是展开为函数调用的语法糖，不引入新的运行时语义。
