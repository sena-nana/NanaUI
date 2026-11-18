# 布局模型、Feature 默认值与运行时迁移指南

本文整理了基准架构收敛之后在布局模型、Cargo feature 默认集合、编译期 CSS 迁移告警、运行时特性降级诊断以及 GPU / 平台能力边界方面的变化与迁移方法。

## 1. 纯 `Flex + Grid + Inline + Layers`（及 `Overlay`）布局模型

核心布局模型已移除历史 `Block` / `FlowRoot` / `InlineBlock` / `Contents` 与 `float` / `clear` 分支以及 `baseline-source`：

| 旧写法 | 迁移方向 | 说明 |
| --- | --- | --- |
| `display: block` / `DisplaySpec::Block` / `Block` 控件 | 省略 `display` 或显式 `display: flex; flex-direction: column`（Rust 用 `Stack::column(...)`） | 未显式写 `display` 且非横排的容器默认沿块轴排列且按 `stretch` 拉宽常规子项；显式 `flex` / `grid` / `layers` 严格按各自 `align-items` 排布 |
| `display: flow-root` | `display: flex; flex-direction: column`（或 `Stack::column(...)`） | 不再存在独立 `FlowRoot` 上下文 |
| `display: inline-block` / `DisplaySpec::InlineBlock` | `display: inline-flex` 或 `display: inline-grid` | 参与行内格式化上下文（IFC）的原子容器；支持基线对齐与收缩包裹 |
| `display: contents` / `DisplaySpec::Contents` | 去掉多余包装节点或把布局样式移到子节点 | 保留树中不再保留无盒包装穿透语义 |
| `float` / `clear` | `display: flex` 或 `display: grid` | 浮动已从核心布局移除；声明会被忽略 |
| `baseline-source` | 遵循规范首基线对齐 | 移除实验性属性 |

### 原子基线与 `subgrid` 对齐

- **`inline-flex` / `inline-grid` 原子基线**：遵循 CSS Flexbox §8.5 与 CSS Grid §11.8：按首个行内/弹性/网格首行基线参与对齐；`overflow: hidden` 等非 `visible` 裁剪不再抑制 `inline-flex` / `inline-grid` 的基线导出；当子项中没有行盒时，按首个子项的边框底边合成基线（空容器退回底外边距边）。
- **整值 `subgrid`**：`grid-template-columns: subgrid [line-name...]` 与 `grid-template-rows: subgrid [line-name...]` 按父网格跨度继承已解析轨道与间隙，同时计入 subgrid 自身的 `border` / `padding` / `margin` 对首尾轨的缩进调整以及子网格自定义 `gap` 的差值补偿。

### 构建期 CSS 迁移告警

在 `.vue`、`view!`、`css!` 与 `stylesheet!` 中使用已移除语法时，`nana-ui-view-codegen` 会在编译期直接给出英文迁移提示并忽略该声明：

- `legacy block display (\`block\` / \`flow-root\`) was removed; use \`display: flex; flex-direction: column\` (or \`grid\` / \`layers\`) instead`
- `\`inline-block\` was removed; use \`display: inline-flex\` or \`display: inline-grid\` instead`
- `\`display: contents\` is not supported in NanaUI's retained Style Model; remove the wrapper or move layout styles onto its children`
- `CSS floats (\`float\` / \`clear\`) were removed; use \`display: flex\` or \`display: grid\` instead`
- `mixing \`subgrid\` with explicit tracks is not supported in ...; use standalone \`subgrid\` (with optional \`[line-name]\` groups)`

更多细节见 [布局（CSS 子集）](../reference/layout.md) 与 [布局与样式（Rust）](../reference/rust-layout.md)。

## 2. Cargo Feature 默认值收紧（按需开启）

为减小最小二进制体积并避免隐式拉起跨crate默认特性，`nana-ui`、`nana-ui-runtime` 与 `nana-ui-scene` 的 `default = []`，`nana-ui-platform` 的 `default = ["clipboard"]`。应用侧按职责显式启用：

| Feature | 所属 crate | 默认值 | 启用后能力 | 关闭时行为与诊断 |
| --- | --- | --- | --- | --- |
| `hosted` / `gpu` | `nana-ui` | 关 | 桌面宿主 / `SceneWgpuPainter`，仅编入平台主后端（`metal`/`dx12`/`vulkan`） | 无窗口宿主或 GPU 绘制；非苹果主机默认读取压缩 Naga IR，macOS 14+ / iOS 17+ 默认读取预编译 `stages.msl.zz` |
| `wgpu-backends` | `nana-ui` | 关（含于 `full`） | 启用全部跨平台标准 WGPU 后端（`dx12`/`vulkan`/`gles`/`metal`） | 仅保留平台主后端 |
| `runtime-wgsl` | `nana-ui` | 关（含于 `full`） | 启用运行时 WGSL 编译回退（`naga` / `wgpu` WGSL） | 仅走预编译管线；内置阶段缺失或被拒绝时返回显式构建错误并发出 `gpu.shader.build_failed`，自定义 WGSL `MaterialProgram` 会被拒绝并发出 `gpu.material.rejected` |
| `icu` | `nana-ui` / `runtime` / `scene` | 关（含于 `full`） | ICU4X 本地化数字、日期与复数格式 | 使用 `PlainFormatter`，并在首次请求本地化格式化时发出 `resource.icu_unavailable` |
| `snippet-transforms` | `nana-ui` / `runtime` | 关（含于 `full`） | Snippet 正则替换变换（`fancy-regex`） | 占位符联动照常执行，变换原样返回，首次命中并发出一次性 `runtime.snippet.transform_unavailable` |
| `http-fetch` | `nana-ui`（转发 `platform/fetch`） | 关（含于 `full`） | 内置基于 `ureq` 的 `http(s)://` 资源与图片拉取 | 未提供自定义 `FetchHost` 时跳过远程请求并发出 `resource.fetch_unavailable` |
| `image-codecs-extra` | `nana-ui` | 关（含于 `full`） | 开启 `image` 的 JPEG / GIF / WebP 解码 | 仅保留 PNG 等基础格式，遇其余格式时发出 `resource.image_codec_unavailable` |
| `svg` | `nana-ui` | 关（含于 `full`） | CPU 端 SVG 图片光栅化（`nana-svg-raster`） | SVG 图片按不可解码处理并发出 `resource.image_codec_unavailable`；内置矢量图标不依赖它 |
| `vector-boolean` | `nana-ui` / `scene` | 关（`components`/`rich-text`/`full` 开启） | `custom_paint` 中基于 `i_overlay` 的路径布尔化简 | 使用内置整数平面扫描线布尔回退求解器（支持自交多边形、`EvenOdd`/`NonZero` 填充、嵌套孔洞与孤岛、裁剪交/差集、描边与 miter 轮廓化简），不链接 `i_overlay` |
| `fetch` | `nana-ui-platform` | 关（`default = ["clipboard"]`） | `NativeFetchHost` | 只编译 `FetchHost` 合同类型 |
| `full` | `nana-ui` | 关 | 桌面全量集合（含上述特性与 `bundled-fonts`、`components`、`syntax-highlighting`、`wgpu-backends` 等） | 适合开箱即用的桌面应用 |

如果应用希望保留以前的“全功能桌面配置”，在 `Cargo.toml` 中直接使用：

```toml
[dependencies]
nana-ui = { path = "../NanaUI/crates/nana-ui", features = ["full", "view-macro"] }
```

最小桌面应用可只打开 `hosted`（及按需加 `wgpu-backends`、`bundled-fonts`、`view-macro` 等）。详见 [应用 API](../reference/application-api.md#cargo-feature)。

## 3. 工作量观测、着色器与跨平台取色边界

- **工作量计数器与诊断指标**：`WorkCounters`（含 `ThemeWorkCounters`、`I18nCounters`、`DynamicLayoutCounters`、`GpuWorkObservation`、`OutputWorkObservation`）与 `HeadlessTurn.work` / `HeadlessRuntime::present(...) -> Option<WorkCounters>` 保持完整观测口径，且不增加 `NodeRecord`、`Hierarchy`、`PublishedLayoutMeta`、`PassInput`、`LayoutStyle` 的每节点常驻字节；`nana-diagnostics` 按追加编号放出 `runtime.style.nodes_resolved`、`runtime.hit_test.nodes_rebuilt`、`runtime.extract.nodes_extracted`、`runtime.extract.nodes_changed`、`layout.measure_nodes` 与 `layout.placement_nodes` 计数器（见 [诊断与应用路径](../reference/diagnostics.md)）。
- **预编译着色器与管线回退**：Apple 平台部署基线为 macOS 14.0+ / iOS 17.0+（`MSL 3.1`），构建 painter 时由 `ArchiveScope` + `ArchiveLease` 按需共享单次 `stages.msl.zz` 解压并在离开作用域后归零常驻字节；内置管线构造提供显式 `try_new*` 返回值并在缺失/拒收时发出 `gpu.shader.precompiled_fallback` 或 `gpu.shader.build_failed`（关闭 `runtime-wgsl` 时自定义材质发出 `gpu.material.rejected`）。见 [实时画面与 GPU 集成](../reference/gpu.md)。
- **屏幕取色（`ColorField` / `ColorSampler`）**：宿主与窗口后端已统一跨平台取消与生命周期安全（macOS 原生 `NSColorSampler`；Linux Wayland 严格要求 XDG Desktop Portal `Screenshot.PickColor` v2+ 且不在 Wayland 下回退到 X11，X11 优先按 visual RGB 掩码解码根窗口像素并按需回退到 colormap；Windows 延迟屏幕快照仅在 `WDA_EXCLUDEFROMCAPTURE` 不可用时采集，并按光标所在显示器动态缩放放大镜 DPI）。宿主不支持取色时调用 `set_eyedropper_supported(false)` 隐藏吸管入口，并经 `window.color_sampler.failed` 记入诊断。

## 4. 已移除的公开 API

在收敛框架边界的过程中，以下历史 API 已被移除或重构，请参考下表迁移：

| 已移除 API | 推荐替代 |
| --- | --- |
| `BuiltinComponents` / `NanaBuiltinComponents` / `NanaBuiltinIdentities` / `ApplicationState::BUILTINS` | 已移除；内置组件在被 `view!`、`.vue` 或 `register_component::<C>()` 引用时由 `ComponentRegistry` 自动按需注册与摇树裁剪 |
| `SemanticSpec` / `SemanticOption` / `PreparedSemanticBinding` / `prepare_semantic_binding` / `ComponentBindKind` | 已移除；自定义组件通过 `ComponentView` 与 `RegisterableComponent`（`app.register_component::<C>()` / `world.register_component::<C>()`）实现并注册 |
| `command_palette_items_from_json` | 应用侧使用 `serde_json` 等自行解析 JSON 后构造 `Vec<CommandPaletteItem>` 传入 `CommandPalette` |
| 基于字符串标签名（tag-name）的内置控件构造 | 使用强类型 Rust 构建器（`Stack`、`Button`、`Widget` 等）、`view!` 或 `.vue` 组件 |

### 无头（Headless）API 变更
- `HeadlessTurn.flushes` 已移除，改用 `HeadlessTurn.work: WorkCounters`。
- `HeadlessRuntime::present()` 现在返回 `Option<WorkCounters>` 而非 `bool`。
- `gpu` 特性不再隐式依赖 `nana-ui-platform/fetch`、`naga` 或 `GLES`。
