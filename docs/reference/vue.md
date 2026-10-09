# Vue

Vue + JS 是 NanaUI 的一等 L1/L2 消费方。它和 Rust L3 共用 Runtime、UiScene 和组件注册合同。

你可以按团队的开发习惯选入口。Rust 入口见 [快速开始](../guide/quick-start.md)。从 Vue 迁过来的对照表在 [从 Vue 过来](../guide/from-vue.md)。

Vue 用来把已经按网页习惯写好的界面，落到**同一棵**原生树上。写法和网页接近。跑起来不是网页。没有 WebView。也不能把普通 `@vue/runtime-dom` 网站产物丢进来当桌面应用。多窗口在 [窗口](vue-windows.md)。标签、网络和扩展控件在 [宿主边界](vue-host.md)。

## 应用怎么接

JavaScript 入口是 `@nanaui/nanavue-runtime` 的 `createApp()`。

你自己的 Vite 工程把 SFC、TypeScript 和 CSS 打成 Nana 能加载的脚本。通常是 IIFE。NanaUI 不扫描 `dist`。它也不提供另一套打包器。

开发期不想每改一次就重启，见 [开发期热重载](hot-reload.md)。

```js
import { createApp } from "@nanaui/nanavue-runtime";
import { NanaButton } from "@nanaui/nanavue-components";
import "@nanaui/nanavue-components/controls.css";

createApp({
  // 根组件
}).mount();
```

Rust 宿主用 `nana_ui_vue::prelude`。`VueRuntimeProgram::run`（或 `mount_vue_as_nana`）把这份脚本和 V8 引擎交给**同一个** `run_runtime`。

`VueRuntimeProgram` 需要 feature `hosted`。它隐含 `scene-view`，把 UiScene 交给 `SceneWgpuPainter`。

后打开的窗口默认共用这一套 JavaScript。你也可以要求独立的 JavaScript 上下文。见下文 [多窗口与 JavaScript 隔离](#多窗口与-javascript-隔离)。两种都共用同一个引擎，也共用同一份 GPU。

## 启动

Early Splash 在 Rust 侧配置：`nana_ui::with_startup(StartupOptions { splash: Some(spec) }, || VueRuntimeProgram::run(...))`。

bundle 在 `UiReady` 求值。那时 Logo 已经在屏幕上。

`Nana.startup` 是宿主启动记录的投影。`state` 可以随时读，形状是 `{ phase, splash, ticket, timeline }`。`deferTakeover()` 只在首次求值时有效。`takeOver()` 和 `cancelTakeover()` 发请求。`onChange(listener)` 收宿主的 `startup` 事件。

Vue 宿主的默认主题是 `Light`。splash 的 `SplashBackground::System` 跟随系统明暗。两者不一致时，给 splash 传 `Color(..)`。合同见 [两阶段启动](startup.md)。


## 多窗口与 JavaScript 隔离

共享和隔离、`tag`、`shadow`、句柄上能做什么，写在 [窗口](vue-windows.md)。

## 两种写法，同一棵树

Vue 标签怎样落到控件、普通标签覆盖哪一段 CSS、输入只有一条路由，写在 [宿主边界](vue-host.md#两种写法同一棵树)。

## 长列表上的 hover

一个 render function 拥有整列时，hover 改参与渲染的状态会让整列重算。数字和写法在 [宿主边界](vue-host.md#长列表上的-hover)。

## Markdown

`NanaMarkdown` 的取值和 `rich-text` feature 在 [宿主边界](vue-host.md#markdown)。

## 本地化文字

`<T id="files" :count="n" />`、`Nana.i18n` 的目录和 locale，以及子树的 `locale` 属性，在 [宿主边界](vue-host.md#本地化文字)。窗口自己的 locale 在 [窗口](vue-windows.md)。

## 它提供的 Web 面，以及明确没有的

`window` / `document` 子集、`fetch`、存储、音频，以及明确没有的浏览器能力，在 [宿主边界](vue-host.md#它提供的-web-面以及明确没有的)。

## 网络与宿主命令

白名单、`fetch` 流、`WebSocket` 和 `Nana.storage` 在 [宿主边界](vue-host.md#网络与宿主命令)。

## 扩展控件

Rust `register_component` 和 JS 命令表的差别在 [宿主边界](vue-host.md#扩展控件)。
