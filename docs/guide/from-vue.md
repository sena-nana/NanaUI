# 从 Vue 过来

指南的章节名沿用 Vue。运行时不是 Vue：没有虚拟 DOM，没有 WebView，树在挂载时建一次。下面这张表是查阅入口。信号、条件和列表的写法在对应章节。完整对照和测量在 [声明式视图](../reference/reactive-view.md#和-vue-的对应)。

## 对照

| Vue | NanaUI | 要改的习惯 |
| --- | --- | --- |
| `ref(0)` | `signal(0)` | `ref` 是 Rust 关键字。信号是 `Copy` 的 id，值在创建它的线程上。拿到别的线程上用会 panic |
| `reactive({ … })` | `store` 加 `#[derive(Store)]` | 按字段追踪，不是深代理。见 [Store](scaling/store.md) |
| `computed(() => …)` | `computed(move \|\| …)` | 构造视图时直接 `.get()` 写进节点的是当时的常量 |
| `watch` / `watchEffect` | `watch_effect` | 返回 `Effect`，不带旧值和新值。见 [侦听](essentials/watch.md) |
| `{{ x }}`、`:label="x"` | `"{x}"`、`.label(x)` | 单独出现的信号会建立依赖。表达式里的 `.get()` / `.with()` 省不掉 |
| `@click` | `@activate` / `.on_activate` | 按钮不是 DOM click。别的事件用 `.on` |
| `v-if` / `v-else` | `when` / `.otherwise` | 条件变了才拆掉旧分支。见 [条件](essentials/conditional.md) |
| `v-show` | `.visible` | 节点留着，只切隐藏 |
| `v-for` + `:key` | `each` | 按 key 保留行。见 [列表](essentials/list.md) |
| `v-model` | `.model` / `v-model` | 写回的是信号 |
| `provide` / `inject` | `provide` / `use_context` | 只能在构建视图时读。见 [依赖提供](components/provide.md) |
| 模板 ref | 句柄 | `node_ref`、`entity_ref`。见 [句柄](essentials/refs.md) |
| `onMounted` / `onUnmounted` | `on_mount` / `on_cleanup` | 见 [挂载与卸载](essentials/lifecycle.md) |
| `nextTick` | `flush_reactive` | 写入先入队。输入事件末尾和每帧开头刷新 |
| props / emits | 函数参数 / 回调 | 见 [属性](components/props.md) 和 [事件](components/events.md) |
| `<KeepAlive>`、`<Teleport>`、`<Transition>`、`<Suspense>` | 同名能力 | 见指南里的内置 |
| Vue Router、Pinia | 应用自己的结构和 `store` | 框架不持有路由、区域内容和持久化 |
| SFC | `.vue` 方言，或 `@nanaui/nanavue-runtime` | 见 [.vue 方言](scaling/sfc.md) 和 [Vue](../reference/vue.md) |

`each` 和 `when` 各自带一个容器。Vue 的 `v-for` / `v-if` 直接生成兄弟节点。组件函数不是惰性的，只在挂载、行、分支这三个作用域边界上划分归属。

## 这里没有的

- 虚拟 DOM 和按树 diff。绑定只改它写的那个字段。
- WebView。不能把 `@vue/runtime-dom` 的产物丢进来当桌面应用。
- 浏览器的 CORS、Cookie、Service Worker、IndexedDB。`fetch` 有白名单，见 [Fetch](scaling/fetch.md)。
- `v-html` 在 Vue 宿主里会解析成子节点，不是一段 HTML 字符串插进 DOM。见 [宿主边界](../reference/vue-host.md)。

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
  <a class="next-step" href="/reference/vue">
    <p class="next-step-link">Vue 宿主</p>
    <p class="next-step-caption">把已有的 Vue 界面接到同一棵树上。</p>
  </a>
</div>
