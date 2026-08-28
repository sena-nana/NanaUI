# 应用 API

查入口用。第一次写应用先看 [开始](start.md) 和 [框架如何运行](how-it-works.md)。签名以 rustdoc 为准，这篇不复制每一份类型。

## 你该依赖什么

| 消费方 | crate / 包 | 入口 |
| --- | --- | --- |
| 新的桌面界面 | `nana-ui`（feature `hosted`） | `nana_ui::runtime`、`RuntimeProgram`、`run_runtime` |
| 窗口设置 / 输入类型 | 通常经 `nana-ui` 再导出；需要时直接 `nana-ui-platform` | `WindowSettings`、`WindowCommand`、`InputEvent` |
| Vue 宿主 | `nana-ui-vue` + `nana-js-v8` | `nana_ui_vue::prelude`（`VueRuntimeProgram::run`） |
| Vue 控件 | `@nanaui/nanavue-components` | `NanaButton` 等 |
| Vue renderer | `@nanaui/nanavue-runtime` | `createNanaApp()` |

不要直接依赖 `nana-ui-devtools`、`nana-css-parity` 来画产品界面。前者是无头调试，后者是 CSS 对照测试。

新代码从 `nana_ui::runtime` 引入控件。crate 根再导出是兼容面。`runtime::internal` 给 Gallery 和宿主适配器，不是第二套产品 API。`runtime::host` 是 Scene / GPU slot 类型；`runtime::perf` 是帧计数，不是视图状态。

## Cargo feature

`nana-ui` 默认 `[]`。按职责打开：

| feature | 作用 |
| --- | --- |
| `hosted` | `run_runtime`、crates.io winit、AccessKit；隐含 `gpu` |
| `gpu` | `SceneWgpuPainter`、`HostTexture`、`GpuView` |
| `bundled-fonts` | 嵌入 Noto Sans SC |
| `components` | 下面组件族的聚合 |
| `full` | fonts + components + hosted + syntax-highlighting |
| `calendar` / `charts` / `controls` / `feedback` / `graph-canvas` / `image-viewer` / `overlays` / `popover` / `qr-code` / `rich-text` / `math` / `diagrams` / `selects` / `settings-components` / `surfaces` / `xy-pad` | 适配器再导出。`math` / `diagrams` 是 `rich-text` 别名（mermaid / 公式由应用画） |
| `syntax-highlighting` | `TextArea` 的 `"highlight"` presenter |

Cargo 不会因你写了 `CalendarHeatmap` 就自动打开 `calendar`。

## RuntimeProgram

应用实现这个 trait，再 `run_runtime::<App>(RuntimeWindowSettings::new("…"))`。

| 方法 | 职责 |
| --- | --- |
| `initialize` | 建程序实例；可返回要在第一帧 `update` 的消息 |
| `document` / `document_mut` | 按 `WindowId` 交出 `RuntimeDocument` |
| `update` | 宿主级消息；保持便宜 |
| `theme_mode` | 深色 / 浅色 |
| `window_material_mode` | 可选；默认实色 |
| `host_textures` | 默认；slot → `HostTexture` |
| `prepare_window_frame` | flush 前准备纹理 |
| `window_frame_presented` | present 后释放旧资源 |
| `scene_gpu_renderers` | 高级。`None` = 演示 `"gpu-view"`；空表 = 不画 |
| `scene_resource_producers` | 高级。按图离屏；第一次可忽略 |
| `bind_window` | present 之后填内容 |
| `rebuild_gpu` | 设备丢失后重绑资源 |
| `window_event` / `input_event` | 窗口生命周期与原始输入 |
| `next_wakeup` / `wake` | 与重绘无关的定时工作 |
| `host_failure` | 宿主已从该错误恢复；默认忽略 |

`RuntimeProgramContext` 提供 `window_id`、`geometry`、`gpu()`、`material()`、`dispatch`、`run_task`。原生窗口句柄不穿过这条边界。

## 建树

```text
RuntimeDocument::new(DocumentId)
AppContext::build(document, |ui| {
    ui.column(12.0, |ui| {
        let save = ui.child("save", Button::new("…"));
        ui.on(save, |_, Activate, cx| { cx.dispatch_program(Msg); });
    })
})
mount { scope.child("key", …) }          // 动态区增删
update_component(entity, |view, _| { … }) // 文案 / loading
```

`build` 是初次整页（一次 commit）。`mount` 是 keyed 子树协调，不是第二套渲染器。点击 handler 不要再 `build` 一遍。Vue 不得用 `create_component` / `build` 分配 ID，它绑定自己已有的节点。细则见 [L3 组成式建树](l3-authoring.md)。

`create_component` / `append_child` / `on` 仍是底层 primitive。

对外身份是 `StableNodeId` / `Entity<V>`。不要依赖内部实体编码。

## 扩展控件

| 目标 | 路径 |
| --- | --- |
| 进入布局、命中、Scene | `UiExtension` + `register_component`；Vue 再加 `nana-*` |
| 仅 JS 命令 / props 白名单 | `NativeComponentRegistry` + `Nana.components.call` |
| GPU 内容 | `GpuTextureView` + 宿主纹理；直写见 `GpuView` |

不支持动态 dylib。

## 性能上你不用手写的

`build` 把整棵子树收成一次 commit。mutation 提交后 Runtime 自己调度脏工作。无变更不刷帧。大列表走 `materialize_virtual_*`。GPU 换纹理升 generation，不重建布局。`dispatch_program` 按类型保留最后一条，在下一帧 `update`。

## 非目标

- 完整浏览器、Tauri、裸 `@vue/runtime-dom` 产物、WebView 产品路径
- 第二套 Device / Queue、CPU 回读伪装零拷贝
- 控件拿窗口句柄
- 以 crate 根控件表或 Vue 的 DOM facade 定义新的框架合同
