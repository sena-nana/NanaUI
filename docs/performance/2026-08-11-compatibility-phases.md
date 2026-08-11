# 兼容性阶段审计索引（2026-08-11）

本文件是日期索引；**权威正文**见：

→ [`docs/compatibility-roadmap.md`](../compatibility-roadmap.md)

## 当日结论（摘要）

| Phase | 主题 | 状态 |
|-------|------|------|
| A | 布局 CSS 硬闸（css-parity） | 硬闸闭合；**A-05 typography / A-06 Button chrome done**；A-04 纪律持续 |
| B | 定位（fixed 子集；sticky defer） | fixed 闭合；sticky defer |
| C | DOM/JS（query/metrics/scrollIntoView/clipboard/lifecycle） | X4/X5/X6 **已兑现**；Observer 真语义 defer |
| D | Vue host（cache/contains/Teleport/事件） | D-01–D-04 **闭合**（事件为扇出子集；祖先冒泡 / `:focus` defer） |
| E | Lilia 证据（Repo/overlays） | home/settings/**repo** + overlays **闭合**（E-01–E-03）；E-04 Diff/Actions defer |

交叉：[`css-layout-parity.md`](../css-layout-parity.md)、[`vue-nana-renderer-system.md`](../vue-nana-renderer-system.md)、[`capabilities.md`](../capabilities.md)、[`2026-08-10-lilia-fidelity-gap.md`](2026-08-10-lilia-fidelity-gap.md)、[`2026-08-11-button-css-chrome.md`](2026-08-11-button-css-chrome.md)（A-06）。
