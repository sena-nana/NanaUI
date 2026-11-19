---
title: CSS 引擎：nana-ui-css
---

# CSS 引擎：`nana-ui-css`

本章深入 `nana-ui-css`：全框架唯一的 CSS 引擎，负责样式表解析、选择器匹配、级联排序，以及将命中的声明映射到 Style Model（`nana_ui_core::LayoutStyle`）。读完后你能搞懂：一份 CSS 从源码字符串到 `LayoutStyle` 字段的完整内部流水线（手写词法/切分 → 选择器匹配 → 级联排序 → 字段映射）；`@media`、`@import`、`@layer`、`@font-face`、`@keyframes`、`@container` 各自被编译成什么数据结构；为什么这个引擎只在 `view-codegen` 构建期运行；以及 `tools/css-parity` 如何用 147 组测试夹具守住它与浏览器盒模型的一致性。

## 职责与边界

`lib.rs` 文件头一句话定调："NanaUI's CSS subset, in one place"（`crates/nana-ui-css/src/lib.rs:1`）。样式表是受支持子集内的标准 CSS——选择器、级联、自定义属性（CSS 变量）、`@media`、`@import`、`@font-face`、`@keyframes` 与交互伪类——全部映射到唯一的 Style Model（`crates/nana-ui-css/src/lib.rs:3-6`）：

- **全仓唯一 CSS 引擎**：词法、解析、级联与映射只在这一个 crate 内实现；"CSS parsing stays out of `nana-ui-core`, `nana-ui-runtime`, `nana-ui-scene` and `nana-ui`"（`crates/nana-ui-css/src/lib.rs:13-14`）。
- **绝不测量容器尺寸**：`@container` 被编译为区间与分桶数据交由 Runtime 在布局阶段求值，"this crate never measures a container"（`crates/nana-ui-css/src/lib.rs:6-8`；`crates/nana-ui-css/src/css_container.rs:3`）。几何测量永远归 Runtime 布局引擎单一权威。
- **不做盒模型求解**：本引擎的输出是 `LayoutStyle` 意图数据；Flexbox / Grid 的实际尺寸求解在 `nana-ui-runtime` 中执行（见 [布局引擎](/learn/runtime-layout)）。
- **显式子集与可审计报告**：受支持的 CSS 子集以 [布局（CSS 子集）](/reference/layout) 为准；超出子集的选择器或声明绝不胡乱猜测，统一记入 `UnsupportedCssReport` 与 `UnsupportedCssTally`。
- **防御性输入上界**：样式表体积、字体大小与 `@import` 嵌套深度均设有硬上界（`MAX_STYLESHEET_BYTES`、`MAX_FONT_FACE_BYTES`、`MAX_REGISTERED_FONT_BYTES`、`MAX_IMPORT_DEPTH`）并配合 `is_blocked_href` 检查（`crates/nana-ui-css/src/lib.rs:34-39`）。

## 依赖与邻居

`crates/nana-ui-css/Cargo.toml` 的普通依赖只有 `nana-ui-core` 与 `serde_json`（`:14-15`），属于标准的第 2 层 crate。框架内唯一的直接消费方是 `nana-ui-view-codegen`，另有一致性工具 `tools/css-parity` 直接依赖它：

| 消费方 Crate | 消费时机 | 具体职责 |
| --- | --- | --- |
| `nana-ui-view-codegen`（`crates/nana-ui-view-codegen/Cargo.toml:11`） | **应用构建时** | 为 `view!`、`stylesheet!`、`css!` 与 `.vue` 解析并匹配类规则，编译为静态 `StylePatch` 数据，使 Rust L3 视图运行时零 CSS 解析开销 |
| `tools/css-parity`（`tools/css-parity/Cargo.toml:17`） | **一致性验证** | 将夹具中的 CSS 文本映射为 `LayoutStyle`，送入 Runtime 布局引擎比对几何盒 |

## 关键类型与数据

| 类型 | 所在文件与行号 | 作用说明 |
| --- | --- | --- |
| `StyleRule` | `crates/nana-ui-css/src/css_cascade.rs:103` | 单条样式规则：选择器列表 + 声明条目 + `source_order` + `layer` |
| `Selector` / `CompoundSelector` | `crates/nana-ui-css/src/css_cascade.rs:114` / `:150` | 复合选择器链与单个复合段（含组合器 `Combinator`，`:125`） |
| `Specificity` | `crates/nana-ui-css/src/css_cascade.rs:262` | 标准三元组 `(ids, classes_and_attrs, types)`（各为 `u16`） |
| `MediaEnv` | `crates/nana-ui-css/src/css_cascade.rs:424` | 媒体查询求值环境（视口宽高、深浅色、对比度等），供 `@media` / `@import` 求值 |
| `MatchContext` / `MatchNode` | `crates/nana-ui-css/src/css_cascade.rs:291` / `:278` | 选择器匹配现场：祖先链、前驱兄弟、`:has()` 位图、焦点与媒体环境 |
| `DeclarationEntry` | `crates/nana-ui-css/src/css_cascade.rs:86` | 单条属性声明：属性名、原始值、`!important` 标志 |
| `StylesheetParseReport` / `UnsupportedCssTally` | `crates/nana-ui-css/src/css_cascade.rs:481` / `:563` | 单次解析诊断报告与子集外构造的累计统计 |
| `RuleIndex` | `crates/nana-ui-css/src/css_cascade.rs:1298` | 按最右端复合选择器的 `type` / `id` / `class` 建立的候选规则索引 |
| `ParsedStylesheet` | `crates/nana-ui-css/src/css_interactive.rs` | 解析结果分桶：静态规则、交互伪类规则、生成伪元素、动效规则、`@container` |
| `KeyframesRule` / `MotionStyleRule` | `crates/nana-ui-css/src/css_interactive.rs` | `@keyframes` 百分比停点与过渡/动画声明，待编译为 Motion IR 轨道 |
| `ContainerRule` / `ContainerQuery` | `crates/nana-ui-css/src/css_container.rs` | `@container` 编译产物：容器名、查询轴与半开区间集合 |
| `InteractiveStyleRule` | `crates/nana-ui-css/src/css_interactive.rs` | `:hover` / `:focus` / `:active` 规则桶（静态级联阶段不应用） |
| `FontFaceRule` / `FontFaceSrc` | `crates/nana-ui-css/src/css_at_rule.rs` | `@font-face` 的源与描述符（由 `StylesheetLoader` 加载） |
| `LayoutStyleCss` | `crates/nana-ui-css/src/css_map.rs:1862` | 属性映射入口 trait：`apply_css_property`、`apply_css_text`、`apply_class_layout_hints` |
| `CssPaintColor` | `crates/nana-ui-css/src/style.rs:30` | CSS 颜色语法解析器，通过 `to_core()`（`:51`）转为 `nana_ui_core::PaintColor` |

## 内部结构

### 1. 词法与解析：手写零外部依赖解析器

引擎不依赖外部重型 CSS 解析器，一套样式表依次经过以下四站：

1. `strip_css_comments`（`css_cascade.rs:3037`）剥离注释；
2. `split_rule`（`:3091`，在首个 `{` 处切分选择器头与声明块）、`split_selector_list`（`:3132`，按顶层逗号切分选择器列表）、`skip_at_rule`（`:3061`）识别并分发 `@` 规则；
3. 选择器进入 `parse_selector`（`:2024`）→ `tokenize_selector_chain`（`:2339`）→ `parse_compound`（`:2433`）；声明块进入 `parse_declaration_entries`（`:1422`）切分为 `DeclarationEntry`；
4. 解析产物按 `ParsedStylesheet` 分入静态规则、交互伪类、生成伪元素（`::before` / `::after`）、动效与 `@container` 五个桶。

解析入口按能力递增：`parse_stylesheet`（`:658`）→ `parse_stylesheet_full`（`:663`）→ `_with_layers`（`:671`）→ `_with_options`（`:680`），底层 `parse_stylesheet_full_with_cache`（`:701`）自带解析结果缓存。

### 2. 选择器模型与自右向左匹配

选择器匹配由 `MatchContext` 驱动（`crates/nana-ui-css/src/css_cascade.rs:291-344`），携带当前节点的 tag、id、classes、attributes、祖先链（直接父节点在前）、前驱兄弟列表（紧邻兄长在前）、预计算的 `:has()` 位图（`has_bits: u64`）、`focus_within`、`is_empty`、`checked` 与 `MediaEnv`。

`selector_matches`（`:1957`）自右向左回溯匹配：`Child`（`>`）严格核对直接父节点，`Descendant`（空格）向上扫描祖先链，`AdjacentSibling`（`+`）与 `SubsequentSibling`（`~`）扫描前驱兄弟列表。候选集先经 `RuleIndex`（`:1298`）按最右端选择器的 id / class / tag 过滤出严格超集，再对候选规则执行 `selector_matches`，使单元素匹配代价为 $O(\text{候选数})$ 而非 $O(\text{全表规则数})$。`:nth-child(An+B)`（`parse_an_plus_b:2980`）、属性选择器（`AttrSelector:223`）以及 `:is()` / `:where()` 列表（特异性取分支最大值 `max_alts_specificity:2918`）均在此层完成判定。

### 3. 级联排序与自定义属性（`var()`）

`matched_declaration_entries_from`（`:1253`）按标准级联键 `(important, cascade_layer_key, specificity, source_order, decl_index)` 对命中声明排序。重建单个元素样式的完整流水线位于 `rebuild_layout_style`（`:1606`）与索引版 `rebuild_layout_style_indexed`（`:1647`）：

```text
样式表普通层 → 工具类提示（shell_contract）→ 属性样式 → 内联 style
  → 样式表 !important 层（含 @container 命中的 !important）
  → 属性/内联 !important 层 → 逻辑盒边（margin/padding-inline/block）最终解析
```

注意：**书写方向（`direction` / `dir`）与书写模式（`writing-mode`）始终先于其余声明应用**（`:1701-1712`），因为它们决定了后续 `margin-inline-start` 等逻辑属性应当映射到哪条物理边。自定义属性（`--*`）通过 `collect_document_custom_properties_from_rules`（`:1458`）与 `matched_custom_properties_indexed`（`:1520`）按 `:root` / `[data-theme]` 及元素继承链收集，并在属性映射前通过 `with_active_css_vars` 展开 `var(...)`。

### 4. 属性映射表：`LayoutStyleCss`（`css_map.rs`）

`crates/nana-ui-css/src/css_map.rs`（约 9,100 行）将 CSS 属性键值对写入 `nana_ui_core::LayoutStyle`（通过 `LayoutStyleCss` trait，`:1862`）。颜色解析走 `CssPaintColor`（`style.rs:30`），保留 `Oklch` / `Hsv` / `Srgb` 精度并通过 `map_css_color_for_tokens`（`:635`）执行语义色令牌策略。宿主工具类（如 `gap-sm`、`p-*`、`w-full`、`flex-1`、`rounded*`）集中隔离在 `crates/nana-ui-css/src/shell_contract.rs` 中，与标准 CSS 语法解析保持清晰分界。工具类与 Tailwind 同义：只有 `flex` 设 `display: flex`（样式表已写 `display` 时不覆盖）；`flex-row` / `flex-col`、`items-*`、`justify-*` 各只设自己那一项。所以 `flex-col items-center` 不写 `flex` 时仍是默认纵向容器（未开启 flex 布局，`align-items` 不生效，子项不会居中），要写 `flex flex-col items-center`。

### 5. 各 `@` 规则与伪类的编译去向

| CSS 构造 | 编译产物 | 消费去向 |
| --- | --- | --- |
| `@import` | 就地展开的规则（保留深度、`@layer` 与媒体条件） | `css_cascade.rs:858`（`apply_import`） |
| `@layer` | 层级序号（按首次声明顺序编号） | `css_cascade.rs:945`（`record_layer_names`） |
| `@media` / `@supports` | `MediaQueryList` + `MediaEnv` / `evaluate_supports_condition` | `css_at_rule.rs` |
| `@font-face` | `FontFaceRule`（`src` / `weight` / `style` / `stretch` 描述符） | 宿主适配器加载字节并注册进 `nana-text` 的 `FontSystem` |
| `@keyframes` / `transition` | `KeyframesRule` / `TransitionShorthand` | 编译为 `nana-ui-core::motion` 的 Motion IR 轨道 |
| `@container` | `ContainerQuery`（容器名、查询轴与半开区间集合） | `plan_container_queries` 切分为至多 `17` 个尺寸桶，Runtime 按容器实测尺寸选桶 |
| `:hover` / `:focus` / `:active` | `InteractiveStyleRule`（`:focus-visible` 映射为 `Focus`） | 静态级联不应用，交由宿主交互状态层按需叠加 |

### 6. 消费时机：只在构建时

- **构建时（`crates/nana-ui-view-codegen/src/style.rs:239`）**：调用 `parse_stylesheet_full` 解析 `<style>`、`stylesheet!` 或 `css!`，在空 `LayoutStyle` 上应用声明并 diff 出被修改的字段，生成携带级联优先级 `Rank` 的静态 `StylePatch`（`@container` 规则同步附带桶索引）。因此 Rust L3 视图在运行期直接按位组合补丁，**零 CSS 解析开销**。补丁是生成的赋值代码：每个被写的字段一条（`layout.paint.outline.width = …`），值是该字段自己的 JSON，按字段类型读出。应用只链接它的样式写到的那些字段类型的解码。整个 `LayoutStyle` 不经过序列化：补丁、以及 `NodeStyle` 各层之间的叠加（`LayoutStyle::overlaid`）都是逐字段写。

## 与 `tools/css-parity` 的一致性验证

工作区内的 `tools/css-parity`（包名 `nana-css-parity`）使用 **147 个标准 JSON 测试夹具**（按 Flexbox `T-F`、尺寸与 `calc` `T-S`、盒模型 `T-B`、Grid `T-G`、内联 `T-I`、布局 `T-L`、定位 `T-P` 分族）端到端锁定「CSS 声明 → `LayoutStyle` → `RuntimeLayoutEngine` → 几何盒」的准确性：

```bash
cargo run -p nana-css-parity -- compare
```

每个夹具给出一棵带样式的节点树、视口尺寸与期望的盒子坐标（默认容差 `2.0 px`），通过 `measure::measure_layout`（`tools/css-parity/src/measure.rs:90`）调用与生产环境完全相同的 `RuntimeLayoutEngine::layout_style_tree` 进行比对。

## 代码地图

| 文件路径 | 核心内容 |
| --- | --- |
| `crates/nana-ui-css/src/css_cascade.rs` | 引擎主干：词法切分、选择器解析、匹配、级联排序与 `RuleIndex`（约 5,600 行） |
| `crates/nana-ui-css/src/css_map.rs` | CSS 属性到 `LayoutStyle` 字段的映射与长度/字号/calc 解析（约 9,100 行） |
| `crates/nana-ui-css/src/css_at_rule.rs` | `@media`、`@supports`、`@import`、`@layer`、`@font-face`、`@keyframes` 解析与求值 |
| `crates/nana-ui-css/src/css_container.rs` | `@container` 查询解析、半开区间运算与尺寸桶规划（`plan_container_queries`） |
| `crates/nana-ui-css/src/css_interactive.rs` | 解析期分桶：交互伪类、生成伪元素、关键帧与过渡规则 |
| `crates/nana-ui-css/src/style.rs` | `CssPaintColor` 颜色解析与语义色 token 映射策略 |
| `crates/nana-ui-css/src/css_motion.rs` | `transition` 简写语法解析 |
| `crates/nana-ui-css/src/shell_contract.rs` | 宿主内置工具类映射（`gap-sm` 等） |
| `crates/nana-ui-view-codegen/src/style.rs` | 构建期消费者：将类规则编译为静态 `StylePatch` |
| `tools/css-parity/` | 147 组 CSS 盒模型一致性夹具与比对 CLI |

## 延伸阅读

- [核心合同](/learn/core-contracts)：`LayoutStyle` 与 Style Model 三层定义（属性映射的目的地）
- [布局引擎](/learn/runtime-layout)：拿到 `LayoutStyle` 后 Flexbox / Grid 如何求解几何
- [作者入口](/learn/authoring-paths) · [作者工具链](/learn/toolchain)：`view!` 与 `.vue` 的样式如何在构建期编译为 Style Model 数据
