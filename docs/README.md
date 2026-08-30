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

## Vue 兼容

已有 Vue / JS 界面要落到同一棵原生树时再看这些。新应用不要从这里起步。

| 你要做的 | 看 |
| --- | --- |
| Vue 怎么进这棵树 | [Vue](vue.md) |
| 兼容路径里 CSS 能写到哪 | [布局](layout.md) |

## 查阅

| 主题 | 看 |
| --- | --- |
| 入口类型、feature、扩展控件 | [应用 API](application-api.md) |
| L3 组成式建树（`build` / `mount`） | [L3 组成式建树](l3-authoring.md) |
| crate 分层、所有权（改框架时） | [架构](architecture.md) |
| 保留树与抽取（改 Runtime 时） | [Runtime 与 Scene](runtime-scene.md) |
| Android（实验，非产品目标） | [Android](android.md) |
