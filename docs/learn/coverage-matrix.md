---
title: 24 crate 覆盖矩阵
---

# 24 Crate 覆盖矩阵：每个 Crate 在哪一章、查什么

本页是 [架构学习](/learn/) 板块的**全量检索底账**：工作区 `crates/` 下全部 **24** 个 crate，每个 crate 均具备四要素说明——**职责边界**、**依赖方向**、**关键入口类型（带 `文件:行号` 锚点）** 与 **归属主章**，并附带可复跑的只读核对命令。

```powershell
# 清点 crates/ 下的目录数（应输出 24）
(Get-ChildItem crates -Directory).Count
```

## 1. 主线 Crate（产品路径 4 个 + 文本引擎 1 个）

| Crate | 一句话职责 | 归属主章 | 依赖方向 | 关键入口与源码锚点 | 只读核对命令 |
| --- | --- | --- | --- | --- | --- |
| **`nana-ui-core`** | 全框架共享词汇：Style Model、几何、主题、Motion IR、WorkspaceModel | [核心合同](/learn/core-contracts) | 入：`nana-diagnostics`；出：几乎全部上层 crate | `LayoutStyle`（`box_layout.rs:3292`）、`SemanticColorRole`（`style_model.rs:257`） | `cargo tree -p nana-ui-core --depth 1` |
| **`nana-ui-runtime`** | 唯一权威保留树 `UiWorld`：变异、布局、输入路由、焦点、IME、无障碍、Shell/Dock/AreaLayout | [保留树](/learn/runtime-tree) · [布局引擎](/learn/runtime-layout) · [输入链路](/learn/runtime-input) · [工作台](/learn/workspace-shell-dock) | 入：`view-schema`、`view-macros`、`core`、`text`、`charts`、`input`、`diagnostics`；出：`nana-ui`、`scene`、`devtools` | `UiWorld`（`world.rs:628`）、`UiMutation`（`mutation.rs:15`）、`AppContext`（`framework.rs:1072`） | `rg "pub struct UiWorld" crates/nana-ui-runtime/src/world.rs` |
| **`nana-ui-scene`** | 后端中立绘制图 `UiScene` 与帧程序 `FramePlan`，不依赖 GPU | [Scene 提取](/learn/scene-extraction) | 入：`runtime`、`core`、`charts`、`diagnostics`；**无** `gpu` / `css` 依赖 | `UiScene`（`scene.rs:1357`）、`apply_delta`（`scene.rs:1672`）、`RuntimeDocument::flush`（`runtime_document.rs:109`） | `cargo tree -p nana-ui-scene --depth 1` |
| **`nana-ui`** | 宿主适配器：`SceneWgpuPainter`、宿主一帧调度、窗口服务、无障碍后端、无窗口输出 | [GPU 与帧](/learn/gpu-frame) · [窗口与平台](/learn/window-platform) | 入：`gpu`、`window`、`platform`、`scene`、`runtime`、`frame-exchange`、`svg-raster`、`package`、`icons` 等 | `SceneWgpuPainter`（`scene_paint/mod.rs:381`）、`run_runtime_scene`（`scene_host.rs:99`） | `rg "pub struct SceneWgpuPainter" crates/nana-ui/src/scene_paint/mod.rs` |
| **`nana-text`** | 产品唯一文本引擎：字体匹配、OpenType 整形、UAX #14 折行、保留布局句柄 | [文本引擎](/learn/text-engine) | 入：`nana-ui-core`、`fontdb`、`skrifa`、`harfrust`、`unicode-*`；出：`runtime`、`nana-ui`、`scene`（benchmark） | `TextEngine`（`engine.rs:25`）、`Shaper`（`shaping/shaper.rs:27`）、`Layouter`（`layout/engine.rs:157`） | `cargo test -p nana-text` |

## 2. 引擎与合同 Crate（CSS / 输入 / GPU / 帧交换，4 个）

| Crate | 一句话职责 | 归属主章 | 依赖方向 | 关键入口与源码锚点 | 只读核对命令 |
| --- | --- | --- | --- | --- | --- |
| **`nana-ui-css`** | 唯一 CSS 词法/解析/选择器匹配/级联引擎，产出 `LayoutStyle` 补丁 | [CSS 引擎](/learn/css-engine) | 入：`core`、`serde_json`；出：`view-codegen`（构建期） | `parse_stylesheet`（`css_cascade.rs:658`）、`rebuild_layout_style`（`:1606`） | `cargo tree -p nana-ui-css --depth 1` |
| **`nana-ui-input`** | 规范输入事件、有界端点、序列器与 `HostServices` 合同 | [输入链路](/learn/runtime-input) | 入：`core`、`diagnostics`；出：`runtime`（唯一路由消费方）、`platform`（再导出） | `CanonicalInputEvent`（`canonical.rs:44`）、`InputEndpoint`（`:410`）、`HostServices`（`host_services.rs:97`） | `cargo tree --workspace -i nana-ui-input --edges normal` |
| **`nana-gpu`** | GPU 后端合同：`GpuContext` / `FrameContext` / `GpuTexture`、代际与提交顺序 | [GPU 与帧](/learn/gpu-frame) | 入：`diagnostics`；出：`nana-ui`、`devtools`、`frame-exchange`（经 `__framework`） | `GpuContext`（`context.rs:358`）、`FrameContext`（`frame.rs:38`） | `rg "pub struct GpuContext" crates/nana-gpu/src/context.rs` |
| **`nana-frame-exchange`** | 跨线程有界最新帧交换，不拥有 UI 设备或提交权 | [GPU 与帧](/learn/gpu-frame) | 入：`gpu`（经 `__framework`）；出：`nana-ui`（`FrameBinding`） | `FrameExchange`（`lib.rs:493`）、`FrameLease`（`:370`） | `rg "copy_texture_to_texture" crates/nana-frame-exchange/src/lib.rs` |

## 3. 窗口与平台 Crate（2 个）

| Crate | 一句话职责 | 归属主章 | 依赖方向 | 关键入口与源码锚点 | 只读核对命令 |
| --- | --- | --- | --- | --- | --- |
| **`nana-ui-platform`** | 平台中立合同：窗口描述符/事件/命令、输入再导出、`fetch`/WebSocket/剪贴板/路径 | [窗口与平台](/learn/window-platform) | 入：`core`、`input`、`diagnostics`、`ureq`、`url`、`arboard`；出：各宿主层 | `WindowDescriptor`（`window.rs:429`）、`WindowEvent`（`:114`）、`WindowCommand`（`:684`） | `cargo tree -p nana-ui-platform --depth 1` |
| **`nana-window`** | 原生窗口执行：句柄、系统材质、无框 chrome、缩放、全屏、Underlay、无头 `WebSurface` | [窗口与平台](/learn/window-platform) | 入：`nana-ui-core`、`window-vibrancy`、`url`；出：`nana-ui` | `apply_system_material`（`material.rs:176`）、`prepare_client_chrome`（`chrome.rs:32`） | `rg "pub fn apply_system_material" crates/nana-window/src/material.rs` |

## 4. 作者工具链 Crate（4 个）

| Crate | 一句话职责 | 归属主章 | 依赖方向 | 关键入口与源码锚点 | 只读核对命令 |
| --- | --- | --- | --- | --- | --- |
| **`nana-ui-view-schema`** | 内置控件唯一描述表 `for_each_control!` | [作者工具链](/learn/toolchain) | 入：零依赖；出：`runtime` 与 `view-codegen` 同时消费 | `for_each_control!`（`lib.rs:49`） | 阅读 `crates/nana-ui-view-schema/src/lib.rs` |
| **`nana-ui-view-macros`** | `view!` / `css!` / `stylesheet!` 过程宏入口 | [作者工具链](/learn/toolchain) | 入：`view-codegen`；出：`runtime`（`view-macro` feature） | `view!`（`lib.rs:16`）、`css!`（`:41`）、`stylesheet!`（`:76`） | `rg "^pub fn" crates/nana-ui-view-macros/src/lib.rs` |
| **`nana-ui-view-codegen`** | 模板展开为声明式视图函数，并在构建时调用 CSS 引擎编译样式 | [作者工具链](/learn/toolchain) | 入：`core`、`css`、`view-schema` | `expand`（`lib.rs:287`）、`compile_styles`（`style.rs:986`） | `rg "use nana_ui_css" crates/nana-ui-view-codegen/src/style.rs` |
| **`nana-ui-sfc`** | `.vue` 单文件组件（Rust 脚本）构建期编译器 | [作者工具链](/learn/toolchain) | 入：`view-codegen`（生成代码通过路径指向 `runtime`） | `Compiler`（`lib.rs:115`）、`component()`（`lib.rs:413`） | `rg "pub struct Compiler" crates/nana-ui-sfc/src/lib.rs` |

## 5. 外围支撑 Crate（9 个）

| Crate | 一句话职责 | 归属主章 | 依赖方向 | 关键入口与源码锚点 | 只读核对命令 |
| --- | --- | --- | --- | --- | --- |
| **`nana-ui-charts`** | 图表模型：`ChartOption`、刻度、布局、命中与 GPU 标记数组 | [外围支撑](/learn/supporting-crates) | 入：仅 `core` + `bytemuck`；出：`runtime`、`scene`、`nana-ui` | `ChartOption`（`option.rs:105`）、`ChartMarks`（`marks.rs:203`） | 查看 `crates/nana-ui-charts/Cargo.toml` |
| **`nana-diagnostics`** | `event!` / `metric!` / `fault!` / `span!` 宏与静态描述符 | [外围支撑](/learn/supporting-crates) | 入：零普通依赖；出：`core` 起整条产品线 | `event!`（`lib.rs:83`）、`framework.rs` 描述符表 | `rg "never renumber" crates/nana-diagnostics/src/framework.rs` |
| **`nana-svg-raster`** | CPU 侧 SVG 栅格化（`resvg → pixmap`） | [外围支撑](/learn/supporting-crates) | 入：`resvg 0.48.1`；出：`nana-ui` | `rasterize_document`（`lib.rs:58`） | `rg "pub fn rasterize" crates/nana-svg-raster/src/lib.rs` |
| **`nana-app-icon`** | 默认应用图标、PNG/ICO/ICNS 编码与 Windows PE 资源嵌入 | [外围支撑](/learn/supporting-crates) | 入：`platform`（optional）、`image`、`flate2`、`embed-resource` | `rasterize`（`mark.rs:8`） | `rg "feature" crates/nana-app-icon/Cargo.toml` |
| **`nana-icons-tabler`** | 自动生成的 Tabler 矢量图标常量表（约 32.3 万行） | [外围支撑](/learn/supporting-crates) | 入：`core` 的图标类型；非 workspace member，由链接器按引用裁剪 | `*_ICON: IconData` 常量族 | 查看 `crates/nana-icons-tabler/src/lib.rs` 头部注释 |
| **`nana-package`** | 打包契约运行时只读侧（解析 `package.json` 与 `.nrpack`） | [外围支撑](/learn/supporting-crates) | 入：纯 Rust（`blake3`、`ruzstd` 等）；出：`nana-ui` | `PackReader`（`pack/reader.rs:47`）、`ResourceClass`（`pack/mod.rs:30`） | 查看 `crates/nana-package/src/lib.rs` |
| **`nana-packager`** | 打包构建侧 CLI（6 个子命令） | [外围支撑](/learn/supporting-crates) | 入：`nana-package`、`nana-app-icon`、C `zstd` 编码器；不随应用分发 | `Command` 枚举（`main.rs:32-38`） | `rg "enum Command" crates/nana-packager/src/main.rs` |
| **`nana-ui-dev`** | 开发期热重载公共底座（release/dist 构建直接 `compile_error!`） | [外围支撑](/learn/supporting-crates) | 入：`ui`、`core`、可选 `devtools` / `sfc` | `ReloadRequest`（`lib.rs:46`）、编译期门禁（`lib.rs:22`） | `rg "compile_error" crates/nana-ui-dev/src/lib.rs` |
| **`nana-ui-devtools`** | 开发期无头 Agent 自动化会话与离屏快照 | [外围支撑](/learn/supporting-crates) | 入：`core`、`platform`、`ui`、`scene`、`gpu` | `AgentSession`（`agent/session.rs:37`）、`RuntimeAgentSession`（`agent/runtime.rs:26`） | `rg "product-facing UI" crates/nana-ui-devtools/src/lib.rs` |

## 覆盖统计

上表五个分组合计 `5 + 4 + 2 + 4 + 9 = 24`，100% 覆盖 `crates/` 下的全部 24 个 crate。

## 延伸阅读

- [模块与分层](/learn/modules-and-layers)：24 个 crate 的依赖深度层级总表与 55 条普通依赖边清单
- [证据清单](/learn/evidence-checklist)：关键架构断言的只读核对命令与实跑读数
- [架构学习总览](/learn/)：按学习路径组织的全部章节地图
