# NanaUI 应用 API

产品路径是同一套权威：`UiWorld` → `UiScene` → `SceneWgpuPainter`。
L1 / L2 / L3 是三种输入合同，不是三套运行时。

## 选哪一层

| 层 | 入口 | 用途 |
|----|------|------|
| L1 | JS `createNanaApp()` + DOM/CSS 子集 + 应用 `HostApiRegistry` | Vue SFC / 网页习惯源码，经 Nana Vite 构建 |
| L2 | `@nanaui/nanavue-components`（`nana-*` 语义 props） | 跳过 CSS，直接语义控件；与 L1 同树 |
| L3 | `nana_ui::runtime`（`AppContext` / `create_component` / `on`） | Rust 保留树；宿主用 `RuntimeProgram` + `run_runtime` |

业务状态、鉴权、配置盘、Region 内容由应用拥有。NanaUI 只提供通用控件与合同。

## L3 最小路径

```rust
use nana_ui::runtime::{AppContext, Button, Entity, Text};
use nana_ui::{RuntimeProgram, run_runtime};

// 1. AppContext 持有 UiWorld 与 typed view state
// 2. create_component / mount 投影到保留树
// 3. on / observe 更新 view；不要把每次点击塞进 RuntimeProgram::Message
// 4. run_runtime 注入宿主 Window / Device / Queue，由 SceneWgpuPainter 绘制
```

- 新应用：`nana_ui::runtime`。crate 根控件表是兼容面。
- 宿主 Scene：`runtime` 或 `runtime::host`。观测：`runtime::perf`。
- Gallery / 适配器可用 `runtime::internal`；不要把它当第二套产品 API。

## L1 / L2

- JS：`createNanaApp()`（[`packages/nanavue-runtime`](../packages/nanavue-runtime/README.md)）。
- Rust 宿主：`nana_ui_vue::prelude`（`mount_vue_as_nana`、`NanaVueApp`、`VueRuntimeProgram`、输入类型、`HostApiRegistry`）。
- CSS cascade / `parse_stylesheet` / measure：adapter internals，不是应用 prelude。

`NanaTreeDocument` / `MessageBridge` / `LayoutBoxStore` 是 Vue 兼容投影，**不是**保留权威。
host op 进 `PendingHostOps`，`flush_host_frame` 才 commit。

## 扩展控件

| 目标 | 路径 |
|------|------|
| 进入布局、命中、Scene 的新语义控件 | `UiExtension` + `ExtensionRegistrar::register_component`（Runtime `ComponentRegistry` / `ComponentTypeId`）+ Vue `nana-*` tag。额外属性走 `SemanticSpec::attr`，不要为每个业务控件加 `WidgetKind`。 |
| 仅 JS 描述符、props 白名单、命令 | `NativeComponentRegistry`（JS host 组件工厂表）+ `Nana.components.call` |
| GPU 内容 | `CustomRenderNode` 不透明键 + 宿主 `HostTexture`（generation / `replace_view`）。不要把 Cubism 直写 Surface。 |

两张 Registry 不是同一条 ABI。只注册其中一张不会让另一条路径生效。不支持动态 dylib，不公开 Bevy Entity。

## 性能（默认语义，不要手写脏位）

- mutation commit 后 Runtime 自己调度；`DirtyMask` 不公开。
- 无变更不刷帧；文本 intrinsic 不变则 layout-stop。
- 大列表必须 `AppContext::materialize_virtual_*`，窗口外不建 live entity。
- GPU 换纹理升 generation/revision，不重建布局。
- 不要把 facade 查询（例如 `LayoutBoxStore::snapshot`）当成每帧热路径。

## 非目标

- 以 crate 根控件表定义新框架合同。
- 把三个 Vue facade 当成第二棵 ECS/DOM 树。
- 完整浏览器、Tauri、裸 `@vue/runtime-dom` 产物、WebView 产品路径。
- 第二套 Device/Queue、CPU 回读伪装零拷贝、控件拿窗口句柄。

细则：[`architecture.md`](architecture.md)、[`runtime-scene.md`](runtime-scene.md)、[`capabilities.md`](capabilities.md)。
