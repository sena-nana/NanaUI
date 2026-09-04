# NanaUI 文档

给要**写应用**的人。仓库 [README](../README.md) 说明这套框架和其他 UI 差在哪；这里说明怎么接到你的窗口上。

改 NanaUI 自身请看 [架构](architecture.md) 和 [Runtime 与 Scene](runtime-scene.md)，不要从那两篇开始写产品。

## 先读

1. [框架如何运行](how-it-works.md) — 树、所有权、一帧怎么走、GPU 节点是什么
2. [开始](start.md) — Cargo feature、第一扇窗口、`RuntimeProgram`

## 按任务

| 你要做的 | 看 |
| --- | --- |
| 按钮、输入、对话框、侧栏 | [控件](components.md) |
| 排行与列、间距、边框、卡片 | [布局与样式（Rust）](rust-layout.md) |
| 工作区、Region、Dock、设置页 | [工作区](workspace.md) |
| 着色器、预览视口、宿主纹理 | [实时画面](gpu.md) |
| 应用内打开网页（IAB，未实现） | [应用内浏览器](gpu.md#应用内浏览器) |
| 标题栏、图标、系统材质、多窗口 | [窗口](window.md) |
| 颜色、尺寸、字体、主题 | [视觉](look.md) |

## Vue + JS

Vue + JS 是一等 L1/L2 消费入口，新应用和已有 Vue 界面均可使用，与 Rust L3 共用原生树和组件合同。

| 你要做的 | 看 |
| --- | --- |
| Vue 怎么进这棵树 | [Vue](vue.md) |
| Vue 的 CSS 支持范围 | [布局](layout.md) |
| 宿主 API / Fetch 安全边界 | [应用 API](application-api.md) |

L1/L2 兼容子集还缺什么（设计延期，不是烂尾实现）：

- Fetch 只有缓冲式正文；流式 body、`FormData`、cookie、cache、CORS/preflight 未做，非默认 Request 选项明确拒绝。
- WebSocket 只留接口与 shim，需宿主注入 socket host，框架不带默认传输。
- CSS：RTL 不翻转 flex/grid 轴（只映射逻辑 inline padding/text-align）；嵌套 `repeat(auto-fit/auto-fill)` 与 subgrid 未做。`position: sticky` 与整表 `repeat(auto-fit)` 已交付。
- 完整浏览器 DOM/CSSOM、未经 Nana 入口的 `@vue/runtime-dom` 生产 bundle、WebGL、真实 WebView 明确不做。

## 查阅

| 主题 | 看 |
| --- | --- |
| 入口类型、feature、扩展控件 | [应用 API](application-api.md) |
| L3 组成式建树（`build` / `mount`） | [L3 组成式建树](l3-authoring.md) |
| crate 分层、所有权（改框架时） | [架构](architecture.md) |
| 保留树与抽取（改 Runtime 时） | [Runtime 与 Scene](runtime-scene.md) |
| Android（实验，非产品目标） | [Android](android.md) |
