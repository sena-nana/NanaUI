# 声明式视图（原型）

`reactive-view` feature 下的 `nana_ui::runtime::view`：视图写成一个表达式，动态部分是信号或闭包，树只建一次，之后每个绑定只更新它写的那个节点字段。不整树 render，不做 diff。它和 `build` / `mount` 写同一棵 `UiWorld`、同一张 assembly key 表，最后落到的还是 `create` / `insert` / `project` / `commit`。

**状态：不稳定。** 稳定前不进入公开合同。开关：`nana-ui` 的 `reactive-view`；模板宏 `view!` 另加 `view-macro`；追踪另加 `reactive-trace`。`.vue` 方言由 `nana-ui-sfc` 在 `build.rs` 里编译。示例：`crates/nana-ui/examples/reactive-counter.rs`（用 `view!` 写），`examples/reactive-sfc`（用 `.vue` 文件写）。

## 写法

```rust
use nana_ui::runtime::view::*;
use nana_ui::runtime::text;

fn counter() -> impl IntoView {
    let count = signal(0u64);                                   // 组件局部状态
    row(8.0, (
        text!("计数 {count}"),                                   // 插值，读到的信号自动成为依赖
        button("增加").on_activate(move || count.update(|c| *c += 1)),
    ))
}

fn todos(list: Signal<Vec<Todo>>, draft: Signal<String>) -> impl IntoView {
    column(8.0, (
        text_input().placeholder("新任务").model(draft),          // v-model
        button("添加").disabled(move || draft.with(|d| d.is_empty())),
        each(list, |t| t.id, |t| text(t.title)),                // v-for + :key
        when(move || list.with(Vec::is_empty), || text("还没有任务"))
            .otherwise(|| text("有任务")),                        // v-if / v-else
    ))
}

let view = cx.mount_view(parent, counter)?;                    // 或 mount_view_root(document, …)
```

- **属性**接受常量、`Signal<T>` / `Computed<T>`、`Fn() -> T` 闭包三种。常量在建节点时写进去，之后没有任何成本。信号直接绑定，只存一条"信号 id + 字段写入函数"的记录，没有闭包。闭包装箱一次。
- **控件属性**由 `view/controls.rs` 里的 `props!` 宏按字段生成 setter（`button(..).disabled(..)`、`slider(..).value(..)`），每个 setter 对应一个 `FieldWrite`。任意控件都能用 `widget(component).bind(|c| …)` 和 `.on::<E>(|e| …)`，自定义字段写 `FieldWrite` 后用 `.prop::<T, W>(..)`。
- **key** 只在 `each` 的元素和需要按路径查找（`resolve_assembly_path`）的节点上写。静态结构只建一次，没写 key 的节点按位置命名（`#v0`、`#v1`…）。
- **`mount_view` 接收闭包。** 视图表达式在挂载作用域里求值，组件函数里创建的信号归这个挂载，`unmount`、销毁根节点、或 `AppContext` 被丢弃时一起回收。`each` 的每一行、`when` 的每个分支各有一个子作用域，行被删掉、分支被切走时只回收它自己的信号和副作用。
- 在任何作用域之外创建的信号（例如应用启动时的全局状态、事件处理器里新建的信号）不归任何挂载，会一直存在到线程结束。列表数据里每行需要的信号，应该在行视图里创建，或者由持有它的挂载创建。
- `each` 里 key 重复时，只保留第一个，后面的重复项跳过。
- 在视图构造阶段直接 `count.get()` 读出来的值是常量，不会跟着变。要跟着变，就传信号本身或者传闭包。这和 Vue `setup` 里读 `.value` 的规则一样。

## `view!` 模板

`view-macro` feature 提供 `view!`（`nana_ui::runtime::view!`）。它是 Vue 模板写法的对应，展开结果就是上面那些函数调用，不增加任何运行时概念。宏在独立的 proc-macro crate `nana-ui-view-macros` 里，不开 feature 就不参与编译。

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

| 模板 | 展开 |
| --- | --- |
| `<Column gap=8>…</Column>` / `<Row>` | `column(8_f32, (…))` / `row(…)`；数字字面量带上类型后缀，表达式原样传入 |
| `<Text>"计数 {count}"</Text>` | `text!("计数 {count}")`；没有 `{…}` 的字符串是 `text("…")` |
| `<Button>"加一"</Button>`、`<Checkbox>` | `button("加一")`、`checkbox(…)` |
| `<Slider min=0 max=1 step=0.05/>`、`<TextInput/>` | `slider(0_f64, 1_f64, 0.05_f64)`、`text_input()` |
| `<Widget of={component}>…</Widget>` | `widget(component).children((…))` |
| `<TodoRow todo={t} list={list}/>`（其他标签） | `todo_row(t, list)`：标签名转成 snake_case，属性值按书写顺序作为参数，子节点作为最后一个参数 |
| `name="x"`、`name=3`、`name={x}`、`name={a.b}` | 原样传入：常量，或者信号本身 |
| `name={其他表达式}` | `move \|\| 表达式`：读到的信号变了就重算 |
| `name={\|\| …}` | 闭包原样传入 |
| `@activate={表达式}` | `.on_activate(move \|\| { 表达式; })`；`@activate={add}` 直接传函数值 |
| `on:RangeChanged={\|e: &RangeChanged\| …}` | `.on::<RangeChanged>(…)` |
| `v-if` / `v-else-if` / `v-else`（兄弟节点） | `when(…).otherwise(…)`，`v-else-if` 嵌套在 `otherwise` 里 |
| `v-for={pat in items} key={…}` | `each(items, move \|item\| { let pat = item; key }, move \|pat\| 元素)` |
| `v-show={x}`、`v-model={sig}`、`key="x"` | `.visible(x)`、`.model(sig)`、`.key("x")` |

多于 12 个子节点时，宏会嵌套成多层 tuple。写错的地方会被准确指出，例如：未闭合的标签、`</Row>` 关了 `<Column>`、`v-else` 前面没有 `v-if`、`v-for` 没写 `key`、未知指令、`v-if` 和 `v-for` 写在同一个元素上。`tests/view_macro.rs` 把同一个页面用模板和手写函数调用各写一遍，挂载后的保留树逐节点相同，改完信号 flush 之后也相同。

在宏里能静态区分的只有三类：常量（字面量）、直接值或信号（路径、字段）、闭包（其他表达式）。"静态依赖"那一类需要知道哪个标识符是信号，只有能看到整段脚本的 [`.vue` 方言](#vue-方言)编译器才能做到。

## `.vue` 方言

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
- `<style>` 不支持：CSS 子集属于 Vue 路径。模板里仍然要遵守 Rust 的所有权规则，例如同一个值既要传给组件又要被事件闭包使用时，得写 `todo.clone()`。
- 生成的代码用 prettyplease 排版后写进 `$OUT_DIR/nana_views.rs`，rustc 的报错会指向可读的代码。模板和脚本本身的错误（语法、标签不配对、缺 `key`、缺参数、computed 成环）在构建时报出，带文件、行、列。

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
| `each` | 用 key 对照：保留的行不重建，节点 id 和控件的交互状态都不变；删掉的行回收作用域并销毁节点；新行在一次 detached build 里建好。重排只移动最长递增子序列之外的节点：插入或删除一行不移动任何已有节点，整体反转移动 n−1 个。一行里的字段变化应该用行内信号，这样不会触发列表重算 |
| `when` | 条件变了才动：旧分支回收作用域并销毁，新分支建好后插入。`.visible(sig)` 则保留节点，只切 `layout.hidden`（对应 `v-show`） |
| 回收 | 节点被销毁时（不管从哪条路径），`commit_mutations` 的清理段会回收它的绑定、结构副作用和锚定在它身上的作用域 |
| 上下文 | `provide(value)` / `use_context::<T>()`：值挂在当前作用域上，下层作用域（包括之后才建出来的行和分支）沿父链读取，最近的提供者优先。只能在构建视图时读；事件处理器运行时不在任何作用域里 |
| 出帧 | 宿主每处理完一次更新（输入、程序消息、定时器），都会检查各窗口有没有待应用的绑定；有就请求重绘，所以在 `RuntimeProgram::update` 里改信号也会出帧 |
| 线程 | 每个线程一个信号运行时。句柄是 `Copy` 的 id，可以放进 `Send` 的事件处理器；在别的线程上使用会 panic。`AppContext` 要留在创建视图的那个线程上 |

## 和 Vue 的对应

| Vue | L3 视图 |
| --- | --- |
| `ref(0)` | `signal(0)`（`ref` 是 Rust 关键字） |
| `computed(() => …)` | `computed(move \|\| …)` |
| `watchEffect` | `watch_effect(move \|\| …)` |
| `{{ x }}`、`:label="x"` | `text!("{x}")`、`.label(x)`，或者 `.bind(move \|c\| …)` |
| `@click` | `.on_activate(move \|\| …)`，其他事件用 `.on(move \|e: &E\| …)` |
| `v-if` / `v-else` | `when(cond, \|\| a).otherwise(\|\| b)` |
| `v-show` | `.visible(sig)` |
| `v-for` + `:key` | `each(items, key, row)` |
| `v-model` | `.model(sig)`（文本输入、滑块、复选框） |
| props / emits | 函数参数 / `impl Fn(T)` 回调参数 |
| slot / 具名 slot | `impl IntoView` 参数；`.vue` 里 `<slot name="x"/>` 与 `<template #x>` |
| 模板 `ref` | `node_ref()` + `.node_ref(r)`；`.vue` 里 `ref="r"` |
| `onMounted` | `on_mount(move \|cx\| …)` |
| `provide` / `inject` | `provide(value)` / `use_context::<T>()` |
| `onUnmounted` | `on_cleanup` |
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

除了样式（Vue 用 CSS 子集，L3 用 Style Model）和脚本逻辑（JS 要改成 Rust），模板和响应式部分可以逐行对上。和 Vue 模板仍有三处不同：

- `each` 和 `when` 各自带一个容器 `Stack`，而 Vue 的 `v-for` / `v-if` 直接生成兄弟节点。
- 表达式里的 `.get()` / `.with()` 省不掉，因为 Rust 稳定版不能给信号实现 `Fn`。只有单独出现的信号可以省，例如 `"{count}"`、`disabled={busy}`。
- 组件函数不是惰性的，只在挂载、行、分支这三个作用域边界上才划分归属。

## 成本

测量命令：`cargo run --release -p nana-ui-runtime --features benchmark,reactive-view --bin nana-reactive-benchmark`。每种写法交替跑 15 轮，读最小值；测的是写法层加 commit，不含布局和绘制。2026-09-28 本机，负载 4–5。

| 场景 | 旧写法 | 新写法 |
| --- | --- | --- |
| 挂载 1,000 个文本 | `build` 5.13 ms | 常量 5.27 ms / 每个都绑定信号 5.41 ms |
| 挂载 5,000 个文本 | `build` 71.0 ms | 常量 72.3 ms / 全部绑定 72.9 ms |
| 5,000 个里改 1 个 | `update_component` 0.41 µs | `set` + flush 0.55 µs |
| 5,000 个里改 100 个 | 逐个 `update_component` 46.2 µs | 100 个信号一次 flush 43.9 µs / 1 个信号绑 100 个节点 40.9 µs |
| 2,000 行里插入再删除 1 行 | 整段 `mount` 重写 7.89 ms | `each` 0.160 ms |
| 2,000 行里改 1 行的字段 | 手写 `update_component` 0.40 µs / 整段 `mount` 重写 6.09 ms | 行内信号 0.54 µs |
| 100 个按钮的绑定重跑但值不变 | — | 6.83 µs |

单点更新比手写 `update_component` 多出约 0.15 µs，这是信号簿记的成本；一次改很多节点时只 commit 一次，反而更快。

第二阶段两项优化各自做了 A/B（前后二进制交替各跑 3 次，读最小值）：

- 字段级比较：绑定重跑但值不变时，21.1 µs → 7.2 µs。值确实改变时基本持平，"1 个信号绑 100 个节点"慢约 4%，原因是被改的直接绑定要读两次信号单元格。
- `reconcile_child_order` 改成最长递增子序列：`each` 插入加删除一行，1.85 ms → 0.165 ms。它也是 `mount` 和 workspace 装配共用的重排路径。

内存（`tests/reactive_view_alloc.rs`，按线程计数的分配器）：

- 5,000 个节点里，常量文本平均每个节点约 11.8 KB、25 次分配。直接绑定信号的节点比它多约 945 B、4 次分配，其中包括哈希表和 Vec 扩容摊下来的部分。
- 同一个节点再加一个直接绑定，只多 1 次分配，就是新信号的订阅表。
- 依赖不变时重新执行副作用，0 次分配；绑定重跑但字段值没变的一次 flush，也是 0 次分配。

## 追踪与检查

- `cx.view_bindings(node)` 返回元素的声明位置，以及每个绑定字段和它的声明位置。一直可用。
- 开 `reactive-trace` 后，`cx.why_updated(node)` 返回这个节点最近一次被改的原因：哪次 `set` / `update` 在哪一行写了哪个信号（信号在哪一行创建）。记录只包含 id 和 `&'static Location`，写进每个线程预先分配好的环形缓冲；记录时不分配、不格式化、不做 I/O。
- 诊断：`runtime.reactive.*` 这组 metric（flush 次数、信号写入、执行的副作用数、改动的节点数、commit 次数、flush 耗时），以及两个 fault：
  - `runtime.reactive.did_not_settle`：副作用互相触发超过 64 轮时报出，剩下的队列会被丢弃；
  - `runtime.reactive.disposed_access`：作用域回收之后又读写了其中的信号时报出，同时 panic。

## 已知限制

- 字段真的改变时，仍然会复制整个组件、完整投影一遍，再由 world 按字段比对标脏。只投影改动字段需要每个控件把投影按字段拆开，目前没做。
- `.bind(|c| …)` 看不出改了哪个字段，所以每次都按"有改动"处理，走复制路径。
- 闭包绑定每个各自装箱一次；只有 `view!` 能看到的整段模板，才有机会把同一节点的闭包合成一个。
- `.model` 目前只覆盖 `TextInput`、`RangeField`、`Checkbox`；带类型的 setter 目前只覆盖 `Text`、`Button`、`RangeField`、`TextInput`、`Checkbox`。

## 不做的事

- 不做整树 render，也不做虚拟 DOM diff。
- 不把 JS 的响应式桥接到 Rust 信号上。
- `view!` 只是展开为函数调用的语法糖，不引入新的运行时语义。
