# Fetch

网络权限在宿主，不在页面。默认一个源都不放行。你先列出允许的 origin，页面里的 `fetch()` 和样式里的远程图片才走得通。

```js
const response = await fetch(url);
const reader = response.body.getReader();
const { value, done } = await reader.read();
```

```rust
use nana_ui::{FetchPolicy, NativeFetchHost, shared_fetch_host};
use nana_ui_vue::{MountOptions, mount_vue_as_nana};

let policy = FetchPolicy::default().with_allowed_origin("https://api.example.com")?;
let app = mount_vue_as_nana(MountOptions {
    fetch_host: Some(shared_fetch_host(NativeFetchHost::new(policy))),
    ..MountOptions::default()
});
```

`FetchPolicy`、`NativeFetchHost` 和 `shared_fetch_host` 在开启 `gpu` 时从 `nana_ui` 再导出，定义在 `nana_ui_platform`。`MountOptions` 在 `nana_ui_vue`。`WebApiState::new()` 默认装的就是 `NativeFetchHost::new(FetchPolicy::default())`，白名单是空的。

## 白名单和上限

源用 `allow_origin` / `with_allowed_origin` 登记，格式 `scheme://host[:port]`。带路径、query 或 fragment 报 `InvalidRequest`。不是 http / https 报 `Policy`。放行 `https://example.com` 就放行它下面的所有路径。`https://api.example.com` 是另一个 origin，要单独登记。localhost 也要单独放行。

| 字段 | 默认 |
| --- | --- |
| `timeout` | 30 秒，跨重定向共享 |
| `max_request_bytes` | 16 MiB（`16 * 1024 * 1024`） |
| `max_response_bytes` | 16 MiB，流式按累计字节算 |
| `max_redirects` | 5，超过报 `Redirect` |
| `worker_count` | 4 |

`NativeFetchHost` 是阻塞的 `ureq`，必须跑在 UI 线程和 JS 线程之外。每一跳都重新核对白名单。跨源时摘掉 `authorization` 和 `proxy-authorization`。303，以及 POST 的 301 / 302，转成 GET，并清空正文和 `content-length` / `content-type`。响应里的 `set-cookie` 不会交给 JS。

错误按 `FetchErrorKind`：`Policy`、`InvalidRequest`、`Network`、`Timeout`、`RequestTooLarge`、`ResponseTooLarge`、`Redirect`、`Cancelled`、`Unsupported`。

`worker_count` 条阻塞线程调用 `fetch_streaming`。队列长度是 `worker_count * 2`。队列满时 `fetchStart` 抛出「fetch worker queue is full」，不堵住发起请求的那条线程。响应头、分块和错误在帧泵里回到引擎线程。

## 和 `url()` 是同一条路

`url()` 图片、`<img src>`、`mask-image`、`border-image` 和 JS `fetch()` 共用同一个 `FetchHost`、同一份 `FetchPolicy`：同样的白名单，同样逐跳复核重定向，同样在跨源时摘掉授权头。

没装 host，远程图就不取。`data:`、`file:` 和 jail 里的相对路径不受影响。host 按文档分开。同一个进程里多次 mount、多个窗口共用一个 painter 时，每个文档的图片只经过它自己的 host，缓存也按 host 分开。

纯 Rust 窗口的默认是 `None`，远程图会被拒绝。给窗口填 `RuntimeProgram::resource_fetch_host`，或 `ApplicationWindow.fetch_host`。自己持有 painter 时，绘制某个文档之前用 `SceneWgpuPainter::set_resource_fetch_host` 把该文档的 host 交过去。`NanaVueApp::fetch_host` 是嵌入式宿主的同一件事。Vue 宿主会返回该窗口 web-api 的那一个 host。

取消发生在 painter 析构，以及图片连续 120 帧没有引用的时候。结束的是网络等待，不是已经开始的解码。`@font-face` 不走网络，只认 `local()`、`data:` 和 jail 内的本机文件。

## 页面上能读到什么

`fetch()` 在响应头到达时 resolve，正文随后分块到达。`text()`、`json()`、`arrayBuffer()`、`blob()` 仍然读完再返回。`getReader()` 之后再调用 `text()` 或 `clone()` 会抛 `TypeError`。101、204、205、304，以及无正文的 `new Response()`，`body` 是 `null`。

没有真正的背压。`getReader({ mode: "byob" })` 会报错。请求正文在交给宿主之前已经是完整字节，`duplex` 会被拒绝。`FormData` 可以当正文，编码为 `multipart/form-data`。`new FormData(formElement)` 会报错。

没有 cookie jar。请求头里的 `cookie` / `set-cookie` 直接 `TypeError`。没有 CORS，也不会发 `OPTIONS`。`cache` 选项会被拒绝。流式响应的宿主接口是 `FetchHost::fetch_streaming` 和 `FetchSink`（先 `head` 一次，再 `chunk`）。只实现了 `fetch` 时，默认实现把整份正文当成一块发出，上限仍按该宿主的 `policy()` 判断。

`WebSocket` 另要一份 `SocketPolicy`。经 `MountOptions.socket_host` 或 `WebApiState::set_socket_host` 装上。默认空白名单会在连接前失败。

```bash
cargo test -p nana-js-v8 --features engine --locked vue_native_websocket -- --test-threads=1
```

宿主合同在 [应用 API](../../reference/application-api.md#fetch-宿主)，页面这一侧在 [Vue](../../reference/vue.md#网络与宿主命令)。

## 接下来

<div class="next-steps">
  <a class="next-step" href="/guide/components/async">
    <p class="next-step-link">异步</p>
    <p class="next-step-caption">Rust 视图里的 resource 跑在 UI 线程的执行器上。</p>
  </a>
  <a class="next-step" href="/guide/scaling/packaging">
    <p class="next-step-link">打包</p>
    <p class="next-step-caption">样式里的 nana://res 从资源包读取。</p>
  </a>
  <a class="next-step" href="/guide/scaling/accessibility">
    <p class="next-step-link">无障碍</p>
    <p class="next-step-caption">名字、隐藏层级和窗口根。</p>
  </a>
</div>
