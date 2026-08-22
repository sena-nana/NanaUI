# Vue → NanaUI 渲染系统

应用入口见 [`application-api.md`](application-api.md)。

## 合同

```text
Vue 3 SFC / TypeScript / JavaScript
        │ Nana Vite entry + createNanaApp
        ▼
reproducible IIFE + CSS subset
        │ V8
        ▼
Custom Renderer hostOps
        │
        ├─ L1 element/class/style ─┐
        └─ L2 nana component props ├─> MessageBridge / Style Model
                                  │
                                  ▼
                         Runtime / UiScene → SceneWgpuPainter
```

L1 是 WebView Vue 源码的 Nana 兼容子集，不是 WebView，也不是 Tauri。L2
`nanavue-components` 和 L3 Rust API 保留；L1/L2 可写入同一棵树，L3 宿主可把语义
快照嵌入自己的 Workspace Region。桌面可见内容走同一条 Runtime/UiScene 绘制路径；
`scene-view` 接入 Scene host 适配。

## 模块所有权

| 模块 | 责任 |
| --- | --- |
| `nana-js-engine` | 引擎无关值、函数和 `HostApiRegistry` |
| `nana-js-v8` | 产品 JS 引擎；`JsEngine` 是测试注入缝 |
| `nanavue-runtime` | Vue `createRenderer` hostOps 与事件桥 |
| `nana-ui-web-api` | window/document/EventTarget/timer/fetch 缓冲子集 |
| `nana-ui-platform` | clipboard、Fetch 策略和真实阻塞 HTTP(S) 后端 |
| `nana-ui-vue` | 树、MessageBridge、CSS 映射、语义快照与帧泵 |
| `nana-ui` | 公共 Rust UI 与 Scene host（`run_runtime` / `SceneWgpuPainter`） |

消费应用拥有业务状态、业务 Host API、鉴权、网络 origin 白名单、配置持久化和窗口。
`VueHost` 不内置 workspace/secret/GitHub 命令。

## 初始化

```rust
let mut host = mount_vue_as_nana(MountOptions {
    width,
    height,
    scale_factor: scale,
    fetch_host: Some(fetch_host),
    ..MountOptions::default()
});
host.inject_stylesheet(app_css);
host.initialize_with_web_api_and_host_api(&mut engine, app_iife, &application_api)?;
host.bind_event_bridge(&mut engine)?;
```

应用 API 与框架默认 API 一次性合并，重复名称直接失败。UTF-8 artifact 在初始化时
组合 Web API shim；Tauri 全局真实不存在。

## 帧循环

网络只在有界 worker 中执行。每个 UI wake：

1. `VueHost::pump_frame` 取出 fetch 完成事件并在引擎线程结算 Promise；
2. 排空 rAF/timeout/interval 与 JS microtasks；
3. 更新 Style Model 布局并通知 ResizeObserver 子集；
4. 宿主用新的 `SemanticSnapshot` 重绘。

宿主用 `VueHost::next_wakeup()` 合并 timer 与在途 fetch；空闲时不轮询。

## 支持边界

允许普通 Vue 3 Composition API、SFC template、TypeScript、Custom Renderer 和已记录的
DOM/CSS/Web API 子集。不支持真实 runtime-dom mount、完整 DOM/CSSOM、Tauri 行为、
流式 Fetch、cookie/CORS/WebSocket。未实现的非默认 Request 选项明确抛错。

权威构建夹具与双引擎测试位于
[`crates/nana-js-engine/fixtures/vue-sfc-compat`](../crates/nana-js-engine/fixtures/vue-sfc-compat)。
