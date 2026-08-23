# Vue

这是兼容路径，不是第一路径。

产品界面的第一路径是原生控件和布局，见 [开始](start.md)。Vue 用来把已经按网页习惯写好的界面落到同一棵原生树上：单文件组件、`<script setup>`、Composition API、模板和样式的一个子集可用。写法和网页接近，跑起来不是网页，也不是产品该怎么写的起点。

入口是 `@nanaui/nanavue-runtime` 的 `createNanaApp()`。你自己的 Vite 工程把 SFC、TypeScript 和 CSS 打成 Nana 能加载的脚本；NanaUI 不扫描 `dist`，也不提供另一套打包器。

```js
import { createNanaApp } from "@nanaui/nanavue-runtime";
import { NanaButton } from "@nanaui/nanavue-components";
import "@nanaui/nanavue-components/controls.css";

const nana = createNanaApp();
nana.createApp({
  // 你的根组件
}).mount();
```

更完整的窗口示例见 `examples/vue-counter` 和 `examples/vue-hosted-acceptance`。

## 两种写法，同一棵树

**用 Nana 控件。** `NanaButton`、`NanaInput`、`NanaDialog` 这类组件直接表达「这是按钮 / 输入 / 对话框」，不必靠 class 去猜。兼容路径里优先这样写，才能和原生第一路径用同一套控件。

**用普通标签和 CSS。** `div`、flex、间距、字号这一类网页习惯仍然可用，但只覆盖 [布局](layout.md) 里列出的子集。它适合结构骨架和轻量样式，不适合拿来冒充完整浏览器。

两种写法可以混在同一棵界面里。对话框、抽屉、菜单请用对应的 Nana 控件，不要用 `position: fixed` 自己搭一层网页浮层。

## 它不是浏览器

没有 WebView，也没有 Tauri 那套窗口和插件协议。不能把普通 `@vue/runtime-dom` 的网站产物丢进来就当桌面应用跑。

有的网页 API 可以用，目的是让熟悉的写法能落到桌面窗口，而不是复刻一个浏览器：

- `window` / `document`、事件、定时器、`requestAnimationFrame`
- 本地存储、剪贴板（桌面）
- 缓冲式 `fetch`：读完整个响应，再交给你的代码

没有的东西同样明确：完整 DOM 和 CSSOM、流式请求体、cookie、浏览器 CORS、WebSocket、Service Worker。未实现的 `fetch` 选项会直接报错，不会假装成功。

## 网络默认全关

应用必须自己规定允许访问哪些源，格式是 `scheme://host[:port]`。默认一个都不放行，也不会因为是 localhost 就自动打开。

跨源跳转时，即使目标也在白名单里，授权类请求头仍会被拿掉。超时、体积和并发都有上限：默认 30 秒、请求和响应各 16 MiB、最多 5 次重定向。

## 业务命令由你提供

NanaUI 不内置登录、设置存储、GitHub 或任何产品业务。你在宿主里注册自己的命令和鉴权，再交给 Vue 调用。框架自带的接口和你注册的名字不能冲突，冲突时启动会失败，避免业务悄悄覆盖渲染或网络。

## 多窗口

后打开的窗口共用同一套 Vue / JavaScript 和图形能力。新窗口可以继续画 Canvas、WebGPU 和实时画面，不必另起一套引擎。窗口怎么创建、怎么关，见 [窗口](window.md)。

## 内部如何工作

```text
Vue 3 SFC / TypeScript / JavaScript
        │ createNanaApp()
        ▼
可复现 IIFE + CSS 子集
        │ V8
        ▼
Custom Renderer hostOps
        │
        ├─ 普通标签 / class / style  ─┐
        └─ nana-* 组件 props         ├─> MessageBridge / Style Model
                                     │
                                     ▼
                            UiWorld / UiScene → SceneWgpuPainter
```

`NanaTreeDocument`、`MessageBridge`、`LayoutBoxStore` 是兼容投影，不是保留权威。host op 先进入 `PendingHostOps`，`flush_host_frame` 才提交到 Runtime。几何权威在 Runtime；`LayoutBoxStore` 只为这个窗口的 JS 查询记下绘制阶段的盒子。

宿主侧：

```rust
let mut host = mount_vue_as_nana(MountOptions { .. });
host.inject_stylesheet(app_css);
host.initialize_with_web_api_and_host_api(&mut engine, app_iife, &application_api)?;
host.bind_event_bridge(&mut engine)?;
```

每个唤醒：结算 fetch Promise → 排空定时器和微任务 → 更新布局 → 宿主拿新的语义快照去画。`next_wakeup()` 在空闲时不轮询。

模块大致是：`nana-js-v8` 跑脚本，`nanavue-runtime` 做 Vue hostOps，`nana-ui-web-api` 提供 window / fetch 子集，`nana-ui-vue` 把树写进 Runtime，`nana-ui` 画窗口。产物一般是 UTF-8 源码；V8 snapshot 只适合顶层不调用宿主的脚本，不能拿来当通用「藏源码」。

诊断走 `nana_ui_devtools::DevtoolsSession`，只给开发工具，不画进产品界面。Inspector 在现有 isolate 里开 CDP，不监听端口，也不另起一个 JS 引擎。
