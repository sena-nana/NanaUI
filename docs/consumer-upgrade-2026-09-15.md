# 消费方升级记录（2026-09-15）

本轮补齐 Issue 51：叠加工具窗可以不出现在任务栏；Issue 54：Vue 窗口可以拥有独立的 JavaScript 上下文；Issue 53/165：可注入的分层存储与 ViewState restoration。

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

### 持久化（localStorage 与 ViewState 分层）

- `KvBackend` / `SharedStore` / `MemoryStore`（`nana-ui-core`）与 `FileStore` / `app_data_dir`（`nana-ui-platform`）提供物理存储。`LocalStorageAdapter` 使用独立 `nana.app.*` namespace，`ViewStateStore` 使用版本化 `nana.view.v1.*` namespace，`AppSettings` 使用 `nana.settings.v1.*` namespace；三者可共享物理文件，但不可共享 authority。
- 新增 `run_runtime_with_store` / `VueRuntimeProgram::run_with_store` / `VueHostedRuntime::with_store` / `VueRuntime::with_store`。
- `WindowDescriptor` 与 `VueWindowOptions` 新增 `persist_key`（JS：`persistKey`）。有 key 时框架把几何写到版本化 ViewStateStore（旧 `nana.window.*` 只迁移一次）。
- `Nana.storage` 是应用 namespace 上的 JSON 助手。JS `localStorage` / `Nana.storage` 不能枚举、清除或伪造 ViewState/Settings；隔离窗口只隔离应用 KV。`indexedDB` 的 `open` / `deleteDatabase` / `databases` / `cmp` 抛 `NotSupportedError`。
- `DockWorkspace` / Window geometry 通过 `ViewStateStore` 保存；`AppearanceSettings` 通过 `AppSettings` 保存。旧 `nana.dock.*`、`nana.window.*`、`nana.appearance.*` 只迁移一次。

## 各应用

| 应用 | 适配 |
| --- | --- |
| 直接构造 `WindowDescriptor` 字面量的应用 | 补 `skip_taskbar: false`、`persist_key: None`、`restoration_scope: RestorationPath::root()`，或改用 `..Default::default()` |
| 穷尽匹配 `WindowEvent` 的应用 | 增加 `SkipTaskbarChanged` 分支（或继续用 `_`） |
| 直接构造 `VueWindowOptions` 字面量的应用 | 补 `isolation: VueWindowIsolation::Shared`、`persist_key: None`，或改用 `..Default::default()` |
| 调用 `VueHost::bind_event_bridge_for_window` 的应用 | 传入窗口脚本所在的 `JsRealmId`（共享窗口为 `JsRealmId::MAIN`） |
| 需要跨进程 localStorage / 窗口几何的应用 | 注入 `FileStore` 并设置 `persist_key`；否则保持内存-only |

公开合同见 [窗口](window.md) 与 [Vue](vue.md#多窗口与-javascript-隔离)。
