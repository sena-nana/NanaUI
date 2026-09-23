# Vue

Vue + JS 是 NanaUI 的一等 L1/L2 消费方，与 Rust L3 共用 Runtime、UiScene 和组件注册合同。应用可按团队的开发习惯选择入口，Rust 入口见 [开始](start.md)。

Vue 用来把已经按网页习惯写好的界面落到**同一棵**原生树上。写法和网页接近，跑起来不是网页：没有 WebView，也不能把普通 `@vue/runtime-dom` 网站产物丢进来当桌面应用。

## 应用怎么接

JavaScript 入口是 `@nanaui/nanavue-runtime` 的 `createApp()`。你自己的 Vite 工程把 SFC、TypeScript 和 CSS 打成 Nana 能加载的脚本（通常是 IIFE）。NanaUI 不扫描 `dist`，也不提供另一套打包器。开发期不想每改一次就重启，见 [开发期热重载](hot-reload.md)。

```js
import { createApp } from "@nanaui/nanavue-runtime";
import { NanaButton } from "@nanaui/nanavue-components";
import "@nanaui/nanavue-components/controls.css";

createApp({
  // 根组件
}).mount();
```

Rust 宿主用 `nana_ui_vue::prelude`：`VueRuntimeProgram::run`（或 `mount_vue_as_nana`）把这份脚本和 V8 引擎交给**同一个** `run_runtime`。`VueRuntimeProgram` 需要 feature `hosted`（隐含 `scene-view`，把 UiScene 交给 `SceneWgpuPainter`）。后打开的窗口默认共用这一套 JavaScript，也可以要求独立的 JavaScript 上下文，见下文[多窗口与 JavaScript 隔离](#多窗口与-javascript-隔离)；两种都共用同一个引擎和同一份 GPU。

## 多窗口与 JavaScript 隔离

`Nana.windows.create({ isolation })` 选择新窗口的 JavaScript 上下文：`"shared"`（默认）或 `"isolated"`。拼错的取值直接报错，不会悄悄退回共享。

**共享（默认）。** 新窗口与打开它的脚本在同一个全局环境里，下列状态跨窗口：JavaScript 全局与模块状态（Vue / Pinia store、组件注册）、`Nana.host.on` 监听与宿主事件队列、`setTimeout` / `setInterval` / `requestAnimationFrame` 的回调表与编号、`fetch` / `WebSocket` 的 JS 对象、`localStorage`、宿主资源句柄、`Nana.dialogs` provider 与 `Nana.components.onError` 监听。仍然每扇窗一份的是：文档、`window` / `document` 对象、`sessionStorage`、定时器与网络请求在宿主侧的调度、焦点与 IME。

**隔离。** 新窗口得到自己的 JavaScript 上下文：同一份应用脚本在里面**重新执行一遍**，全局变量、模块状态、定时器、`localStorage` 都与其他窗口互不可见。`localStorage` 和 `Nana.storage` 只在内存里，窗口关闭即释放。脚本用 `Nana.windows.current()` 判断自己身在哪扇窗：

```js
const current = Nana.windows.current(); // { id, isolation, params, ... }
createApp(current.params?.view === "settings" ? Settings : Main).mount();
```

打开方通过 `params` 传入初始数据，须可 JSON 序列化（循环引用在 `create()` 时抛 `TypeError`）：

```js
const settings = await Nana.windows.create({ isolation: "isolated", params: { view: "settings" } });
settings.focus();
```

`params` 只送给隔离窗口。窗口种类用 `tag`，共享和隔离窗口都能用：它是字符串，出现在 `create()`、`current()`、`list()` 返回的句柄上（`handle.tag`，未设置为 `null`），并原样成为 `WindowDescriptor::tag`，Rust 侧从 `RuntimeProgramContext::window_tag()` 读回，见 [窗口](window.md)。主窗口的 `tag` 取自传给 `VueRuntimeProgram::run` 的描述符，脚本首次执行时 `Nana.windows.current().tag` 就已可读。

```js
const tracking = await Nana.windows.create({ tag: "tracking", role: "tool" });
tracking.mount(tracking.tag === "tracking" ? TrackingPanel : Main);
```

打开方拿到的句柄可以控制窗口（`focus` / `close` / `setBounds` / `ready` / `closed` 等），但不能 `mount`——窗口内容由它自己的上下文挂载，句柄上的 `window` / `document` / `root` 为 `null`。在隔离窗口里再打开的共享窗口属于这个隔离上下文。`Nana.windows.list()` 只列出当前上下文里的窗口。一个隔离上下文在它最后一扇窗口关闭、`window-closed` 送达并完成卸载后销毁。

**两种模式都跨窗的**：GPU Device / Queue 以及 WebGPU、Canvas、SVG、媒体、视频运行时，原生组件与宿主纹理注册表，应用样式表，动画时钟，诊断输出，你注册的宿主命令（它们的 Rust 状态），以及按窗口 id 生效的窗口控制。所有上下文还共用同一个 V8 堆、同一个线程和同一个微任务队列：隔离的是状态，**不是**性能或故障——一个窗口里的死循环仍会卡住所有窗口。窗口控制也不是安全边界，同一份脚本、同一个进程。

隔离窗口需要以源码形式加载的应用脚本；V8 snapshot 形式的脚本和不支持多上下文的引擎会让 `create()` 以 `WindowOpenError` 失败。

JS 的 `windowSetFullscreen` / `windowSetAlwaysOnTop` 接口不变（仍是布尔）。`windowGeometry().fullscreen` 和窗口的 `alwaysOnTop` 现在是宿主观察到的值：请求入队时不再乐观写入，要等 `WindowEvent::ModeChanged` 回流。调用后立刻读几何可能仍是旧值。

窗口化对照 `examples/vue-hosted-acceptance`。`examples/vue-counter` 是引擎探针（含无头点击），不是应用模板。

## 启动

Early Splash 在 Rust 侧配置：`nana_ui::with_startup(StartupOptions::default().with_splash(spec), || VueRuntimeProgram::run(...))`。bundle 在 `UiReady` 求值，那时 Logo 已在屏幕上。`Nana.startup` 是宿主启动记录的投影：`state` 随时读取（`{ phase, splash, ticket, timeline }`），`deferTakeover()` 只在首次求值时有效，`takeOver()` / `cancelTakeover()` 发请求，`onChange(listener)` 收宿主的 `startup` 事件。合同见 [两阶段启动](startup.md)。

## 两种写法，同一棵树

**Nana 控件。** `NanaButton`、`NanaInput`、`NanaDialog` 直接表达语义，Vue 标签和 Rust `create_component` 通过同一份 `ComponentRegistry` 解析组件类型。

**普通标签和 CSS。** `div`、flex、间距、字号这一类网页习惯可用，但只覆盖 [布局](layout.md) 列出的子集。适合结构骨架，不适合冒充完整浏览器。

和 Runtime 同语义的 Vue 标签会落到对应控件：`button`、`a`、`input`（含 `checkbox` / `radio` / `range` / `number`）、`textarea`、`select` + `option`、`ul`/`ol`/`menu`/`li`、`table`/`tr`/`td`/`th`、`progress`、`meter`、`hr`、`dialog`、`details`/`summary`。布局与文本骨架标签（`div`、`section`、`figure`、`figcaption`、`hgroup`、`address`、`picture`、`datalist`、`slot`、`map` 等）落到 Column / Text。未识别的标签和退役的 `nana-button` 一类别名会报错，不会当成布局盒；插件 tag 须先 `register_component`。`v-html` 会把片段解析成子节点。`Teleport to="body|html"`、`Transition`、`KeepAlive`、`Suspense` 走同一套 host ops，没有第二棵树。

语义不同就换名：`search-dropdown` 不是 HTML `<search>`；`nana-scroll-view` 不是随便一个 `div`。`<iframe>`、`<audio>`、`<embed>`、`<object>` 不伪造浏览器，落为不可见布局盒——`<video>` / `nana-video` 在有 `data-nana-video` 时走宿主推帧（`nana.video`，槽 `video:{id}`）；无槽才显示 `poster`。应用内打开网页的拟议控件是 `webview`（`nana.webview`），不是 `<iframe>`，目前未实现，见 [应用内浏览器](gpu.md#应用内浏览器)。`<audio>` 不解码、不写空视频帧；PCM 播放走上面的 `AudioContext` 子集。`<source>` / `<track>` / `<area>` 没有自身视觉，`<col>` / `<colgroup>` 在 Runtime Table 里没有列定义，都显式跳过。

地标标签携带 a11y landmark role：`nav` → navigation、`main` → main、`aside` → complementary、`search` → search、`header` → banner、`footer` → contentinfo（`header` / `footer` 是 `article` / `aside` / `main` / `nav` / `section` 后代时除外）；`section` / `form` 只有带可访问名时才是 region / form——名字可以来自 `aria-label`、`aria-labelledby` 或自身文本内容。class / role hints 把地标标签改成具体控件（如 `<nav role="tablist">`）时保留控件角色。显式 `role` 属性优先于标签推断。这只影响读屏与 agent a11y dump——`<search>`、`<form>` 仍是布局盒，搜索与表单控件仍用 `search-dropdown` / `form-field`。

两种写法可以混在同一棵界面里。对话框、抽屉、菜单请用对应的 Nana 控件，不要用 `position: fixed` 自己搭网页浮层。

## 长列表上的 hover

一个 render function 拥有整列时，hover 处理器改一个参与渲染的 `ref` 会让 Vue 重建整列的 vnode——实测 2,000 行时一次鼠标移动 9 ms（超过半帧），是同一棵树无处理器时的 140 倍。模板里的 `v-for` 编译出来就是这个形状，不生成子组件。这是长列表上最容易写出来的写法，也是这条路径上目前唯一还随树增长的成本。把行拆成各自的组件能省 39%（2,000 行 8.6 → 5.3 ms），但**不会变成常数**——剩下的在 Runtime 的脏帧路径上（2 个脏节点在 2,000 节点的树上要 1.5 ms，是 Issue #8 一直未过的门禁），与 Vue 怎么写无关，见 [输入成本](input-cost.md)。今天就能拿到常数的写法是让 hover 只改不参与渲染的状态。

事件本身不贵：Vue 每个指针事件是常数约 0.062 ms，不随树增长（Rust L3 是 0.0003 ms，也是常数）。数据与四轮改动的经过见 [输入成本](input-cost.md)。

## Markdown

`NanaMarkdown` 用 `modelValue` 或 `value` 传入原始 Markdown，需启用 `rich-text`。源码未变化时保留解析结果和选区；源码变化后重新解析。高亮由共享 Runtime 绑定路径处理。

## 它提供的 Web 面，以及明确没有的

为了让熟悉的写法落到桌面窗口，而不是复刻浏览器：

有：`window` / `document` 的一个子集、事件、定时器、`requestAnimationFrame`、本地存储（NanaUI 的存储就是这份 `localStorage`；默认内存，宿主注入 `FileStore` 后主上下文可落盘）、`Nana.storage`（同一张表上的 JSON 助手）、桌面剪贴板、`fetch`（响应头到了就 resolve，正文可以边到边读）、Web Audio 的 PCM 子集（`AudioContext`、从 `Float32Array` 填充的 `AudioBuffer`、`AudioBufferSourceNode`、`GainNode`、`destination`、`ScriptProcessorNode` / `onaudioprocess`）。桌面输出走 cpal；无宿主或无输出设备时构造 `AudioContext` 抛 `NotSupportedError`。测试注入 mock sink，不依赖扬声器。这条路径不写 HostTexture。

没有：完整 DOM / CSSOM、流式**请求**体、cookie、浏览器 CORS、Service Worker、IndexedDB（`indexedDB.open` / `deleteDatabase` / `databases` / `cmp` 抛 `NotSupportedError`，结构化持久数据走 `Nana.storage` 或你注册的 `HostApiRegistry`）、Tauri invoke / 插件 / 窗口协议、完整 Web Audio 节点图 / 空间化 / `AudioWorklet` / `decodeAudioData`。未实现的 `fetch` 选项会报错，不会假装成功（`duplex` 仍在拒绝之列——请求侧流式正文需要分块上传，宿主协议还没有这条路）。`<audio>` 仍只是播放态 shim，不解码进 mixer。

`fetch()` 在响应**头**到达时就 resolve，和浏览器一样；正文随后分块到达，每一块在 `pump_frame` 里交给 JS，回调不离开引擎线程。`response.body` 是 `ReadableStream`：

```js
const response = await fetch(url);
const reader = response.body.getReader();
for (;;) {
  const { value, done } = await reader.read();
  if (done) break;
  // value 是 Uint8Array，这一块现在就能用
}
```

`text()` / `json()` / `arrayBuffer()` / `blob()` 仍然读完整份再 resolve，写法不用改。`clone()` 也照旧——已到达的分块会留着，两份可以各读各的。但**拿了 reader 就不能再走缓冲读法**：`getReader()` 之后 `text()` / `clone()` 抛 `TypeError`，和浏览器一致；读干净之后也一样（`bodyUsed` 会变真）。101/204/205/304 与无正文的 `new Response()` 的 `body` 是 `null`，不是一个永远空的流。

`ReadableStream` 也装成全局，`new ReadableStream({ start, pull, cancel })` 三个回调都接着，`controller` 有 `enqueue` / `close` / `error` / `desiredSize`。两点限制说在前面：**没有真正的背压**——`BodySource` 会留着全部分块好让 clone 各读各的，所以 `desiredSize` 报的是「还没被读走多少」，不会反过来卡住生产；**BYOB reader 不支持**，`getReader({ mode: "byob" })` 直接报错，它需要调用方自己的缓冲区，宿主通道没有这条路。

流式是为了**早点开工**，不是为了绕开上限：上限按**累计**字节算，超了就在中途以 `ResponseTooLarge` 中断这条流，不会因为分块就放行更大的正文。

上限的具体数值来自**执行这次请求的宿主自己的** `FetchPolicy`（内置 `NativeFetchHost` 是 16 MiB）。宿主侧的对应接口是 `FetchHost::fetch_streaming`，默认实现回落到缓冲式并把整份正文当成一块发出——所以只实现了 `fetch` 的应用宿主照常能用，只是不会流；默认实现同样会按该宿主 `policy()` 声明的上限裁剪，不会因为它没实现流式就把上限漏掉。

`FormData` 可以直接当 `fetch` 的正文：`append` / `set` / `get` / `getAll` / `has` / `delete` / 迭代都在，编码为 `multipart/form-data`，boundary 由框架生成并写进 `content-type`（你自己写了 `content-type` 就不覆盖）。文件项传 `Blob`，字节走已有的资源对象通道。`new FormData(formElement)` 不支持——它要走真实表单控件，直接报错而不是发一个空正文。

`WebSocket` 由桌面端内置 `NativeWebSocketHost` 提供传输（ws/wss URL、`send`/`close`、`onopen/onmessage/onclose/onerror`）。默认 `SocketPolicy` 仍为空白名单，应用必须通过 `MountOptions.socket_host` 注入配置了 `SocketPolicy` 源白名单的 host，或调用 `WebApiState::set_socket_host` 替换默认 host；未授权源会在连接前失败。网络 I/O 在线程中执行，入站消息和连接状态事件在下一帧泵里送达回调。

桌面端到端验收使用真实 V8 + Vue 和本地 loopback 服务端，覆盖白名单拒绝、`onopen` 中 `send`、消息/关闭事件的分帧与同帧回调、Vue 状态更新及关闭应答：

```bash
cargo test -p nana-js-v8 --features engine --locked vue_native_websocket -- --test-threads=1
```

## 网络与宿主命令

网络默认全关。应用必须列出允许的源，格式 `scheme://host[:port]`。localhost 不会自动放行。跨源跳转时，即使目标在白名单里，授权类请求头仍会被拿掉。默认超时 30 秒，请求和响应各 16 MiB，最多 5 次重定向。

受管的不只是 JS：同一个 `fetch_host` 也是这个文档在引擎里的资源出口（按 mount / 窗口各自生效，不是进程级）——`url()` 图片、`<img src>`、`mask-image`、`border-image` 走的是同一份 `FetchPolicy`，同样逐跳复核重定向，也同样可取消——painter 拆掉或图片不再被引用时，在飞的请求会被 shutdown，不会挂到超时才收场。没注入 host 就一张远程图都不取，`data:`、`file:` 与相对路径不受影响。`@font-face` 不取远程，只认 `local()`、`data:` 和 jail 内的本机文件。

NanaUI 不内置登录或任何产品业务。存储就是一份 `localStorage`：宿主注入 `PersistentStore`（默认内存；`FileStore::open(app_data_dir("YourApp")?)` 落成目录里的 `local-storage.bin`）。`Nana.storage` 是同一张表上的 JSON 助手（`set` 写入 `JSON.stringify` 后的字符串）。Dock / Appearance / 窗口几何也写进这张表，key 分别为 `nana.dock.{persistKey}`、`nana.appearance.{persistKey}`、`nana.window.{persistKey}`。`Nana.storage` 的 `get` / `set` / `clear` / `remove` / `keys` 会留下这些框架 key；`localStorage` 仍能读、写、删它们。隔离窗口仍是私有内存桶。你还可以注册自己的命令（`HostApiRegistry`）交给 Vue 调用。框架自带的接口名和你注册的名字不能冲突，冲突时启动失败。

```js
await Nana.storage.set("session", { user: "nana" });
const session = await Nana.storage.get("session"); // { user: "nana" } 或 null
```

```rust
VueRuntimeProgram::run_with_store(
    settings.persist_key("main"),
    engine,
    artifact,
    application_api,
    shared_store(FileStore::open(app_data_dir("YourApp")?)?),
)?;
```

## 扩展控件

要让一种新控件进入布局、点击和绘制：在 Rust 里 `register_component`，Vue tag 等于 `ComponentTypeId` 去掉 `nana.` 前缀（`nana.preview-card` → `preview-card`）。和 HTML 同语义就用原生标签（`button`、`table`/`tr`/`td`）。语义不同就换名（`search-dropdown`，不是 HTML `<search>`）。只暴露 JS 命令时走 `NativeComponentRegistry`。两张表不是同一条 ABI，只登记其中一张，另一条路径不会生效。见 [控件](components.md)。
