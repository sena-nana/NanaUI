---
title: CSS 引擎：nana-ui-css
---

# CSS 引擎：`nana-ui-css`

本章深入 `nana-ui-css`：全框架唯一的 CSS 引擎，负责严格遵循 W3C CSS Syntax Level 3 的样式表词法与语法解析、CSS Cascade 5 级联排序、将声明直接降维为强类型 `nana_ui_core::StyleOp` 操作，以及生成零开销的纯 Rust `StylePatch` 补丁代码。读完后你能搞懂：一份 CSS 如何以零外部依赖从源码一路编译为 `StylePatch` 函数指针；`@layer`、`@media`、`@container`、`@supports`、`@keyframes`、`@font-face`、伪元素与祖先交互选择器如何端到端接入运行时；运行时的严格 4 层样式合成模型；以及 `tools/css-parity` 如何用 156 组测试夹具守住引擎与浏览器盒模型的一致性。

## 职责与边界

`crates/nana-ui-css/src/lib.rs` 明确界定了引擎的唯一权威与设计边界：

- **全仓唯一 CSS 引擎，零外部依赖，零线程局部状态**：`crates/nana-ui-css/Cargo.toml` 仅依赖 `nana-ui-core` 一个工作区 crate，不引入任何第三方 crate（无 `serde_json`、无外部 CSS 解析库），也不使用 `thread_local!` 状态。词法、解析、级联、属性降维与 Rust 补丁代码生成全部收口在 `nana-ui-css` 中；`nana-ui-core`、`nana-ui-runtime`、`nana-ui-scene` 与 `nana-ui` 不含任何 CSS 解析代码。
- **直接产出 `StyleOp`，无 JSON 解码与中间 diff**：每一条属性解析器直接接收 `&[ComponentValue]` 并返回 `Result<Vec<StyleOp>, CssDiag>`，直接映射到 `nana_ui_core::StyleOp` 与 `style_patch::set::*`（单一事实来源表 `define_style_ops!`，共 **242** 个变体，与 `LayoutStyle::OVERLAY_LEAVES` 严格 1:1 对应），运行时无需任何字段路径字符串表或逐字段 JSON 解码。
- **绝不测量容器尺寸**：`@container` 仅被降维为轴与半开区间集合（`ContainerQuery`）并规划断点（至多 `MAX_CONTAINER_BREAKPOINTS = 16` 个断点 / 17 个尺寸桶），由 Runtime 在布局阶段按容器实测尺寸选桶。几何测量永远归 Runtime 布局引擎单一权威。
- **不做盒模型求解**：本引擎输出 `StyleOp` / `StylePatch` 意图数据；Flexbox / Grid / Inline / Layers 的实际尺寸求解在 `nana-ui-runtime` 中执行（见 [布局引擎](/learn/runtime-layout)）。
- **分级编译期诊断与防御性边界**：所有块、函数、一元运算与布尔条件嵌套深度受 `MAX_NESTING = 32` 约束，Token 流变量展开受 `MAX_EXPANDED_TOKENS = 16_384` 与循环检测保护；字体注册受 `MAX_FONT_FACE_BYTES`（8 MiB）与 `MAX_REGISTERED_FONT_BYTES`（16 MiB）约束。非法声明只产出结构化诊断，绝不覆盖先前的级联值，也绝不静默吞掉错误。

## 依赖与邻居

`crates/nana-ui-css/Cargo.toml` 的普通依赖**只有 `nana-ui-core`**（零外部依赖），属于标准的第 2 层 crate。框架内唯一的直接消费方是构建期的 `nana-ui-view-codegen`（以及开发热重载路径），另有一致性工具 `tools/css-parity` 直接依赖它：

| 消费方 Crate | 消费时机 | 具体职责 |
| --- | --- | --- |
| `nana-ui-view-codegen` | **应用构建时** | 为 `view!`、`stylesheet!`、`css!` 与 `.vue` 解析并级联样式表，通过 `nana-ui-css` 的详尽代码生成器产出静态 `StylePatch`、`SheetMediaQuery`、`FontFaceRule` 与动效数据；开发热重载模式下产出 `HotStylePatch::from_ops(ops)` |
| `tools/css-parity` | **一致性验证** | 通过 `apply_css_text` / `compile_inline_css` 将 156 组夹具的 CSS 文本映射为 `LayoutStyle`，送入 `RuntimeLayoutEngine::layout_style_tree` 比对几何盒 |

## 关键类型与数据

| 类型 / 入口 | 所在文件 | 作用说明 |
| --- | --- | --- |
| `StyleOp` / `style_patch::set::*` | `crates/nana-ui-core/src/style_patch.rs` | 由单一宏表 `define_style_ops!` 同时生成的 242 个强类型叶子写操作与直接 setter 函数（`STYLE_OP_COUNT == LayoutStyle::OVERLAY_LEAVES`） |
| `StylePatch` / `HotStylePatch` | `crates/nana-ui-core/src/style_patch.rs` | 产品构建的零开销函数指针补丁 `StylePatch(fn(&mut LayoutStyle))` 与开发期热重载纯数据补丁 `HotStylePatch { ops: Vec<StyleOp> }` |
| `MediaQueryList` / `MediaEnv` | `crates/nana-ui-core/src/media_query.rs` | 后端中立的 `@media` 契约，基于强 Kleene 三值逻辑（`Some(true)` / `Some(false)` / `None`）对视口、色彩方案、减少动效、对比度、指针/悬停与分辨率求值 |
| `FontFaceRule` / `FontFaceSource` | `crates/nana-ui-core/src/font_face.rs` | 后端中立的 `@font-face` 描述符（`family`、`sources`、`weight` / `weight_end`、`style`、`base_href`），含 8 MiB 单文件 / 16 MiB 总量字节上界 |
| `Token` / `ComponentValue` / `Span` | `crates/nana-ui-css/src/syntax/{tokenizer,parser,span}.rs` | W3C CSS Syntax Level 3 UTF-8 字符安全词法记号、嵌套有界（`MAX_NESTING = 32`）组件值树与精确字节跨度（支持 1-based 行列映射） |
| `VarScope` / `expand_vars` | `crates/nana-ui-css/src/syntax/vars.rs` | 分层 Token 流自定义属性作用域与 `var(...)` 展开器（支持任意嵌套回退、循环检测与 `MAX_EXPANDED_TOKENS = 16_384` 上界） |
| `ComplexSelector` / `Specificity` | `crates/nana-ui-css/src/syntax/selector.rs` | Selectors Level 4 AST、引号感知属性选择器、`An+B`、独立 AND 组合的 `:is()` / `:where()` / `:not()` 与精确三元组特异性 |
| `CssDiag` / `DiagSeverity` / `DiagCode` | `crates/nana-ui-css/src/syntax/diag.rs` | 带精确 `Span` 的分级诊断：`Error`（编译错误）与 `Warning`（安全可忽略的厂商前缀属性等） |
| `ParsedSheet` / `ParsedRule` / `CascadeKey` | `crates/nana-ui-css/src/cascade.rs` | 完整样式表解析产物（规则、关键帧、`@font-face`、`LayerRegistry`、诊断）与 CSS Cascade 5 全序级联键 |
| `CompileContext` / `CompiledDecls` / `CompiledInline` | `crates/nana-ui-css/src/properties/mod.rs` | 无线程局部状态的显式编译上下文，以及声明块/内联 CSS 编译产出的 `StyleOp`、过渡、动画与元素级 `--*` 自定义属性 |
| `ContainerQuery` / `plan_container_queries` | `crates/nana-ui-css/src/lower.rs` | `@container` 降维产物（容器名、轴与半开区间）与断点桶规划器（`MAX_CONTAINER_BREAKPOINTS = 16`） |
| `emit_style_ops_rust` / `emit_media_query_list_rust` / `emit_font_face_rule_rust` | `crates/nana-ui-css/src/codegen.rs` | 详尽无通配分支的 Rust 源码生成器：将 `StyleOp`、`MediaQueryList` 与 `FontFaceRule` 发射为纯 Rust 赋值与构造代码 |
| `Sheet` / `StyleSite` / `SheetMediaQuery` / `InteractiveStateHandle` | `crates/nana-ui-runtime/src/view/style.rs` | 运行时样式表消费端：按类/伪类/`@media` 掩码缓存共享 `Arc<LayoutStyle>`，驱动 `@container` 响应式规则与祖先交互态信号 |

## 内部结构

### 1. W3C CSS Syntax Level 3 基础层（`syntax/`）

引擎在 `crates/nana-ui-css/src/syntax/` 中内置了严格遵循 W3C 规范、零外部依赖的前端：

1. **词法分析（`tokenizer.rs`）**：UTF-8 字符安全的 CSS Syntax Level 3 词法器，产出 `SpannedToken { token: Token, span: Span }`，覆盖字符串、未闭合注释/字符串、`url()`、哈希、维度、百分比、函数与转义序列；
2. **语法解析（`parser.rs`）**：将记号流组装为 `ComponentValue` 树、`Stylesheet`、`QualifiedRule`、`AtRule`、`Declaration` 与 CSS Nesting Level 1 的 `StyleBlockItem`，所有括号/块/函数/条件嵌套严格受 `MAX_NESTING = 32` 限制；
3. **Token 流自定义属性展开（`vars.rs`）**：`VarScope` 按「文档 `:root` → 祖先匹配规则/内联样式 → 元素自身规则/内联样式」分层存储原始 `Vec<ComponentValue>`，在属性降维前经 `expand_vars` 直接在 Token 流上展开 `var(--name, fallback)`，天然支持多层嵌套回退（`var(--a, var(--b, 12px))`）、环检测（`DiagCode::VarCycle`）与展开 Token 上限（`MAX_EXPANDED_TOKENS = 16_384`）；
4. **选择器与 `@` 规则 AST（`selector.rs`、`at_rule.rs`、`values.rs`）**：解析 Selectors 4 复合/复杂选择器、`:is()` / `:where()` / `:not()`、`An+B`、伪类与伪元素，统一解析长度、`calc()` / `min()` / `max()` / `clamp()`、`ThemeExpr`、颜色（`light-dark()`、`color-mix()`、Oklch/HSL/RGB/语义色）与渐变停点，以及 `@layer`、`@media`、`@container`、`@supports`、`@keyframes`、`@font-face`。

### 2. CSS Cascade 5 级联与元素作用域自定义属性（`cascade.rs`）

`parse_stylesheet`（及 `parse_stylesheet_with_base`）完成 CSS Nesting 脱糖、`@supports` 解析期求值、`@layer` 层序注册与规则收集，产出 `ParsedSheet`。

每条命中声明的优先级由 `CascadeKey` 严格按 **CSS Cascade Level 5** 排序：

```text
普通声明（按 @layer 首次声明升序）
  < 普通未分层声明（unlayered）
  < !important 未分层声明
  < !important 分层声明（按 @layer 声明逆序：先声明的层赢！）
  ── 同层级再按 Specificity（ids, classes/attrs/pseudos, types）比较
  ── 同特异性再按 source_order（后者赢）
```

元素级静态自定义属性（`--*`）在规则中保留为 `CustomPropDecl`（原始 `ComponentValue` 序列）。构建期代码生成器遍历静态模板树时维护分层 `VarScope`（`:root` / `[data-theme]` → 祖先元素匹配的 `--*` 与内联 `style="..."` → 当前元素自身的 `--*` 与内联 `style="..."`），使 `.dark-card { --bg: #111; } .dark-card .title { color: var(--bg); }` 在编译期即可精确解析并继承；未解析的 `--nana-*` 主题令牌则保留为动态 `PaintColor` / `ThemeExpr`，在运行时随当前主题与 `ThemeScope` 实时求值。

### 3. 直接属性降维：`properties/` → `Vec<StyleOp>`

`crates/nana-ui-css/src/properties/` 按领域拆分为 8 个内聚模块：`layout.rs`、`grid.rs`、`paint.rs`、`text.rs`、`theme.rs`、`transform.rs`、`motion.rs`、`pseudo.rs`。

`compile_declarations`（供规则块）与 `compile_inline_css`（供 `css!` 与 `style="..."`）共同执行以下流程：

1. 提取 `--*` 声明为 `CustomPropDecl`，并将 `direction`、`writing-mode`、`text-orientation` 置于其余属性之前应用，确保同一声明块中的逻辑盒边（如 `margin-inline-start`、`padding-block`）按生效的书写模式映射；
2. 对每条声明展开 `var(...)`，并交由对应领域解析器直接产出 `Result<Vec<StyleOp>, CssDiag>`；
3. **原子不污染语义**：若某条声明语法无效或值非法，解析器返回 `Err(CssDiag)`，绝不发射半成品 `StyleOp`，也绝不覆盖此前级联写入的合法值；
4. 伪元素声明（`::placeholder`、`::selection`、`::-webkit-scrollbar*`）走 `compile_pseudo_property`，直接产出 `StyleOp::PlaceholderColor` / `PlaceholderOpacity`、`StyleOp::SelectionBackground` / `SelectionColor` 与合并后的 `StyleOp::Scrollbar`（`merge_scrollbar_skin`）。

### 4. 端到端贯通的 CSS 能力总览

所有受支持的 CSS 构造均已从 `nana-ui-css` → `nana-ui-view-codegen` → `nana-ui-runtime`（及 `nana-ui` 宿主）端到端打通：

| CSS 构造 | `nana-ui-css` 编译产物 | 运行时 / 宿主消费方式 |
| --- | --- | --- |
| `@layer` | `LayerRegistry` + `LayerOrder` | 参与 `CascadeKey` 完整 CSS Cascade 5 排序（含 `!important` 层序反转） |
| `@media` | `lower_media_query_list` → `nana_ui_core::MediaQueryList`（Kleene 三值逻辑） | 发射为 `SheetMediaQuery` 挂入 `Sheet::with_media`；运行时通过 `media_env_signal()` 按 `MediaEnv`（视口宽高、深浅色、减少动效、对比度、悬停/指针精度、分辨率）响应式求值 |
| `@container` | `lower_container_query` → `ContainerQuery`（轴 + 半开区间） | `plan_container_queries` 切分为至多 `16` 个断点（17 个桶），交付 Runtime `ResponsiveRule` 按最近匹配容器的实测尺寸切桶 |
| `@supports` | 解析期调用 `is_supported_declaration` 求值 | 成立分支的规则直接并入当前级联，不成立分支跳过 |
| `@keyframes` / `transition` / `animation` | `KeyframesRule`、`CompiledTransition`、`CompiledAnimation` | 编译为 `nana-ui-core::motion` 的 Motion IR 轨道（`SheetTransition` / `SheetAnimation`），动态主题色关键帧实时随主题重算 |
| `@font-face` | `lower_font_face_rule` → `nana_ui_core::FontFaceRule` | 发射为 `Sheet::with_font_faces` 并挂在根元素 `El::sheet(&SHEET)` 上；宿主通过 `nana_ui::ingest_font_face_rules` / `drain_world_font_faces` 注册进 `nana-text` 的 `FontSystem`（受 8 MiB / 16 MiB 上界约束） |
| 交互伪类（`:hover` / `:focus` / `:focus-visible` / `:active` / `:selected`） | 5 位伪类掩码（高 5 位 `PSEUDO_BIT_SHIFT = 59`，其余 `MAX_CONDITIONAL_CLASSES = 59` 位留给条件类） | 参与 `StyleSite` 共享布局合成，状态变化时只切换缓存的 `Arc<LayoutStyle>` |
| 祖先交互选择器（如 `.card:hover .icon`、`.row:active .label`） | 复杂选择器在模板树上匹配祖先与后代 | 编译期在祖先节点发射 `El::track_interactive_state(handle, mask)`，在后代节点发射 `El::ancestor_pseudo(class, handle, mask)`，零运行时 DOM 选择器遍历 |
| 伪元素（`::placeholder`、`::selection`、`::-webkit-scrollbar*`） | 直接降维为 `StyleOp`（`Placeholder*`、`Selection*`、`Scrollbar`） | 直接写入 `LayoutStyle` 对应字段，由输入框、文本选区与滚动条绘制管线原生消费 |
| 内联 `style="..."` 与 `.css(css!{...})` | `compile_inline_css`（在当前元素的 `VarScope` 下编译） | 发射为 Layer 2 `El::inline_patch(&INLINE_PATCH)`，在 `.vue`、`view!` 与 Rust 函数 API 间保持同一契约 |
| 元素级静态 `--*` 自定义属性 | `CustomPropDecl` + 模板树分层 `VarScope` | 编译期按祖先链继承并展开 `var(...)`；`--nana-*` 语义令牌保持为运行时动态主题值 |

### 5. 详尽代码生成与运行时严格 4 层合成

#### 零开销 `StylePatch` 与开发期 `HotStylePatch`

`crates/nana-ui-css/src/codegen.rs` 的 `emit_style_op_rust` 对 `nana_ui_core::StyleOp` 执行**无通配分支的详尽匹配**（在 `nana-ui-core` 新增任意 `StyleOp` 变体，若未补全代码生成会直接编译失败）：

- **产品构建**：每条编译规则发射为一个零开销函数指针 `StylePatch::new(|__nana_layout| { __patch::set::padding_left(__nana_layout, ...); ... })`，直接调用 `nana_ui_core::style_patch::set::*`。整个运行期**零 CSS 解析、零 JSON 解码、零字符串字段表查找**。
- **开发期热重载**：`nana-ui-dev` 在仅修改 `.vue` 静态 `<style>` 时调用 `nana-ui-css` 生成 `HotStylePatch::from_ops(Vec<StyleOp>)` 注入运行中窗口，无需重编译。

#### `El` / `NodeStyle` 的严格 4 层样式合成

运行时在 `crates/nana-ui-runtime/src/view/style.rs` 与 `node.rs` 中严格执行 4 层覆盖顺序，与链式构建器调用顺序无关：

```text
Layer 0  控件默认布局（例如 Stack::row() 的默认对齐、控件固有布局）
   ↓
Layer 1  样式表规则（.class / .class_when / 交互伪类 / @container / @media / .ancestor_pseudo）
   ↓
Layer 2  作者内联覆盖（.vue 与 view! 的 style="..." -> .inline_patch、.css(css!{...})、静态 Rust 布局/绘制 builder）
   ↓
Layer 3  响应式 Rust 属性绑定（信号与闭包绑定，位于 NodeBindings::composed_style 之后执行）
```

因此：Layer 2 的内联 `style="..."` 或 `.padding(20.0)` 总是压过 Layer 1 的类规则与 `@container` / `@media` 桶；Layer 3 的响应式绑定（如 `.paint_opacity(sig)`、`.visible(sig)`）在每次重算时总是压过前 3 层。

### 6. 作者宏语法、`.vue` 作用域与分级诊断

- **两种宏内 CSS 写法**：`css!`、`stylesheet!` 与 `view!` 的 `<style>` 同时支持**直接写 Rust Token CSS** 以及**原始字符串字面量**（`css!(r#"..."#)`、`stylesheet!(r#"..."#)`、`view! { <style>r#"..."#</style> ... }`）。在 Token 模式下，需要 CSS 字符串字面量的属性（如 `grid-template-areas`、`font-feature-settings`、`font-variation-settings`、`font-family`、`content`，以及 `url(...)` 与属性选择器）会自动保留双引号；其余属性仍可使用 `"1.5em"` / `"#9ecafe"` 作为 Rust 词法转义。
- **`.vue` `<style>` 作用域规则**：`.vue` 单文件组件中的 `<style>` **默认组件作用域**（无论是否书写 `scoped` 属性）；若需显式声明全局样式，使用 `<style global>` 或独立的 `stylesheet!` 宏。
- **分级编译期诊断（`CssDiag`）**：
  - **`DiagSeverity::Error`（编译错误）**：语法错误、未闭合注释/字符串、超过 `MAX_NESTING`、非法属性值、未知属性、不支持的选择器/组合器或 `@` 规则、自定义属性循环引用（`VarCycle`）或展开超限（`VarDepthExceeded`）——在过程宏中产出 `compile_error!`，在 SFC 构建期返回 `Err`；
  - **`DiagSeverity::Warning`（编译警告）**：已知可安全忽略的厂商前缀属性（如 `-moz-*`、`-ms-*`、`-o-*` 及未映射的 `-webkit-*`，注：已原生支持的 `-webkit-backdrop-filter`、`-webkit-mask-image`、`-webkit-line-clamp`、`-webkit-text-stroke*`、`-webkit-text-emphasis*` 正常编译不警告）或模板中未命中的类等可恢复提示。

## 与 `tools/css-parity` 的一致性验证

工作区内的 `tools/css-parity`（包名 `nana-css-parity`）使用 **156 组标准 JSON 测试夹具**（覆盖盒模型 `T-B`、Flexbox `T-F` / `T-FX`、Grid `T-G` / `T-GB`、内联 `T-I`、布局 `T-L`、定位 `T-P`、尺寸与 `calc` `T-S`、视口 `T-V`、书写模式与折行 `T-W` / `T-WM`）端到端锁定「CSS 声明 → `StyleOp` → `LayoutStyle` → `RuntimeLayoutEngine` → 几何盒」的准确性：

```bash
cargo run -p nana-css-parity -- compare
```

每个夹具给出一棵带样式的节点树、视口尺寸与期望的盒子坐标（默认容差 `2.0 px`），通过 `tools/css-parity/src/measure.rs` 调用与生产环境完全相同的 `RuntimeLayoutEngine::layout_style_tree` 进行比对。

## 代码地图

| 文件路径 | 核心内容 |
| --- | --- |
| `crates/nana-ui-core/src/style_patch.rs` | `define_style_ops!` 单一事实来源表：242 个 `StyleOp`、`set::*`、`StylePatch`、`HotStylePatch` |
| `crates/nana-ui-core/src/media_query.rs` | `MediaQueryList`、`MediaCondition`、`MediaFeature`、`MediaEnv` 与 Kleene 三值逻辑 |
| `crates/nana-ui-core/src/font_face.rs` | `FontFaceRule`、`FontFaceSource`、`FontStyleSpec` 与 8 MiB / 16 MiB 字节上界 |
| `crates/nana-ui-css/src/syntax/` | W3C CSS Syntax Level 3 词法器、嵌套有界解析器、Token 流 `VarScope`、Selectors 4、`at_rule`、`values`、`span` 与分级 `diag` |
| `crates/nana-ui-css/src/cascade.rs` | 样式表解析、CSS Nesting 脱糖、CSS Cascade 5 `CascadeKey`、`@layer` / `@media` / `@container` / `@supports` / `@font-face` / `--*` 提取 |
| `crates/nana-ui-css/src/properties/` | 8 个领域属性编译器（`layout` / `grid` / `paint` / `text` / `theme` / `transform` / `motion` / `pseudo`），直接产出 `Vec<StyleOp>` |
| `crates/nana-ui-css/src/lower.rs` | `@media` → `MediaQueryList`、`@font-face` → `FontFaceRule`、`@container` → `ContainerQuery` 与 `plan_container_queries` |
| `crates/nana-ui-css/src/codegen.rs` | 详尽无通配的 Rust 代码生成器（`emit_style_ops_rust`、`emit_media_query_list_rust`、`emit_font_face_rule_rust`） |
| `crates/nana-ui-view-codegen/src/style.rs` | 构建期消费者：匹配模板树、维护分层 `VarScope`、生成 `Sheet` / `StylePatch` / 祖先交互追踪与分级诊断 |
| `crates/nana-ui-runtime/src/view/style.rs` | 运行时 4 层样式合成、`StyleSite` 缓存、`SheetMediaQuery`、`InteractiveStateHandle` 与 `__patch` 重导出 |
| `tools/css-parity/` | 156 组 CSS 盒模型一致性夹具与比对 CLI |

## 延伸阅读

- [核心合同](/learn/core-contracts)：`LayoutStyle` 与 Style Model 三层定义（属性映射的目的地）
- [布局引擎](/learn/runtime-layout)：拿到 `LayoutStyle` 后 Flexbox / Grid / Inline / Layers 如何求解几何
- [作者入口](/learn/authoring-paths) · [作者工具链](/learn/toolchain)：`view!`、`stylesheet!`、`css!` 与 `.vue` 的样式如何在构建期编译为 Style Model 数据
