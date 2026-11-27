---
title: 作者工具链
---

# 作者工具链

开发者写下的 `view!`、`css!`、`stylesheet!` 宏以及 `.vue` 单文件组件，在**应用编译期**就会被编译为 `nana-ui-runtime` 的声明式视图函数调用与静态 Style Model 补丁数据。本章将两条构建期编译管线逐一拆开，看清模板、样式与脚本在每个 crate 中的转换过程。

## 职责与边界

| Crate | 一句话职责 | 明确不做的事 |
| --- | --- | --- |
| `nana-ui-view-schema` | 内置控件的唯一描述表（`for_each_control!` 中间表示） | 零外部依赖，不含任何代码生成逻辑 |
| `nana-ui-view-macros` | `view!` / `css!` / `stylesheet!` 与 `#[derive(Store)]` 过程宏入口 | 只做过程宏转发，核心展开逻辑交给 `view-codegen` |
| `nana-ui-view-codegen` | 将模板元素树展开为 Runtime 视图函数；在构建时调用 `nana-ui-css` 编译样式 | 不引入任何新的运行时概念 |
| `nana-ui-sfc` | `.vue` 单文件组件（`<script setup lang="rust">`）构建期编译器 | 不进入应用运行时；构建期输出标准 Rust 源码与源映射 |

两条编译管线共同遵守一条铁律：**运行时收到的是编译好的数据，而非待解析的 CSS 源码**（`crates/nana-ui-runtime/src/view/style.rs:1-27`）。

## 依赖与邻居

| Crate | 层级 | 工作区内普通依赖（normal） | 边界说明 |
| --- | --- | --- | --- |
| `nana-ui-view-schema` | 0 | — | 零依赖纯宏表，同时喂给 `runtime` 与 `view-codegen` |
| `nana-ui-css` | 2 | `nana-ui-core` | 唯一 CSS 引擎，只在构建期被 `view-codegen` 调用 |
| `nana-ui-view-codegen` | 3 | `nana-ui-core`, `nana-ui-css`, `nana-ui-view-schema` | 在宿主编译期运行，将 CSS 规则编译为静态 `StylePatch` |
| `nana-ui-view-macros` | 4 | `nana-ui-view-codegen` | `proc-macro` crate，经 `nana-ui-runtime`（`view-macro` feature）再导出 |
| `nana-ui-sfc` | 4 | `nana-ui-view-codegen` | 在应用的 `build.rs` 中运行，生成代码通过路径字符串引用 `nana_ui::runtime` |

## 关键类型与数据

| 类型 / 常量 | 所在文件与行号 | 作用说明 |
| --- | --- | --- |
| `for_each_control!` | `crates/nana-ui-view-schema/src/lib.rs:49` | 26 个内置声明式控件的标签、构造函数、字段写入策略与事件表 |
| `Control` / `CONTROLS` | `crates/nana-ui-view-codegen/src/lib.rs:18` / `:53-77` | 由 `for_each_control!` 展开的静态控件元数据表 |
| `Node` / `Element` / `Attr` | `crates/nana-ui-view-codegen/src/lib.rs:333-405` | 模板语法树的中间表示 |
| `CompiledStyles` / `CascadeKey` | `crates/nana-ui-view-codegen/src/style.rs` / `crates/nana-ui-css/src/cascade.rs` | 构建时样式编译产物与 CSS Cascade 5 级联优先级键（`@layer`、`!important`、`Specificity`、`source_order`） |
| `CssDiag` / `DiagSeverity` | `crates/nana-ui-css/src/syntax/diag.rs` | 分级构建期诊断：语法/非法值/不支持的选择器或 `@` 规则报 `Error`（编译错误）；可安全忽略的厂商前缀属性等报 `Warning` |
| `Component` / `Compiler` | `crates/nana-ui-sfc/src/parse.rs:23` / `lib.rs:115` | `.vue` 文件解析出的四大块（name / props / script / template / style）与构建编译器 |

## 内部结构

### 1. 管线一：Rust 过程宏链（`view!` / `css!` / `stylesheet!`）

在 Rust 路径中，`view!` 宏与声明式视图函数是一等等价写法，展开为同一套构建与挂载调用：

```rust
use nana_ui::runtime::view;

view! {
    <Column gap=8>
        <Button @activate={save}>"确定"</Button>
    </Column>
}
```

四个过程宏定义在 `crates/nana-ui-view-macros/src/lib.rs`，由 `nana-ui-runtime` 在 `view-macro` feature 下再导出：

| 宏名称 | 入口位置 | 编译期展开行为 |
| --- | --- | --- |
| `view!` | `view-macros/src/lib.rs:17` | 解析 `<Tag>` 模板树并交给 `view-codegen`，展开为 `column` / `text` / `each` / `when` 等视图函数调用 |
| `#[derive(Store)]` | `view-macros/src/lib.rs:24` | 为结构体生成 `<Name>StoreFields` trait，为每个具名字段提供路径追踪方法 |
| `css!` | `view-macros/src/lib.rs` | 接收单元素 CSS 声明块（支持 Rust Token CSS 或原始字符串 `css!(r#"..."#)`），构建时经 `nana-ui-css` 编译为 `StylePatch` 供 `.css(...)` 使用 |
| `stylesheet!` | `view-macros/src/lib.rs` | 将 `mod styles; .card { … }`（支持 Token 或原始字符串）编译为包含静态 `Sheet` 与同名 `view::Class` 常量的模块 |

#### 单一事实来源：`nana-ui-view-schema`

`crates/nana-ui-view-schema/src/lib.rs:49` 的 `for_each_control!` 宏在一处集中定义了内置控件的模板标签、构造函数、字段写入方式（`write`）与事件：

- `nana-ui-runtime` 将这张表展开为每个控件的类型化 setter、`.model(...)` 与事件绑定方法；
- `nana-ui-view-codegen` 将同一张表展开为 `<Tag>` 模板编译期校验表——因此拼写错误的标签名或属性名在编译期就会在对应行号直接报错，而在表里新增一个字段则两边同时生效。
- 字段写入策略 `write`（`lib.rs:23-40`）精确控制值如何落入组件：`set`（普通字段直接赋值）、`text_state`（编辑文本仅在内容实际不同时替换，保护输入光标位置）、`assign`（数值输入框赋值并将光标移至末尾）、`clamp`（滑块范围钳制）、`span_low` / `span_high` / `span`（双滑块区间钳制，推着另一端移动而不允许交叉）、`builder`（调用同名构建器方法以维护派生分量，如 `ColorField` 的 HSV）、`tab`（切换选中页签并同步焦点）、`resource`（绑定宿主纹理槽）。

#### 构建时样式编译：`view-codegen/src/style.rs`

`crates/nana-ui-view-codegen/src/style.rs` 提供 `compile_inline`（`css!` 与 `style="..."`）、`compile_styles`（`<style>`）与 `compile_stylesheet`（`stylesheet!`）入口，并在构建期直接构建于 `nana-ui-css` 的 `StyleOp` 管线之上：

- 完整支持 CSS Cascade 5 `@layer` 优先级排序（`CascadeKey`）、`@media`（经 `Sheet::with_media` 与 `SheetMediaQuery` 按 Kleene 三值逻辑响应式求值）、`@container`（区间断点桶）、`@supports`、`@keyframes` / `transition`、`@font-face`（经 `Sheet::with_font_faces` 接入宿主字体注册）；
- 支持类与复合类选择器、静态子代（`>`）与后代（` `）组合器、主体及祖先交互伪类（`:hover` / `:focus` / `:focus-visible` / `:active` / `:selected`、`.card:hover .icon` → `track_interactive_state` + `ancestor_pseudo`）、绘制级伪元素（`::placeholder`、`::selection`、`::-webkit-scrollbar*`）、元素级静态 `--*` 自定义属性继承（分层 `VarScope`）以及 `.vue` / `view!` 的内联 `style="..."`（Layer 2 `.inline_patch`）；
- 产品构建通过 `emit_style_ops_rust` 生成零开销的纯 Rust `StylePatch(fn(&mut LayoutStyle))` 函数指针（调用 `nana_ui_core::style_patch::set::*`，零运行时 JSON）；开发期热重载产出 `HotStylePatch::from_ops(ops)`；
- 诊断严格分级：语法错误、非法值、未知属性与不支持的选择器或 `@` 规则直接报编译错误，已知可安全忽略的厂商前缀属性或未使用类报编译警告，绝不静默丢弃。

### 2. 管线二：`.vue` 单文件组件编译器（`nana-ui-sfc`）

`nana-ui-sfc` 在应用的 `build.rs` 中将 `<script setup lang="rust">` 的 `.vue` 文件编译为标准 Rust 视图函数（例如 `TodoList.vue` 生成 `pub fn todo_list(...) -> impl IntoView`）：

1. **三块拆解（`parse.rs:23`）**：解析出 `<script setup>`、`<template>` 与 `<style>`，并记录行列偏移以还原错误位置；
2. **全组件静态信号分析与常量折叠（`lib.rs:413`）**：由于编译器能看见整段脚本，它会将从未被写入且未逃逸的 `signal(v)` 自动折叠为 `constant(v)`（只读常量的属性绑定在建节点时直接赋值一次，不创建运行期 `Effect`）；检测出 `computed` 循环依赖直接报编译错误；将可见的静态依赖列表包装为 `__checked`（debug 下核对实际读取是否越界，见 `runtime.reactive.static_deps_mismatch`）；
3. **产物输出（`$OUT_DIR`）**：生成格式化好的 `nana_views.rs`、依赖报告 `nana_views.deps.md` 以及源映射 `nana_views.map.json`（遵循 `nana-sfc-source-map/1`）。开启 `.hot(true)` 时还会将纯静态文本提入可热替换表 `__NANA_HOT`，配合 `nana-ui-dev` 的 `watch_templates` 实现免重启换字。

## 协作与数据流

```text
管线一  view! / css! / stylesheet! ──view-macros──▶ view-codegen（展开 + 构建时 CSS）──▶ Runtime 视图函数 + StylePatch
管线二  .vue 文件 ──nana-ui-sfc──▶ 信号静态分析折叠 + view-codegen 展开 ──▶ $OUT_DIR/nana_views.rs
```

## 代码地图

| Crate | 核心入口文件 | 规模与定位 |
| --- | --- | --- |
| `nana-ui-view-schema` | `crates/nana-ui-view-schema/src/lib.rs` | 236 行：`for_each_control!` 唯一内置控件表 |
| `nana-ui-view-macros` | `crates/nana-ui-view-macros/src/lib.rs` | 685 行：`view!`、`css!`、`stylesheet!`、`Store` 过程宏入口 |
| `nana-ui-view-codegen` | `crates/nana-ui-view-codegen/src/{lib,style}.rs` | 1,873 + 1,504 行：模板展开器与构建时 CSS 编译器 |
| `nana-ui-sfc` | `crates/nana-ui-sfc/src/{lib,parse}.rs` | 1,030 + 638 行：`.vue` 编译器与静态信号依赖分析 |

## 延伸阅读

- [作者入口](/learn/authoring-paths)：两条编写路径如何在 `UiMutation` / `UiWorld` 汇合
- [CSS 引擎](/learn/css-engine)：构建期调用的 `nana-ui-css` 内部实现
- [.vue 方言（指南）](/guide/scaling/sfc) · [声明式视图（参考）](/reference/reactive-view)
