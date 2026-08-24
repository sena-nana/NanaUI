# 布局（Vue CSS 子集）

Rust 第一路径用控件自己的布局，不写 CSS。这篇只描述 Vue 兼容路径能用的那一部分网页 CSS，用来排界面骨架。

它不是浏览器里的 CSS 引擎：没有完整选择器世界，对不上就改布局，不靠容差假装一致。对话框、抽屉、菜单用 [控件](components.md) 里的浮层。

## 能用的

**Flex。** 横排、纵排、换行、间距、对齐、拉伸和收缩、`order`。侧栏加主区用这一套就够。

**尺寸。** 像素、百分比、`min` / `max`、简单加减、`vw` / `vh`、`em` / `rem`、`min()` / `max()` / `clamp()`。

**盒子。** padding、margin、边框计入盒子。默认按 border-box。

**一层网格轨道。** `grid-template-columns` / `rows`、`fr`、`minmax`、固定次数的 `repeat(N, …)`。不是完整二维 Grid：没有 `auto-fit`、没有跨轨、没有自动填空。

**定位。** `relative`、脱流的 `absolute`、相对窗口的 `fixed`。`fixed` 只适合普通节点贴在视口上；产品浮层仍走控件。

**文字。** 字号、字重、字体、行高、字距、颜色。字距是近似。

**隐藏。** `display: none` 和 `visibility: hidden` 都不占位、不参与点击。

## 不要指望的

`sticky`、从右到左的书写方向、`@media`、`:hover` 这类交互伪类、`!important`、完整 `calc()`、会自动折列的 `repeat(auto-fit)`。

主题色走控件和外观设置，不要用任意业务色去改框架 token。

未知声明会被忽略；不要假设「写了就能在某处生效」。

## 和 Rust 布局的关系

Vue 侧解析出的是同一份 `LayoutStyle`。真正算盒子的是 Runtime 的 `RuntimeLayoutEngine`，产品帧走 `RuntimeDocument::flush`。JavaScript 查询到的盒子是绘制阶段的投影，滚动不写回 Runtime 的布局权威。
