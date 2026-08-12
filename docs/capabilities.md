# Web API 与应用宿主边界

`nana-ui-vue::VueHost` 默认只注册 renderer、DOM 子集和 Web API。NanaUI 不内置
workspace、secret、GitHub 或任何消费应用业务命令，也不提供 Tauri 权限模型。

## 应用 API

消费宿主用 `HostApiRegistry` 注册自己的业务命令和鉴权，再调用：

```rust
host.initialize_with_web_api_and_host_api(&mut engine, artifact, &application_api)?;
```

框架 API 与应用 API 在交给 JS 引擎前合并。任何重复名称都会使初始化失败，合并
保持原子性；应用不能覆盖 renderer 或 Web API。

## Fetch 授权

`nana-ui-platform` 拥有同步 `FetchHost` 合同和基于 `ureq 3.x + rustls` 的
`NativeFetchHost`。`FetchPolicy` 默认值：

| 项 | 默认 |
| --- | --- |
| origin | 空白名单，全部拒绝 |
| 总超时 | 30 秒 |
| 请求/响应 body | 各 16 MiB |
| 重定向 | 最多 5 次，每跳重新授权 |
| worker | 4 个，有界提交队列 |

origin 使用 `scheme://host[:port]` 的标准化精确匹配。消费应用必须显式创建策略，
NanaUI 不推断开发服务器、localhost 或业务域名。跨 origin 重定向即使目标也获授权，
仍会移除 Authorization 类敏感请求头。

`nana-ui-web-api` 在线程池执行阻塞后端；worker 只写完成队列。`VueHost::pump_frame`
在 JS 引擎线程结算 Promise，`VueHost::next_wakeup` 把 timer 和在途 fetch 合并为
宿主 wakeup；完全空闲时返回 `None`。

取消会立即以 `AbortError` 拒绝 JS Promise，并忽略迟到的阻塞后端结果。取消不声称
能强行中止操作系统中已经开始的同步 I/O。

## 不支持

流式 body、Blob/FormData、cookie、cache、ServiceWorker、浏览器 CORS/preflight、
WebSocket，以及 Tauri invoke/插件/窗口/事件/存储协议。非默认相关 Request 选项会
明确拒绝。

clipboard 与 Fetch 同为平台注入能力，但各自独立：桌面可用 `OsClipboard`，Android
仍使用诚实的 unsupported backend。
