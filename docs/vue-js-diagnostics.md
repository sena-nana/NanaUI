# Vue+JS 开发诊断接入

诊断只面向开发工具，不应渲染到产品 UI。一个 `DevtoolsSession` 可同时接收 V8、Vue、
Host Bridge、资源和帧数据：

```rust
let diagnostics = nana_ui_devtools::DevtoolsSession::default();

engine.set_diagnostic_sink(Some(diagnostics.js_sink()));
vue_host.set_diagnostics(
    Some(diagnostics.js_sink()),
    Some(diagnostics.host_call_observer()),
);
```

多窗口使用 `VueRuntime::set_diagnostics`；设置会同步到现有窗口，并由之后创建的窗口
继承。资源快照由开发工具在需要时采集，避免产品运行时持续序列化：

```rust
diagnostics.set_resource_count(
    "v8.handle",
    engine.host_resources().map(|resources| resources.len()).unwrap_or(0),
);
diagnostics.set_resource_counts("vue", vue_host.resource_counts());
```

宿主可在 present 后将帧间隔转发给
`DevtoolsSession::record_frame`。宿主提供目标刷新间隔，诊断器据实际 present 间隔估算
丢帧；WGPU GPU timestamp query 仍由需要它的渲染器显式启用。

## V8 Inspector

`V8Engine::enable_inspector` 在现有 isolate/context 内创建 CDP session，不监听网络端口，
也不创建第二个 JS 引擎。开发工具负责 WebSocket、鉴权和远程暴露策略：

```rust
let transport = engine.enable_inspector()?;
engine.dispatch_inspector_protocol_message(
    r#"{"id":1,"method":"Runtime.enable"}"#,
)?;
for message in transport.drain_messages() {
    // 转发给开发期 CDP 客户端。
}
```

CDP 客户端应先启用 `Runtime`，读取 `Runtime.executionContextCreated` 的 context id，再发送
需要指定 context 的命令。

## 数据边界

- Host API trace 不记录参数和返回值，只记录方法名、同步/异步状态、结果和耗时。
- V8 未捕获异常及未处理 Promise rejection 保留消息和调用栈。
- Vue warning/error 通过 `app.config.warnHandler` / `errorHandler` 上报。
- `HostedRuntimeEvent` 报告渲染暂停、Device Lost 原因、恢复及恢复失败。
- 事件队列有固定容量；资源统计为覆盖式快照，不会无限增长。
