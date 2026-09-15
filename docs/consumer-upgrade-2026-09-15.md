# 消费方升级记录（2026-09-15）

本轮补齐 Issue 51：叠加工具窗可以不出现在任务栏；Issue 54：Vue 窗口可以拥有独立的 JavaScript 上下文。

## API 变化

### 窗口

- `WindowDescriptor` 新增 `skip_taskbar: bool`，默认 `false`。
- 新增 `WindowCommand::SetSkipTaskbar { id, skip_taskbar }` 与 `WindowHandle::set_skip_taskbar`。
- 新增 `WindowEvent::SkipTaskbarChanged { id, skip_taskbar, result }`：每次请求都回报，`skip_taskbar` 为实际状态。
- Windows 生效；macOS、Linux 返回 `Unsupported`。
- `nana-window` 新增 `set_skip_taskbar` 与 `SkipTaskbarError`，供宿主适配器使用；普通控件仍拿不到窗口句柄。
- Vue JS 接口不变。

### Vue 窗口隔离

- `Nana.windows.create` 新增 `isolation: "shared" | "isolated"`（默认 `"shared"`，行为不变）与 `params`（仅隔离窗口，须可 JSON 序列化）；新增 `Nana.windows.current()`。句柄新增 `isolation` / `params`；打开方拿到的隔离窗口句柄 `window` / `document` / `root` 为 `null`，`mount` 抛 `InvalidAccessError`。
- `nana-js-engine` 新增 `JsRealmId` 与 `JsEngine::{create_realm, dispose_realm, register_host_api_in, initialize_in, resolve_function_in, host_event_sender_in}`，均有默认实现：自定义引擎不改也能编译，只是隔离窗口会以 `WindowOpenError` 失败。
- `VueWindowOptions` 新增 `isolation: VueWindowIsolation`。
- `VueHost::bind_event_bridge_for_window` 新增 `realm: JsRealmId` 参数（位于 `engine` 之后）。
- `VueRuntime` 新增 `register_host_apis` 与 `dispose_released_realms`；自己驱动 `VueRuntime` 而不经 `VueHostedRuntime` 的宿主，在窗口 `Closed` / `OpenFailed` 之后调用后者，GPU 绑定或替换后调用前者。

## 各应用

| 应用 | 适配 |
| --- | --- |
| 直接构造 `WindowDescriptor` 字面量的应用 | 补 `skip_taskbar: false`，或改用 `..Default::default()` |
| 穷尽匹配 `WindowEvent` 的应用 | 增加 `SkipTaskbarChanged` 分支（或继续用 `_`） |
| 直接构造 `VueWindowOptions` 字面量的应用 | 补 `isolation: VueWindowIsolation::Shared`，或改用 `..Default::default()` |
| 调用 `VueHost::bind_event_bridge_for_window` 的应用 | 传入窗口脚本所在的 `JsRealmId`（共享窗口为 `JsRealmId::MAIN`） |

公开合同见 [窗口](window.md) 与 [Vue](vue.md#多窗口与-javascript-隔离)。
