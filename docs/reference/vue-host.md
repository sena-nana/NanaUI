# 宿主边界

同一棵树上，Vue 标签能落到什么、网络和存储有哪些、扩展控件怎么登记。接入步骤在 [Vue](vue.md)，多窗口在 [窗口](vue-windows.md)。

## 两种写法，同一棵树

**Nana 控件。** `NanaButton`、`NanaInput`、`NanaDialog` 直接表达语义。Vue 标签和 Rust 的 `create_component` 通过同一份 `ComponentRegistry` 解析组件类型。

**普通标签和 CSS。** `div`、flex、间距、字号这一类网页习惯可以用。但只覆盖 [布局](layout.md) 列出的子集。适合结构骨架。不适合冒充完整浏览器。

和 Runtime 同语义的 Vue 标签会落到对应控件。这些标签是 `button`、`a`、`input`（含 `checkbox`、`radio`、`range`、`number`）、`textarea`、`select` 加 `option`、`ul` / `ol` / `menu` / `li`、`table` / `tr` / `td` / `th`、`progress`、`meter`、`hr`、`dialog`、`details` / `summary`。

布局和文本骨架标签落到 Column 或 Text。例如 `div`、`section`、`figure`、`figcaption`、`hgroup`、`address`、`picture`、`datalist`、`slot`、`map`。

未识别的标签，以及退役的 `nana-button` 一类的别名，会报错。不会当成布局盒。插件 tag 须先 `register_component`。

`v-html` 会把片段解析成子节点。`Teleport to="body|html"`、`Transition`、`KeepAlive`、`Suspense` 走同一套 host ops。没有第二棵树。

语义不同就换名。`search-dropdown` 不是 HTML `<search>`。`nana-scroll-view` 不是随便一个 `div`。

`<iframe>`、`<audio>`、`<embed>`、`<object>` 不伪造浏览器。它们落为不可见的布局盒。

`<video>` 和 `nana-video` 在有 `data-nana-video` 时走宿主推帧（`nana.video`，槽是 `video:{id}`）。无槽才显示 `poster`。

网页内容使用 Runtime `BrowserView` 的宿主原生例外。不要通过 Vue `webview` 加载。也不要通过 `<iframe>` 加载。当前只有 macOS 后端。见 [应用内浏览器](gpu.md#应用内浏览器)。

`<audio>` 不解码，也不写空视频帧。PCM 播放走上面的 `AudioContext` 子集。

`<source>`、`<track>`、`<area>` 没有自身视觉。`<col>` 和 `<colgroup>` 在 Runtime Table 里没有列定义。它们都显式跳过。

地标标签携带 a11y landmark role。`nav` 是 navigation。`main` 是 main。`aside` 是 complementary。`search` 是 search。`header` 是 banner。`footer` 是 contentinfo。`header` 和 `footer` 是 `article`、`aside`、`main`、`nav`、`section` 的后代时除外。

`section` 和 `form` 只有带可访问名时才是 region 或 form。名字可以来自 `aria-label`、`aria-labelledby`，或自身文本内容。

class 和 role hints 把地标标签改成具体控件时，保留控件角色。例如 `<nav role="tablist">`。显式 `role` 属性优先于标签推断。

这只影响读屏，以及 agent 的 a11y dump。`<search>` 和 `<form>` 仍是布局盒。搜索和表单控件仍用 `search-dropdown` 和 `form-field`。

两种写法可以混在同一棵界面里。对话框、抽屉、菜单请用对应的 Nana 控件。不要用 `position: fixed` 自己搭网页浮层。

输入只有一条路。原生窗口、`VueHostedRuntime::runtime_input`，以及 `VueHost` 自己的 `dispatch_*` 和 `commit_text`，都先把事件交给 Runtime 路由。页面再收到 `pointer*`、`key*`、`wheel`、`input`、`composition*`。

所以焦点、Tab 顺序、滚动和文字编辑按 Runtime 的规则走。可聚焦的是注册成控件的标签。`tabindex` 不参与。页面对 `wheel` 或 `keydown` 调用 `preventDefault`，拦不住已经发生的滚动和输入。焦点离开时，未完成的组字被取消。

## 长列表上的 hover

一个 render function 拥有整列时，hover 处理器改一个参与渲染的 `ref`，会让 Vue 重建整列的 vnode。

实测 2,000 行时，一次鼠标移动是 9 ms，超过半帧。这是同一棵树、没有处理器时的 140 倍。

模板里的 `v-for` 编译出来就是这个形状。它不生成子组件。这是长列表上最容易写出来的写法。也是这条路径上目前唯一还随树增长的成本。

把行拆成各自的组件，能省 39%。2,000 行从 8.6 ms 到 5.3 ms。但**不会变成常数**。剩下的在 Runtime 的脏帧路径上。2 个脏节点在 2,000 节点的树上要 1.5 ms。那是 Issue #8 一直未过的门禁。这和 Vue 怎么写无关。见 [输入成本](input-cost.md)。

今天就能拿到常数的写法，是让 hover 只改不参与渲染的状态。

事件本身不贵。Vue 每个指针事件是常数，约 0.062 ms，不随树增长。Rust L3 是 0.0003 ms，也是常数。数据和四轮改动的经过见 [输入成本](input-cost.md)。

## Markdown

`NanaMarkdown` 用 `modelValue` 或 `value` 传入原始 Markdown。需要启用 `rich-text`。

源码未变化时，保留解析结果和选区。源码变化后，重新解析。高亮由共享的 Runtime 绑定路径处理。

## 它提供的 Web 面，以及明确没有的

这些面是为了让熟悉的写法落到桌面窗口。不是为了复刻浏览器。

有这些：`window` 和 `document` 的一个子集、事件、定时器、`requestAnimationFrame`、本地存储（应用 namespace；默认内存，宿主注入 `FileStore` 后主上下文可落盘）、`Nana.storage`（应用 namespace 上的 JSON 助手）、桌面剪贴板、`fetch`（响应头到了就 resolve，正文可以边到边读）、Web Audio 的 PCM 子集（`AudioContext`、从 `Float32Array` 填充的 `AudioBuffer`、`AudioBufferSourceNode`、`GainNode`、`destination`、`ScriptProcessorNode` / `onaudioprocess`）。

桌面输出走 cpal。没有宿主，或没有输出设备时，构造 `AudioContext` 抛 `NotSupportedError`。测试注入 mock sink，不依赖扬声器。这条路径不写 HostTexture。

没有这些：完整 DOM 和 CSSOM、流式**请求**体、cookie、浏览器 CORS、Service Worker、IndexedDB（`indexedDB.open`、`deleteDatabase`、`databases`、`cmp` 抛 `NotSupportedError`。结构化持久数据走 `Nana.storage`，或你注册的 `HostApiRegistry`）、Tauri invoke、插件、窗口协议、完整 Web Audio 节点图、空间化、`AudioWorklet`、`decodeAudioData`。

还没有的 `fetch` 选项会报错，不会假装成功。`duplex` 仍在拒绝之列。请求侧的流式正文需要分块上传，宿主协议还没有这条路。`<audio>` 仍只是播放态 shim，不解码进 mixer。

`fetch()` 在响应**头**到达时就 resolve。和浏览器一样。正文随后分块到达。每一块在 `pump_frame` 里交给 JS。回调不离开引擎线程。`response.body` 是 `ReadableStream`：

```js
const response = await fetch(url);
const reader = response.body.getReader();
for (;;) {
  const { value, done } = await reader.read();
  if (done) break;
  // value 是 Uint8Array，这一块现在就能用
}
```

`text()`、`json()`、`arrayBuffer()`、`blob()` 仍然读完整份再 resolve。写法不用改。

`clone()` 也照旧。已经到达的分块会留着。两份可以各读各的。

但**拿了 reader 就不能再走缓冲读法**。`getReader()` 之后，`text()` 和 `clone()` 抛 `TypeError`。和浏览器一致。读干净之后也一样。`bodyUsed` 会变真。

101、204、205、304，以及无正文的 `new Response()`，它们的 `body` 是 `null`。不是一个永远空的流。

`ReadableStream` 也装成全局。`new ReadableStream({ start, pull, cancel })` 三个回调都接着。`controller` 有 `enqueue`、`close`、`error`、`desiredSize`。

两点限制说在前面。**没有真正的背压。** `BodySource` 会留着全部分块，好让 clone 各读各的。所以 `desiredSize` 报的是「还没被读走多少」。不会反过来卡住生产。**BYOB reader 不支持。** `getReader({ mode: "byob" })` 直接报错。它需要调用方自己的缓冲区。宿主通道没有这条路。

流式是为了**早点开工**。不是为了绕开上限。上限按**累计**字节算。超了，就在中途以 `ResponseTooLarge` 中断这条流。不会因为分块就放行更大的正文。

上限的具体数值来自**执行这次请求的宿主自己的** `FetchPolicy`。内置 `NativeFetchHost` 是 16 MiB。

宿主侧的对应接口是 `FetchHost::fetch_streaming`。默认实现回落到缓冲式，并把整份正文当成一块发出。所以只实现了 `fetch` 的应用宿主照常能用，只是不会流。默认实现同样会按该宿主 `policy()` 声明的上限裁剪。不会因为它没实现流式，就把上限漏掉。

`FormData` 可以直接当 `fetch` 的正文。`append`、`set`、`get`、`getAll`、`has`、`delete` 和迭代都在。编码为 `multipart/form-data`。boundary 由框架生成，并写进 `content-type`。你自己写了 `content-type`，就不覆盖。

文件项传 `Blob`。字节走已有的资源对象通道。`new FormData(formElement)` 不支持。它要走真实表单控件。直接报错，而不是发一个空正文。

`WebSocket` 由桌面端内置的 `NativeWebSocketHost` 提供传输。支持 ws 和 wss URL、`send`、`close`，以及 `onopen`、`onmessage`、`onclose`、`onerror`。

默认 `SocketPolicy` 仍是空白名单。你必须通过 `MountOptions.socket_host` 注入配置了 `SocketPolicy` 源白名单的 host。或调用 `WebApiState::set_socket_host` 替换默认 host。未授权的源会在连接前失败。

网络 I/O 在线程中执行。入站消息和连接状态事件在下一帧泵里送达回调。

桌面端到端验收使用真实 V8 加 Vue，以及本地 loopback 服务端。覆盖白名单拒绝、`onopen` 中的 `send`、消息和关闭事件的分帧与同帧回调、Vue 状态更新，以及关闭应答：

```bash
cargo test -p nana-js-v8 --features engine --locked vue_native_websocket -- --test-threads=1
```

## 网络与宿主命令

网络默认全关。你必须列出允许的源。格式是 `scheme://host[:port]`。localhost 不会自动放行。

跨源跳转时，即使目标在白名单里，授权类请求头仍会被拿掉。默认超时 30 秒。请求和响应各 16 MiB。最多 5 次重定向。

受管的不只是 JS。同一个 `fetch_host` 也是这个文档在引擎里的资源出口。按 mount 或窗口各自生效。不是进程级。

`url()` 图片、`<img src>`、`mask-image`、`border-image` 走的是同一份 `FetchPolicy`。同样逐跳复核重定向。也同样可取消。painter 拆掉，或图片不再被引用时，在飞的请求会被 shutdown。不会挂到超时才收场。

没注入 host，就一张远程图都不取。`data:`、`file:` 和相对路径不受影响。

`@font-face` 不取远程。只认 `local()`、`data:`，以及 jail 内的本机文件。

NanaUI 不内置登录，也不内置任何产品业务。

存储由宿主注入物理 `KvBackend`。默认是内存。`FileStore::open(app_data_dir("YourApp")?)` 落成目录里的 `local-storage.bin`。

`localStorage` 和 `Nana.storage` 只访问应用 namespace。Dock 和窗口恢复由 `ViewStateStore` 负责。Appearance 由 Settings namespace 负责。

旧 key 只迁移一次。JS 无法枚举、清除或伪造 framework state。隔离窗口仍是私有的应用 KV。

你还可以注册自己的命令（`HostApiRegistry`），交给 Vue 调用。框架自带的接口名和你注册的名字不能冲突。冲突时启动失败。

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

要让一种新控件进入布局、点击和绘制，在 Rust 里 `register_component`。Vue tag 等于 `ComponentTypeId` 去掉 `nana.` 前缀。`nana.preview-card` 变成 `preview-card`。

和 HTML 同语义，就用原生标签。例如 `button`，以及 `table`、`tr`、`td`。语义不同就换名。例如 `search-dropdown`，不是 HTML `<search>`。

只暴露 JS 命令时，走 `NativeComponentRegistry`。两张表不是同一条 ABI。只登记其中一张，另一条路径不会生效。见 [控件](components.md)。
