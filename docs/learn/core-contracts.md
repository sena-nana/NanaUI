---
title: 核心合同：nana-ui-core
---

# 核心合同：`nana-ui-core`

本章深入整个框架的基石 `nana-ui-core`：它定义了全仓共享的基础合同——Style Model（Tokens / Semantics / Layout 三层）、主题栈、几何原语、Workspace 合同以及 Motion IR——并解释为什么这些定义必须抽到最底层。读完后你能搞懂：一条样式从 CSS 声明或 Rust 构建器变成 `LayoutStyle` 字段时落在哪一层；语义调色板如何同时兼顾深浅主题与 WCAG 对比度；`WorkspaceModel` 如何在不依赖窗口后端的前提下管理区域布局与折叠动画；以及 Motion 为什么只允许一条时间线权威。本章承接 [模块与分层](/learn/modules-and-layers)，是 [CSS 引擎](/learn/css-engine) 与 [文本引擎](/learn/text-engine) 的共同前置。

## 职责与边界

`nana-ui-core` 是对图形与窗口后端中立的数据合同层（`crates/nana-ui-core/src/lib.rs:1-9`）：所有样式路径统一汇入 Style Model（**Tokens + Semantics + Layout**）；构建期编译的 CSS 映射到该模型，Rust API 则是它的原生入口；实际绘制留在 Runtime 与 UiScene。

- **共享模型，而非解析实现**：`core` 只定义「样式数据长什么样」「主题如何编译」「区域如何声明」，绝不解析任何样式表或脚本。文件头明确规定："This crate must **not** depend on CSS parsers, JS engines, WebView, or window backends"（`crates/nana-ui-core/src/lib.rs:8-9`）。
- **零 CSS 引擎边界**：CSS 解析被完整隔离在 `core` 之外，`style_model.rs` 的非目标清单写明 CSS 解析与 CSSOM 不得进入本 crate（`crates/nana-ui-core/src/style_model.rs:43-46`）。`core` 与 CSS 唯一的接触点是 `SemanticColorRole::from_css_token_name`（`crates/nana-ui-core/src/style_model.rs:341`，仅将 `--nana-*` 变量名映射为语义色角色，未知 `#hex` / `rgb()` 直接返回 `None`）及少量单值枚举关键字解析。
- **入口中立**：Rust 构建器与构建期 CSS 编译必须产出同一份样式数据，因此合同层必须比任何一方都更底层；文本、CSS、图表、输入、窗口、GPU 六个第 2 层 crate 全部只依赖 `core` 与外部算法库。
- **诊断走静态描述符**：遵循 `nana-diagnostics` 的静态描述符合同，不引入外部日志 crate——这也是它仅有的三个依赖项之一。

## 依赖与邻居

`crates/nana-ui-core/Cargo.toml` 的依赖表只有三项：`nana-diagnostics`、`serde`、`serde_json`——没有任何 CSS 解析器、窗口库或 GPU 类型：

| 邻居 Crate | 协作关系 |
| --- | --- |
| `nana-text` / `nana-ui-css` / `nana-ui-charts` / `nana-ui-input` / `nana-window` / `nana-gpu` | 六个第 2 层 crate，仅依赖 `core` 与外部库；排版词汇（`LineBreakSpec`、`WritingModeSpec` 等）从 `core` 借用 |
| `nana-ui-runtime` | 在 `core` 之上构建保留树；`NodeStyle` 内含 `LayoutStyle`，主题经 `CompiledTheme` 编译后挂入世界 |
| `nana-ui-css` | 唯一 CSS 引擎，将样式声明映射到 `core` 的 `LayoutStyle`（见下文 [内部结构](#内部结构)） |
| `nana-diagnostics` | 提供 `event!` / `metric!` / `fault!` 静态描述符，`core` 是最底层的消费方之一 |

## 关键类型与数据

| 类型 | 所在文件与行号 | 作用说明 |
| --- | --- | --- |
| `StyleModelRef` | `crates/nana-ui-core/src/style_model.rs:839` | 每节点样式读取句柄：`theme_appearance` / `metrics` / `palette` / `titlebar` / `on_media` / `opacity`（注释强调 "small on purpose"） |
| `SemanticColor` | `crates/nana-ui-core/src/style_model.rs:60` | RGBA、`f32` 分量 `0..=1` 的已解析语义色 |
| `PaintColor` | `crates/nana-ui-core/src/style_model.rs:70` | 作者空间绘制色：`Srgb` / `Oklch` / `Hsv` / `LinearScRgb`，带 `to_srgb` / `to_linear_sc_rgb` 换算 |
| `PaintColorSlots` | `crates/nana-ui-core/src/style_model.rs:165` | `LayoutStyle` 上的颜色槽位：前景、背景、四边边框、轮廓与选区颜色 |
| `SemanticColorMix` | `crates/nana-ui-core/src/style_model.rs:393` | 按万分比基点权重的语义色混合，`resolve()` 在预乘 alpha 空间求值 |
| `SemanticColorRole` | `crates/nana-ui-core/src/style_model.rs:257` | 约 45 个语义色角色（含代码高亮 token 角色 `Function` / `Builtin` / `Type` 等），`from_css_token_name` 位于 `:341` |
| `SemanticPalette` | `crates/nana-ui-core/src/style_model.rs:456` | 语义调色板：`dark()` / `light()` / `high_contrast()` / `for_system_contrast()` |
| `LayoutStyle` | `crates/nana-ui-core/src/box_layout.rs:3292` | Style Model 的盒级布局切片：Flex / Grid / 间距 / 定位 / 变换 / 书写方向等全部字段 |
| `LengthSpec` | `crates/nana-ui-core/src/box_layout.rs` | 长度意图：`Px` / 百分比 / `calc()` 表达式等，在布局阶段对包含块求解 |
| `RegionState` / `WorkspaceLayout` | `crates/nana-ui-core/src/layout.rs:94` / `:386` | Workspace 区域合同：`placement` / `scope` / 尺寸边界 / 窄窗折叠策略 |
| `WorkspaceModel` | `crates/nana-ui-core/src/workspace_model.rs:85` | 区域布局唯一权威：持久区域表 + 拖拽缩放交互 + 视口事实 + 折叠过渡 |
| `MotionTrack` / `MotionGraph` | `crates/nana-ui-core/src/motion/ir.rs` | 绝对时间动画轨道；序列与并行组合统一编译至此 |
| `ThemeDefinition` / `CompiledTheme` / `ThemeRegistry` | `crates/nana-ui-core/src/theme/` | 主题定义 → 编译产物 → 注册与解析 |
| `LogicalRect` / `LogicalPoint` | `crates/nana-ui-core/src/geometry.rs:22` | 逻辑像素几何原语（区别于 Scene 层的物理设备像素） |
| `AccentRamp` | `crates/nana-ui-core/src/theme/tokens.rs:42` | 强调色阶梯：`base` / `strong` / `on_soft` / `text` + `soft` 系 alpha + 显式指定的 `focus` |
| `MotionTokens` / `MotionRole` | `crates/nana-ui-core/src/theme/tokens.rs` | 动效时长与缓动的 token 类别（`DEFAULT_MOTION` 即 `MotionTokens::DEFAULT`） |
| `ContentFit` | `crates/nana-ui-core/src/geometry.rs:44` | CSS `object-fit` 语义：`Fill` / `Contain` / `Cover` / `None` / `ScaleDown` |
| `ControlSemantics` | `crates/nana-ui-core/src/style_model.rs:914` | 组件语义控制点：`size` / `button_kind` / `card_kind` / `status` |

## 内部结构

### 1. Style Model：Tokens + Semantics + Layout

`style_model.rs` 模块头（`crates/nana-ui-core/src/style_model.rs:19-24`）定义了三层归属规则：

| 层级 | 核心内容 | 由谁写入 |
| --- | --- | --- |
| **Tokens** | `ThemeMetrics` / `ThemeAppearance` / `SemanticPalette`（`crates/nana-ui-core/src/theme/`） | 已安装的主题定义；单个组件不得越权私造 token |
| **Semantics** | `crate::semantics`：`ButtonKind`、`ControlSize`、`CardKind` 等组件语义 | 已知语义类名（如 `nana-btn--primary`）归入此层，不新造 token |
| **Layout** | `crate::layout`（Workspace 壳层区域）+ `crate::LayoutStyle`（盒级弹性布局，`box_layout.rs`） | Flex / Grid / `gap` / `padding` / 尺寸声明 |

配套的颜色策略（`crates/nana-ui-core/src/style_model.rs:26-30`）规定：已知 token 名或语义类名映射为 `SemanticColorRole`；未知的 `#hex` / `rgb()` **不得**混入正式 `ThemeTokens`，只能作为受限的节点绘制色留在 `PaintColorSlots` 中或被丢弃。这保证了业务 CSS 中的局部颜色不会反向污染全局主题调色板。

两类 Layout 切片在 `layout.rs` 头部明确区分（`crates/nana-ui-core/src/layout.rs:1-8`）：`layout.rs` 描述应用壳层区域（Workspace region layout），`box_layout.rs` 描述盒级 Flexbox / Grid 意图（`LayoutStyle`）；两者都是绘制管线的输入数据，均不是浏览器 CSSOM。

### 2. 语义色与调色板（`SemanticPalette`）

`SemanticPalette` 提供 `dark()`（`:501`）、`light()`（`:541`）、`high_contrast()`（`:593`）、`high_contrast_light()`（`:635`）与 `for_system_contrast()`（`:678`）构造器；`with_accent_ramp()`（`:697`）在切换强调色时自动重新派生 `accent_soft` 家族，确保底色与前景文字始终协调。取色入口 `get_in(role, OpacityTokens)`（`:760`）内置了完整的回退与系统映射。

两个值得注意的设计细节：

- **预乘混合**：`SemanticColorMix`（`:393`）使用万分比基点权重在预乘 alpha 空间混合两种语义色，`resolve()` 是唯一求值出口。
- **测试守卫的对比度保证**：焦点环面积对比度 $\ge 3:1$（WCAG 2.4.11）、焦点文本对比度 $\ge 4.5:1$（WCAG 1.4.3，深色主题实测 `5.80:1`）。`AccentRamp` 中的 `focus` 字段之所以显式指定而非用固定 alpha 派生，是因为深色底 `#181818` 上 `0.50` alpha 可达 `3.16:1`，而在纯白底色上乘任何 `< 1` 的 alpha 都达不到 `3:1`（`crates/nana-ui-core/src/theme/tokens.rs:42-69`）。

### 3. 主题栈：`ThemeDefinition` → `CompiledTheme` → `ThemeRegistry`

主题系统的核心原则写在 `theme/tokens.rs` 头部（`crates/nana-ui-core/src/theme/tokens.rs:1-10`）：**"the component names a step, the theme owns the value"**——组件只声明枚举档位（如 `RadiusTier::Md`、`MotionRole::HoverColor`），具体数值归已安装的主题所有。查询是对枚举执行编译期安全的 `match`，而非运行期字符串哈希查找（严禁 `theme.get("radius.medium")`）；裸常量（`space::*`、`type_scale::*`、`HAIRLINE`）全部从 `DEFAULT` 结构体推导，`scripts/audit-theme-hardcoding.py` 负责监控组件代码中是否新增了硬编码字面量。

### 4. 几何原语与 Workspace 合同

`geometry.rs` 定义逻辑像素原语与窗口几何常量：`LogicalPoint`（`:22`）、`LogicalRect`（`:102`，构造时将负宽高钳制为 `0`）、`ContentFit`（`:44-56`，对应 CSS `object-fit`，由绘制端通过采样 UV 与裁剪实现，不额外复制 GPU 纹理）、`ImageSampling`（`Resample` / `Mipmap`，`:92-98`）以及 `TITLE_BAR_HEIGHT = 36.0`。

Workspace 合同分为静态描述与交互模型两部分：

- **静态区域描述（`crates/nana-ui-core/src/layout.rs`）**：`RegionId`（`:15-24`）、`RegionRole`、`RegionPlacement`、`RegionScope`、`NarrowBehavior`（`Shrink` / `Collapse` / `Overlay`）与 `RegionState`（`:94`）。
- **交互与过渡模型（`crates/nana-ui-core/src/workspace_model.rs`）**：`WorkspaceModel`（`:82`）在不依赖任何窗口或渲染后端的前提下，统一管理持久化区域尺寸、拖拽缩放交互、视口事实与折叠过渡动画（`RegionTransition`，`:44`，使用 `EaseInOutCubic` 在 `WORKSPACE_REGION_TRANSITION_DURATION = 260 ms` 内按绝对时间求值）。

### 5. `LayoutStyle`：盒级弹性布局意图

`box_layout.rs`（约 6,300 行）承载 Style Model 的盒级布局切片 `LayoutStyle`（`crates/nana-ui-core/src/box_layout.rs:3292`），字段涵盖 `paint_colors`、`direction` / `dir` / `writing_mode` / `text_orientation`、`flex_reverse` / `order` / `flex_wrap`、`display` / `position` / `z_index`、`transform`（绘制级 2D/3D 变换，不改变 Flex/Grid 测量盒）、`gap` / `padding` / `margin`（物理四边 TRBL + 逻辑边）等。书写方向枚举同样定义在此：`DirSpec`（`:48`）重映射逻辑边与 `text-align: start/end`；`WritingModeSpec`（`:65`）对不支持的 `sideways-*` 采用 fail-closed 跳过；`TextOrientationSpec`（`:87`）遵循 UAX #50。

### 6. Motion / Motion IR：唯一时间线权威

`motion/mod.rs` 头部明确声明（`crates/nana-ui-core/src/motion/mod.rs:1-3`）：**"This module is the only Motion timeline authority"**——Runtime 的 `AnimationSpec` 只是时序与播放控制子集，采样统一走 `evaluate_progress`；CSS 编译器必须产出 `MotionTrack` / `MotionGraph`，不得自建平行时钟；`PresentationStore` 是建立在这些轨道之上的瞬态覆盖层，而非第二条时间线。

IR 本体位于 `crates/nana-ui-core/src/motion/ir.rs`：`MotionTrackId`（`:18`，同 ID 重启即原子替换）、`MotionTargetId`（`:32`，Runtime 映射为 `StableNodeId`）、`MotionValue`（`:46`，`Scalar` / `Color` / `Transform` / `Discrete`）。`MotionValue::lerp`（`:93`）刻意不将缓动进度钳回 `0..=1`，以保留弹簧与回弹曲线的有意过冲（overshoot），仅颜色通道做范围钳制。子模块分工清晰：`eval` 提供 CPU 求值唯一入口 `evaluate_track`；`descriptor` 将合成器安全轨道编码进带版本号 `MOTION_DESCRIPTOR_VERSION` 的代际化 `MotionDescriptor` 表；`presentation` 的 `PresentationStore` 承载瞬态呈现覆盖；`gpu` 负责上传存储缓冲打包。

### 7. 为什么这些合同必须抽到最底层

将 Style Model、主题、几何、Workspace 与 Motion 放在第 1 层，源于四条硬工程约束：

1. **多入口产出同一份数据**：Rust 构建器与构建时 codegen 需要产出可逐字段比对的样式，因此必须共享同一个目标结构体 `LayoutStyle`。
2. **菱形依赖的公共底座**：`text`、`css`、`charts`、`input`、`window`、`gpu` 六个第 2 层 crate 各自只依赖 `core`；若任何一个合同留在 `runtime` 中，六个子系统就会被迫反向依赖保留树或各自复制定义。
3. **排版词汇单一来源**：`nana-text` 直接借用 `core` 的 12 个排版枚举（白名单守卫），使 `UiWorld` 到文本引擎的适配退化为零损耗的字段传递。
4. **保持纯数据与廉价克隆**：`core` 零解析器、零窗口、零 GPU，使 Runtime 得以实现「写入样式或切换主题时解析一次，读取时仅克隆 `Arc<LayoutStyle>`」的常数级读取策略。

## 协作与数据流

以一张卡片的样式与动效为例，数据流按以下 4 步穿过本层合同：

1. **入口写入**：Rust 组件 API 或 `nana-ui-css`（构建时 / 运行时）将样式声明写入 `LayoutStyle` 字段与 `ControlSemantics`。
2. **主题解析**：Runtime 安装主题得到 `CompiledTheme`；`StyleModelRef` 按节点暴露当前调色板、度量与外观。
3. **布局与绘制消费**：`RuntimeLayoutEngine` 读取 `LayoutStyle` 与 `WorkspaceModel` 计算几何；绘制层读取 `PaintColorSlots` 与 `ContentFit`；Motion 轨道经 `evaluate_track` 求值，合成器安全轨道写入 `MotionDescriptor` 表，瞬态采样绝不写回基础样式。
4. **文本汇入**：文本节点测量时，将 `LayoutStyle` 中的字号、语言与书写方向转化为 `nana-text` 的 `TextStyle` / `TextConstraints`。

## 代码地图

| 文件路径 | 核心内容 |
| --- | --- |
| `crates/nana-ui-core/src/style_model.rs` | Style Model 总纲：三层归属规则、语义色/角色/调色板、`StyleModelRef` |
| `crates/nana-ui-core/src/theme/` | `tokens.rs`、`definition.rs`、`compiled.rs`、`registry.rs`、`recipe.rs`（组件配方） |
| `crates/nana-ui-core/src/box_layout.rs` | `LayoutStyle` 与盒级弹性布局词汇（约 6,300 行） |
| `crates/nana-ui-core/src/layout.rs` | Workspace 区域合同（与盒级 Flex 的边界写在文件头） |
| `crates/nana-ui-core/src/workspace_model.rs` | 区域布局交互权威与折叠过渡动画 |
| `crates/nana-ui-core/src/geometry.rs` | 逻辑几何原语、`ContentFit`、窗口几何常量 |
| `crates/nana-ui-core/src/motion/` | `ir.rs`（Motion IR）、`easing.rs`、`eval.rs`、`descriptor.rs`、`presentation.rs`、`gpu.rs` |

## 延伸阅读

- [模块与分层](/learn/modules-and-layers)：`nana-ui-core` 在依赖图中的第 1 层位置与第 2 层邻居全景
- [CSS 引擎](/learn/css-engine)：`nana-ui-css` 如何将样式声明映射到本章的 `LayoutStyle`
- [文本引擎](/learn/text-engine)：`nana-text` 从 `core` 借用的 12 个排版词汇清单
- [合同与不变量](/learn/contracts-and-invariants)：「单一色彩权威」「呈现不写回基础样式」等非协商合同
