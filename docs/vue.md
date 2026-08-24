# Vue

这是兼容路径，不是新产品的默认写法。第一路径是 Rust 控件，见 [开始](start.md)。

Vue 用来把已经按网页习惯写好的界面落到**同一棵**原生树上。写法和网页接近，跑起来不是网页：没有 WebView，也不能把普通 `@vue/runtime-dom` 网站产物丢进来当桌面应用。

## 应用怎么接

JavaScript 入口是 `@nanaui/nanavue-runtime` 的 `createNanaApp()`。你自己的 Vite 工程把 SFC、TypeScript 和 CSS 打成 Nana 能加载的脚本（通常是 IIFE）。NanaUI 不扫描 `dist`，也不提供另一套打包器。

```js
import { createNanaApp } from "@nanaui/nanavue-runtime";
import { NanaButton } from "@nanaui/nanavue-components";
import "@nanaui/nanavue-components/controls.css";

const nana = createNanaApp();
nana.createApp({
  // 根组件
}).mount();
```

Rust 宿主用 `nana_ui_vue::prelude`：`VueRuntimeProgram::run`（或 `mount_vue_as_nana`）把这份脚本和 V8 引擎交给**同一个** `run_runtime`。后打开的窗口共用这一套 JavaScript 和同一份 GPU，不必另起引擎。

窗口化对照 `examples/vue-hosted-acceptance`。`examples/vue-counter` 是引擎探针（含无头点击），不是应用模板。

## 两种写法，同一棵树

**Nana 控件。** `NanaButton`、`NanaInput`、`NanaDialog` 直接表达语义，不必靠 class 去猜。兼容路径里优先这样写，才能和 Rust 第一路径用同一套控件。

**普通标签和 CSS。** `div`、flex、间距、字号这一类网页习惯可用，但只覆盖 [布局](layout.md) 列出的子集。适合结构骨架，不适合冒充完整浏览器。

两种写法可以混在同一棵界面里。对话框、抽屉、菜单请用对应的 Nana 控件，不要用 `position: fixed` 自己搭网页浮层。

## 它提供的 Web 面，以及明确没有的

为了让熟悉的写法落到桌面窗口，而不是复刻浏览器：

有：`window` / `document` 的一个子集、事件、定时器、`requestAnimationFrame`、本地存储、桌面剪贴板、缓冲式 `fetch`（读完整响应再交给你）。

没有：完整 DOM / CSSOM、流式请求体、cookie、浏览器 CORS、WebSocket、Service Worker、Tauri invoke / 插件 / 窗口协议。未实现的 `fetch` 选项会报错，不会假装成功。

## 网络与宿主命令

网络默认全关。应用必须列出允许的源，格式 `scheme://host[:port]`。localhost 不会自动放行。跨源跳转时，即使目标在白名单里，授权类请求头仍会被拿掉。默认超时 30 秒，请求和响应各 16 MiB，最多 5 次重定向。

NanaUI 不内置登录、设置存储或任何产品业务。你在宿主里注册自己的命令（`HostApiRegistry`），再交给 Vue 调用。框架自带的接口名和你注册的名字不能冲突，冲突时启动失败。

## 扩展控件

要让一种新控件进入布局、点击和绘制：在 Rust 里 `register_component`，并提供 Vue 的 `nana-*` 标签。只暴露 JS 命令时走 `NativeComponentRegistry`。两张表不是同一条 ABI，只登记其中一张，另一条路径不会生效。见 [控件](components.md)。
