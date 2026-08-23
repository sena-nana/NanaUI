# 布局

第一路径用控件自己的布局。这篇讲的是 Vue 兼容路径能读到的那一部分网页 CSS，用来排界面骨架。它不是浏览器里的 CSS 引擎：没有完整的选择器世界，也没有「看起来像网页，其实差几个像素就放宽」这种做法。排出来对不上，改布局，不改容差。

对话框、抽屉、菜单请用 [控件](components.md) 里的浮层。不要用 `position: fixed` 自己做一层网页弹窗。

## 能用的

**Flex。** 横排、纵排、换行、间距、对齐、拉伸和收缩、`order`。侧栏加主区、工具条加内容，用这一套就够。

**尺寸。** 像素、百分比、`min` / `max`、简单的加减、`vw` / `vh`、`em` / `rem`、`min()` / `max()` / `clamp()`。

**盒子。** padding、margin、边框计入盒子。默认按 border-box 理解。

**一层网格轨道。** 可以写 `grid-template-columns` / `rows`、`fr`、`minmax`、固定次数的 `repeat(N, …)`。这不是完整的二维 Grid：没有 `auto-fit`、没有跨轨、没有自动往空格里填。

**定位。** `relative`、脱流的 `absolute`，以及相对窗口的 `fixed`。`fixed` 只适合普通节点贴在视口上；产品浮层仍走控件。

**文字。** 字号、字重、字体、行高、字距、颜色。字距是近似，不要拿它当排版软件。

**隐藏。** `display: none` 和 `visibility: hidden` 都不占位、不参与点击。

## 不要指望的

`sticky`、从右到左的书写方向、`@media`、`:hover` 这类交互伪类、`!important`、完整的 `calc()` 表达式、网页里那种会自动折列的 `repeat(auto-fit)`。

主题色走控件和外观设置，不要用任意业务色去改框架令牌。

## 内部如何工作

布局数据在 `nana-ui-core::box_layout`（`LayoutStyle`、长度、grid 轨道）。解析和级联现在住在 `nana-ui-vue`（`css_map` / `css_cascade`）。真正算盒子的是 Runtime 的 `RuntimeLayoutEngine`：Vue 的 `measure_layout` 只是薄适配，产品帧走 `RuntimeDocument::flush`，同一套算法。

```text
声明 / class / stylesheet
        │
        ▼
   LayoutStyle（纯数据）
        │
        ▼
RuntimeLayoutEngine → 未滚动的 LayoutBox（权威在 Runtime）
        │
        ▼
绘制阶段的 LayoutBoxStore（只给这个窗口的 JS 查询）
```

中立的布局内核里不应出现 `nana-*` 类型、Vue DOM 类型或业务 class 特判。Nana 壳层的 class 提示（侧栏宽度这类）留在 `shell_contract`，不要渗进 core。

产品几何权威在 Runtime / UiScene。`LayoutBoxStore` 不写回 Runtime 的 `LayoutBox`，滚动只影响绘制投影。公共 `nana-ui` 不引入 CSS 解析依赖。

对照测试在 `nana-css-parity`：同一套 fixture 比 Nana 的盒子和期望值（可选 WebView 参照），默认容差 ±2px。失败时改引擎，不放宽夹具。这套测试不得链进 `nana-ui` 的默认依赖。
