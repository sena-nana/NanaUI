# .vue 方言

日常写视图用 `view!` 或函数写法。`.vue` 文件适合从 Vue 迁过来、想逐文件对照，或者希望模板、脚本、样式分开放。它不进侧边栏顶部的开关。编译结果和 `view!` 走同一个代码生成器、同一套调用，运行时没有多出来的能力。多出来的是编译器能看见整段脚本，所以能做依赖分析和常量折叠。脚本和模板表达式没有 rust-analyzer 支持。

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

构建脚本把 `views/` 编成普通函数。模板报错带文件、行、列。

```rust
// build.rs
fn main() {
    if let Err(error) = nana_ui_sfc::Compiler::new("::nana_ui::runtime").build("views") {
        panic!("{error}");
    }
}

// src/lib.rs
pub mod views {
    include!(concat!(env!("OUT_DIR"), "/nana_views.rs"));
}
```

`TodoList.vue` 变成 `pub fn todo_list(…) -> impl IntoView`。子组件的参数用 `defineProps!` 声明：

```vue
<script setup lang="rust">
defineProps!(todo: Todo, on_remove: impl Fn() + Send + 'static);
</script>
```

同一批编译的组件按 prop 名匹配参数，不按书写顺序。`@remove` 对应 `on_remove`。子节点对应最后一个名为 `children` 的 prop。缺参数、多参数都是编译错误。

## 插槽、ref 和样式

子组件声明 `header: impl IntoView` 这样的视图参数，模板里用 `<slot name="header"/>` 放进去。`<slot/>` 放 `children`。父组件写 `<template #header>…</template>`。`#default` 等于其余子节点。插槽内容按值传入，只能放一次，也没有后备内容。

`ref="name"` 把节点 id 写进脚本里的 `let name = node_ref();`。`on_mount(move |cx| …)` 在视图进树之后执行，用来聚焦或读布局。组件上的 `key` 落在第一个根节点上。不在这一批里的标签，按 `view!` 的规则调用同名的 Rust 函数。

`<style>` 写不写 `scoped` 都只作用于本组件，在构建时编译。模板仍遵守 Rust 的所有权，同一个值既要传给组件又要被事件闭包使用时，写 `todo.clone()`。

生成的代码排版后写进 `$OUT_DIR/nana_views.rs`。每个组件的信号和每条绑定的分类写进 `$OUT_DIR/nana_views.deps.md`。

## 编译器替你折掉的

扫描把 `get` / `with` 当成读，`set` / `update` 当成写。看不透的调用算动态。看不见变量遮蔽，不确定的情况不优化。

信号从未被写入、也没有传出时，折成 `constant(…)`：句柄还在，读取不建立依赖，作为属性时只写一次。只读常量的绑定在建节点时写一次，不创建副作用。computed 成环是编译错误。有写入但没人读，或 `watch_effect` 写了自己读的信号，经 `cargo:warning` 报出。

看得见的静态依赖包成 `__checked`。debug 构建里每次执行都核对实际读到的信号没有超出声明，不一致报 `runtime.reactive.static_deps_mismatch`。release 里原样返回闭包，正确性不押在这次扫描上。一次「值没变」的节点重跑大约 70 ns。

只改 `.vue` 里的静态文字时，构建脚本写 `Compiler::new(..).hot(debug)`，运行中的窗口可以换字。见 [热重载](hot-reload.md)。

## 和手写差在哪里

`.vue` 和惯用手写（静态值直接传、单个信号直接绑）的挂载和更新差距在 3% 以内。朴素写法把不会变的值也写成绑定，每行多 2 个副作用、约 830 B 常驻。`.vue` 靠折叠得到惯用写法的结果。`constant(…)` 仍占一个信号槽，所以比惯用手写每行多约 68 B。

条件类在切换时比手写的 `FieldWrite` 慢约 11%：重跑时要算类掩码并查 `StyleSite`。debug 里 `hot(true)` 让每个纯静态文字节点多一次读表。在 release 里单独量这份生成代码：每行多 1 个副作用、约 315–330 B 常驻，挂载慢约 3%。构建脚本编译 9 个 `.vue` 文件大约 0.25 s。

只从 Rust 类型、`view!` 或 `.vue` 建树时，可以用 `BuiltinComponents::Typed` 丢掉按标签构造的绑定器。体积和 Vue 宿主为什么仍用完整模式，见 [扩展控件](../components/registry.md)。

合同和测量条件在 [声明式视图](../../reference/reactive-view.md)。

## 接下来

<div class="next-steps">
  <a class="next-step" href="/guide/scaling/hot-reload">
    <p class="next-step-link">热重载</p>
    <p class="next-step-caption">改完 CSS、打包产物或 .vue 文字之后怎么看到结果。</p>
  </a>
  <a class="next-step" href="/guide/scaling/store">
    <p class="next-step-link">Store</p>
    <p class="next-step-caption">一份嵌套状态按路径追踪。</p>
  </a>
  <a class="next-step" href="/guide/components/registry">
    <p class="next-step-link">扩展控件</p>
    <p class="next-step-caption">精简模式省下的体积从哪来。</p>
  </a>
</div>
