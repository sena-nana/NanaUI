# 类与样式

排一行或一列，先用 `column()` / `row()` 的 `gap`。要写一段 CSS 时，样式表在构建时编译，运行时不再解析。`view!`、`css!` 和 `stylesheet!` 都在 `view-macro` 后面。属性全集不在这里列出，见 [布局（CSS 子集）](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/layout.md) 和 [布局（Rust）](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/rust-layout.md)。

类名写错，两边都是编译错误。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::view::IntoView;

fn todos(empty: nana_ui::runtime::view::Signal<bool>) -> impl IntoView {
    view! {
        <style>
            .todos { opacity: 1; transition: opacity 120ms ease-out; }
            .todos.empty { opacity: 0.6; }
        </style>
        <Column class="todos" class:empty={empty} gap=8>
            <Text>"任务"</Text>
        </Column>
    }
}
```

```rust rust
use nana_ui::runtime::view::*;

stylesheet! {
    mod todo_styles;
    .todos { opacity: 1; transition: opacity 120ms ease-out; }
    .todos.empty { opacity: 0.6; }
}

fn todos(empty: Signal<bool>) -> impl IntoView {
    column()
        .gap(8)
        .class(todo_styles::todos)
        .class_when(todo_styles::empty, empty)
        .children(text("任务"))
}
```

:::

模板里的 `<style>` 放在最前面，编译成一张 `Sheet`。`class="todos"` 展开成 `.class(…)`，`class:empty={empty}` 展开成 `.class_when(…)`。`stylesheet!` 为每个类生成一个 `Class` 常量，`-` 换成 `_`，所以上面的 `.empty` 是 `todo_styles::empty`。`mod todo_styles;` 的常量只给父模块用，写成 `pub mod` 才公开。

一个元素的类必须来自同一张表。固定类、条件类和交互伪类上的 `transition`（支持 `transition-delay`）会在状态变化改变 `opacity`、`transform`、`filter`、`width`、`height`、`background` 或 `color` 时，沿着合成器轨道播到新值；`@keyframes` 和 `animation` 同样编译成 `#87 MotionTrack` 轨道并在运行时按当前主题求值。逻辑样式直接取新值。

只给一个元素写几条声明，在模板（`.vue` 或 `view!`）中可直接写内联 `style="..."`，在函数写法中用 `.css(css! { ... })`：

```rust
use nana_ui::runtime::view::{column, css};

column().css(css! { padding: 12px; opacity: 0.8 })
// 也支持原始字符串字面量，规避所有 Rust 词法限制：
column().css(css!(r#"padding: 12px; font-size: 1.5em;"#))
```

`css!`、`stylesheet!` 与 `view!` 的 `<style>` 同时支持 Rust Token 写法与原始字符串字面量（`r#"..."#`）。在 Token 模式下，需要 CSS 字符串值的属性（如 `grid-template-areas`、`font-family`、`font-feature-settings`、`font-variation-settings`、`content` 以及 `url(...)` 与选择器）会自动保留双引号；其余属性里 Rust 词法写不出的值（如 `"1.5em"`、`"#9ecafe"`）仍可放进双引号转义。

元素样式遵守严格的 4 层覆盖顺序，与链式调用顺序无关：**Layer 0**（控件默认布局）→ **Layer 1**（样式表规则：`.class` / `.class_when` / 交互伪类 / `@container` / `@media` / 祖先交互）→ **Layer 2**（内联 `style="..."` / `.css(css!{...})` / 静态 Rust 构建器）→ **Layer 3**（响应式 Rust 属性绑定）。

::: warning
一个元素混用两张样式表的类，会在运行时断言失败。条件类和固定类写同一份 `stylesheet!`。
:::

构建期端到端编译类与复合类选择器（`.todos`、`.todos.empty`）、主体交互伪类（`:hover`、`:focus`、`:focus-visible`、`:active`、`:selected`）、模板 `<style>` 内的静态子代（`>`）与后代（` `）组合器及祖先交互选择器（如 `.card:hover .title`）、绘制级伪元素（`::placeholder`、`::selection`、`::-webkit-scrollbar*`）、元素级静态 `--*` 自定义属性继承、内联 `style="..."`，以及 `@layer`、`@media`、`@container`、`@supports`、`@keyframes`/`animation`、`transition` 与 `@font-face`。语法错误、非法值、未知属性与不支持的选择器或 `@` 规则（如兄弟组合器 `+`/`~`、标签/id/属性选择器、`::before`/`::after` 生成盒）会直接报编译错误；已知可安全忽略的厂商前缀属性（如 `-moz-*`、`-ms-*`）报编译警告。不要假设「写进样式表就一定生效」。

`@container` 块里的类规则也编进去，按容器的尺寸生效。容器用 `container-type` 和 `container-name` 声明：

```css
.card { container-type: inline-size; container-name: card; }
@container card (max-width: 300px) { .row { height: 40px; } }
```

`card` 的宽度不超过 300px 时，里面的 `.row` 高 40px。尺寸由运行时量，跨过断点时只重排受影响的节点。写在后面的普通规则仍然压过容器规则。一个元素同一时刻只跟一个容器的一条轴，至多 16 个断点，超出时在元素上报警告。`css!` 里不能写 `@container`。能写的条件和不支持的部分见 [布局（CSS 子集）](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/layout.md)。

`each`、`when` 的容器同样接受 `.class`、`.class_when` 和 `.css`。模板里用 `<Block class="strip">` 包住一个 `v-for` 或一条 `v-if` 链，类给容器，不给每一行。行和分支自己的 `class` 仍然留在行上。

## 跟着主题走的表面

样式表里的主题色 token（`var(--nana-*)`、语义色名、`theme(...)`、`light-dark(...)`、`:root` / `:root[data-theme="dark"]` 自定义属性以及含语义色的 `color-mix(...)`）会编译成动态 `PaintColor`，切换已安装的 `ThemeId` 或外层 `ThemeScope`（`El::theme_colors`）时在运行时自动重算；写死的字面颜色仍保持静态。视图上每个元素也都有 `.foreground`、`.background`、`.border` 和 `.radius`：前三个是 `SemanticColorRole`，圆角是 `RadiusTier`，都可以绑定。

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{RadiusTier, SemanticColorRole};

view! {
    <Column gap=8 background={SemanticColorRole::Surface} radius={RadiusTier::Md}>
        <Text foreground={SemanticColorRole::Muted}>"次要说明"</Text>
    </Column>
}
```

```rust rust
use nana_ui::runtime::view::{column, text};
use nana_ui::runtime::{RadiusTier, SemanticColorRole};

column()
    .gap(8)
    .background(SemanticColorRole::Surface)
    .radius(RadiusTier::Md)
    .children(text("次要说明").foreground(SemanticColorRole::Muted))
```

:::

`.border` 只写边框颜色。宽度在布局里。颜色和宽度缺一边，边框就不画。组件上一次写全用 `NodeStyle::outline(role, width)` 或 `Stack::outline(role, width)`，见 [布局（Rust）](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/rust-layout.md)。

内置 `nana.light` 和 `nana.dark` 主题提供两套预制 token；应用也可以通过 `ThemeDefinition + ThemeRegistry` 注册自定义主题。当前选择持久化为 `ThemeId`，组件只依赖语义角色，不要依赖主题名称或外观提示。

卡片的留白、`Stack` 的预设（`row`、`column`、`fill_column`）也在 Rust 布局那一篇。页面外沿和卡片内部是两道边，框架不会按嵌套自动清掉。

## 接下来

<div class="next-steps">
  <a class="next-step" href="/guide/essentials/conditional">
    <p class="next-step-link">条件</p>
    <p class="next-step-caption">v-if 怎样展开成 when。</p>
  </a>
  <a class="next-step" href="/reference/layout">
    <p class="next-step-link">布局（CSS 子集）</p>
    <p class="next-step-caption">样式表能写的属性，以及写了也不会生效的部分。</p>
  </a>
</div>
