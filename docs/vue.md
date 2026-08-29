# Vue

这是兼容路径，不是新产品的默认写法。第一路径是 Rust 控件，见 [开始](start.md)。

Vue 用来把已经按网页习惯写好的界面落到**同一棵**原生树上。写法和网页接近，跑起来不是网页：没有 WebView，也不能把普通 `@vue/runtime-dom` 网站产物丢进来当桌面应用。

## 应用怎么接

JavaScript 入口是 `@nanaui/nanavue-runtime` 的 `createApp()`。你自己的 Vite 工程把 SFC、TypeScript 和 CSS 打成 Nana 能加载的脚本（通常是 IIFE）。NanaUI 不扫描 `dist`，也不提供另一套打包器。

```js
import { createApp } from "@nanaui/nanavue-runtime";
import { NanaButton } from "@nanaui/nanavue-components";
import "@nanaui/nanavue-components/controls.css";

createApp({
  // 根组件
}).mount();
```

Rust 宿主用 `nana_ui_vue::prelude`：`VueRuntimeProgram::run`（或 `mount_vue_as_nana`）把这份脚本和 V8 引擎交给**同一个** `run_runtime`。后打开的窗口共用这一套 JavaScript 和同一份 GPU，不必另起引擎。

窗口化对照 `examples/vue-hosted-acceptance`。`examples/vue-counter` 是引擎探针（含无头点击），不是应用模板。

## 两种写法，同一棵树

**Nana 控件。** `NanaButton`、`NanaInput`、`NanaDialog` 直接表达语义，不必靠 class 去猜。兼容路径里优先这样写，才能和 Rust 第一路径用同一套控件。

**普通标签和 CSS。** `div`、flex、间距、字号这一类网页习惯可用，但只覆盖 [布局](layout.md) 列出的子集。适合结构骨架，不适合冒充完整浏览器。

和 Runtime 同语义的 Vue 标签会落到对应控件：`button`、`a`、`input`（含 `checkbox` / `radio` / `range` / `number`）、`textarea`、`select` + `option`、`ul`/`ol`/`li`、`table`/`tr`/`td`/`th`、`progress`、`meter`、`hr`、`dialog`、`details`/`summary`。`v-html` 会把片段解析成子节点。`Teleport to="body|html"`、`Transition`、`KeepAlive`、`Suspense` 走同一套 host ops，没有第二棵树。

语义不同就换名：`search-dropdown` 不是 HTML `<search>`；`nana-scroll-view` 不是随便一个 `div`。`<iframe>`、`<video>` 播放、未接槽的 `<canvas>` 2D 不伪造浏览器。

地标标签携带 a11y landmark role：`nav` → navigation、`main` → main、`aside` → complementary、`search` → search、`header` → banner、`footer` → contentinfo（`header` / `footer` 是 `article` / `aside` / `main` / `nav` / `section` 后代时除外）；`section` / `form` 只有带可访问名时才是 region / form——名字可以来自 `aria-label`、`aria-labelledby` 或自身文本内容。class / role hints 把地标标签改成具体控件（如 `<nav role="tablist">`）时保留控件角色。显式 `role` 属性优先于标签推断。这只影响读屏与 agent a11y dump——`<search>`、`<form>` 仍是布局盒，搜索与表单控件仍用 `search-dropdown` / `form-field`。

两种写法可以混在同一棵界面里。对话框、抽屉、菜单请用对应的 Nana 控件，不要用 `position: fixed` 自己搭网页浮层。

## 它提供的 Web 面，以及明确没有的

为了让熟悉的写法落到桌面窗口，而不是复刻浏览器：

有：`window` / `document` 的一个子集、事件、定时器、`requestAnimationFrame`、本地存储、桌面剪贴板、缓冲式 `fetch`（读完整响应再交给你）。

没有：完整 DOM / CSSOM、流式请求体、cookie、浏览器 CORS、Service Worker、Tauri invoke / 插件 / 窗口协议。未实现的 `fetch` 选项会报错，不会假装成功。

`WebSocket` 是预留接口：JS 面有 `WebSocket` 构造器（ws/wss URL、`send`/`close`、`onopen/onmessage/onclose/onerror`），但框架不内置任何传输。应用不注入实现时，`new WebSocket()` 直接报"不可用"；注入方式与 `fetch_host` 相同，通过 `MountOptions.socket_host` 提供应用自己的 `WebSocketHost` 实现，并为其配置 `SocketPolicy` 源白名单（默认全拒绝）。入站消息和连接状态事件在下一帧泵里送达回调。

## 网络与宿主命令

网络默认全关。应用必须列出允许的源，格式 `scheme://host[:port]`。localhost 不会自动放行。跨源跳转时，即使目标在白名单里，授权类请求头仍会被拿掉。默认超时 30 秒，请求和响应各 16 MiB，最多 5 次重定向。

NanaUI 不内置登录、设置存储或任何产品业务。你在宿主里注册自己的命令（`HostApiRegistry`），再交给 Vue 调用。框架自带的接口名和你注册的名字不能冲突，冲突时启动失败。

## 扩展控件

要让一种新控件进入布局、点击和绘制：在 Rust 里 `register_component`，Vue tag 等于 `ComponentTypeId` 去掉 `nana.` 前缀（`nana.preview-card` → `preview-card`）。和 HTML 同语义就用原生标签（`button`、`table`/`tr`/`td`）。语义不同就换名（`search-dropdown`，不是 HTML `<search>`）。只暴露 JS 命令时走 `NativeComponentRegistry`。两张表不是同一条 ABI，只登记其中一张，另一条路径不会生效。见 [控件](components.md)。
