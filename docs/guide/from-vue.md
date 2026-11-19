# 从 Vue 过来

指南的章节名沿用 Vue。运行时不是 Vue：没有虚拟 DOM，没有 WebView，树在挂载时建一次。下面这张表是查阅入口。信号、条件和列表的写法在对应章节。完整对照和测量在 [声明式视图](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/reactive-view.md#和-vue-的对应)。

## 对照

| Vue | NanaUI | 要改的习惯 |
| --- | --- | --- |
| `ref(0)` | `signal(0)` | `ref` 是 Rust 关键字。信号是 `Copy` 的 id，值在创建它的线程上。拿到别的线程上用会 panic |
| `reactive({ … })` | `store` 加 `#[derive(Store)]` | 按字段追踪，不是深代理。见 [Store](https://github.com/sena-nana/NanaUI/blob/main/docs/guide/scaling/store.md) |
| `computed(() => …)` | `computed(move \|\| …)` | 构造视图时直接 `.get()` 写进节点的是当时的常量 |
| `watch` / `watchEffect` | `watch_effect` | 返回 `Effect`，不带旧值和新值。见 [侦听](https://github.com/sena-nana/NanaUI/blob/main/docs/guide/essentials/watch.md) |
| `{{ x }}`、`:label="x"` | `"{x}"`、`.label(x)` | 单独出现的信号会建立依赖。表达式里的 `.get()` / `.with()` 省不掉 |
| `@click` | `@activate` / `.on_activate` | 按钮不是 DOM click。别的事件用 `.on` |
| `v-if` / `v-else` | `when` / `.otherwise` | 条件变了才拆掉旧分支。见 [条件](https://github.com/sena-nana/NanaUI/blob/main/docs/guide/essentials/conditional.md) |
| `v-show` | `.visible` | 节点留着，只切隐藏 |
| `v-for` + `:key` | `each` | 按 key 保留行。见 [列表](https://github.com/sena-nana/NanaUI/blob/main/docs/guide/essentials/list.md) |
| `v-model` | `.model` / `v-model` | 写回的是信号 |
| `provide` / `inject` | `provide` / `use_context` | 只能在构建视图时读。见 [依赖提供](https://github.com/sena-nana/NanaUI/blob/main/docs/guide/components/provide.md) |
| 模板 ref | 句柄 | `node_ref`、`entity_ref`。见 [句柄](https://github.com/sena-nana/NanaUI/blob/main/docs/guide/essentials/refs.md) |
| `onMounted` / `onUnmounted` | `on_mount` / `on_cleanup` | 见 [挂载与卸载](https://github.com/sena-nana/NanaUI/blob/main/docs/guide/essentials/lifecycle.md) |
| `nextTick` | `flush_reactive` | 写入先入队。输入事件末尾和每帧开头刷新 |
| props / emits | 函数参数 / 回调 | 见 [属性](https://github.com/sena-nana/NanaUI/blob/main/docs/guide/components/props.md) 和 [事件](https://github.com/sena-nana/NanaUI/blob/main/docs/guide/components/events.md) |
| `<KeepAlive>`、`<Teleport>`、`<Transition>`、`<Suspense>` | 同名能力 | 见指南里的内置 |
| Vue Router、Pinia | 应用自己的结构和 `store` | 框架不持有路由、区域内容和持久化 |
| SFC | `.vue` 方言，脚本写 Rust | 见 [.vue 方言](https://github.com/sena-nana/NanaUI/blob/main/docs/guide/scaling/sfc.md) |

`each` 和 `when` 在父容器内默认作为无盒片段直接生成兄弟节点，与 Vue 的 `v-for` / `v-if` 相同；只在显式使用 `.gap`、`.horizontal`、`.grow`、`.class`、`.key` 等容器选项或处于单根插槽/独立根挂载边界时才建一个包装 `Stack`。组件函数不是惰性的，只在挂载、行、分支这三个作用域边界上划分归属。

## 这里没有的

- 虚拟 DOM 和按树 diff。绑定只改它写的那个字段。
- WebView 和 JS 运行时。不能把 `@vue/runtime-dom` 的产物丢进来当桌面应用。
- 浏览器的 CORS、Cookie、Service Worker、IndexedDB。远程资源有白名单，见 [Fetch](https://github.com/sena-nana/NanaUI/blob/main/docs/guide/scaling/fetch.md)。
- `v-html`。

## 接下来

<div class="next-steps">
  <a class="next-step" href="/guide/quick-start">
    <p class="next-step-link">快速开始</p>
    <p class="next-step-caption">写出第一段界面。</p>
  </a>
  <a class="next-step" href="/guide/essentials/reactivity">
    <p class="next-step-link">响应式</p>
    <p class="next-step-caption">信号怎么读、怎么写。</p>
  </a>
  <a class="next-step" href="/guide/scaling/sfc">
    <p class="next-step-link">.vue 方言</p>
    <p class="next-step-caption">模板、脚本、样式分文件写，脚本是 Rust。</p>
  </a>
</div>
