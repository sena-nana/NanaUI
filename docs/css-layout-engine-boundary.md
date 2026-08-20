# CSS / 布局引擎边界（压缩与下一刀）

> 产品运行时无 WebView（`AGENTS.md`）。本文件定界 **中立 layout 内核** vs **Vue/Nana 适配**，
> 避免半吊子抽 crate 或巨型新引擎。验收闸门仍是 [`css-layout-parity.md`](css-layout-parity.md)。

## 判定

| 问题 | 结论 |
|------|------|
| 现在能否抽高性能中立 parser+layout crate？ | **有条件** — 先剥离，再抽 |
| 现在是否新建完整 CSS 引擎？ | **否** |
| 公共 `nana-ui` / core 是否引入 CSS parse？ | **否**（`style_model` 合同不变） |

## 理想分层

```text
L0  nana-ui-core::box_layout     LayoutStyle / LengthSpec / grid 分配（纯数据）
L1  neutral parse                声明 / 长度 / var / 网格轨 → LayoutStyle
L2  cascade                      stylesheet match → LayoutStyle（MatchContext 注入树）
L3  measure                      LayoutStyle 树 → MeasuredBox（测试 + 预绘制回退）
L4  shell_contract（非中立）     nana-* / 工具 class → 部分 LayoutStyle（留 vue）
L5  Scene/Runtime adapter        LayoutStyle / Semantics → UiWorld / UiScene
```

现状：L0 已在 core；L1–L3 在 `nana-ui-vue`（`css_map` / `css_cascade` / `measure`）；
L4 = `shell_contract`（`css_map::LayoutStyleCss::apply_class_layout_hints` 薄委托）；
L5 = Scene host（`scene-view`）。历史 `iced_app` 是旧 Iced widget 适配，不再是产品绘制环。

## 权威事实源

| 数据 | SoT | 备注 |
|------|-----|------|
| Stylesheet → `LayoutStyle` | `MessageBridge` + `css_cascade` | `NanaTreeDocument::stylesheets` 仅诊断计数 |
| 产品几何盒 | Runtime/UiScene → `LayoutBoxStore` | paint 后 JS 投影；权威在 Runtime |
| 预绘制 / parity / hit-test 盒 | `MessageBridge::resolve_document_layout` → `measure_layout` | 无 paint 盒时回退；与 css-parity 共用算法 |

几何只保留两个阶段：paint 前 Style Model measure，paint 后 Runtime writeback。
`NanaTreeDocument` 只缓存两者结果，不再拥有 `StyleIntent + resolve_now` 合成布局。

迁移旧诊断 API 时，调用宿主 `resolveLayout` / `VueHost::resolve_layout` 获取同一
Style Model 的预绘制盒；`BoxSnapshot` 不再暴露另一套合成背景结果。这样 API
不会暗示仍存在第三个几何或绘制权威。

## Neutral 定义

中立内核 **不得** 包含：

- `nana-*` / `WidgetKind` / `HostValue` / iced / Vue DOM 类型
- 业务或 `lilia-*` BEM class 特判（已清理，禁止倒退）

允许留在 `nana-ui-vue`：

- `shell_contract::apply_class_layout_hints`（文档化的 Nana 壳 / controls 合同）
- `widget_map` / `layout_map` / DesktopShell 投影（`region_views` 等）
- `svg_icon` / iced canvas heatmap（临时 L1 paint 例外）
- `style` L1 paint 色值解析（`parse_css_color`；供 `resolve_paint_color` 复用）

## L1 规范化进度

**已完成（本回合 + 前序压缩）：**

| 项 | 状态 |
|----|------|
| 删除 `view` 别名 / Blitz 空 feature / `map_widget_kind` | ✅ |
| 模块头标明 cascade SoT、measure 角色、shell 非中立 | ✅ |
| `apply_class_layout_hints` → `shell_contract.rs`；`css_map` 中立 parse + 薄委托 | ✅ |
| 删除 `StyleIntent` / `resolve_now` 合成几何；host `resolveLayout`、stylesheet、theme、viewport 统一走 `resolve_document_layout` | ✅ |
| measure / Runtime 两阶段 SoT 写进 `measure.rs` / `tree.rs` / `box_layout` 头 | ✅ |
| 共享盒助手留在 `nana-ui-core::box_layout`（content-box / inset / padding·margin·gap） | ✅（标明；未另抽 crate） |

**仍属短期、勿抽 crate：**

1. 不新增 iced-primitive paint 分支；heatmap 优先 SVG 或 L3 控件（L2 已标 DEFER canvas）
2. ~~selector 匹配索引 / dirty 子树 cascade~~ → **声明 entries 已缓存**（`StyleRule.declaration_entries`；match 不再重切；document `--*` 从 rules 重建，不刮 raw）；inject 空 sheet 跳过全树；完整 dirty 子树 / 选择器索引仍待
3. ~~`iced_app` 按文件切分~~ → 见下方 **L2 规范化进度**

## L2 规范化进度（Vue 适配 / 树→Runtime）

> 与上节 L1 并行；互不删除对方段落。L2 = 语义树 → `widget_map` → Runtime/UiScene → `nana_ui` Scene host。
> 下表中的 `iced_app` 切分是历史完成项，不是当前产品绘制环。

**已完成（本回合）：**

| 项 | 状态 |
|----|------|
| `iced_app` 按文件切分（`include!` 同模块）：`layout_flow` / `button` / `settings` / `layout_convert` / `l1_charts` / `surface` / `overlay` / `selection` | ✅ 行为不变 |
| Heatmap 单轨：优先 `svg_icon`（resvg）；canvas path-d 标 **DEFER**，不删除以免破视觉 | ✅ |
| Semantics 集中：`widget_map` 为 kind 唯一解析入口；`bridge` / `layout_map` / `renderer` 模块头标明 L2 边界 | ✅ |
| `svg_icon` 标明 L1 几何→iced 适配，禁止扩第二套 path-d 解析 | ✅ |
| DesktopShell `region_views` 投影 | ✅ 保持 bridge 所有权；一次私有索引统一 reachability / nearest owner / nested seed，祖先清理由全表循环改为父链队列 |

**L2 仍不做：**

- 半吊子抽 crate；业务 class 特判倒退
- 删除仍被视觉路径依赖的 canvas heatmap（待 L3 CalendarHeatmap）
- `view_widget` / `view_widget_owned` 合并（需生命周期方案）

## 中期抽 crate 边界（下一刀，需测绿）

建议名：`nana-layout`（或 `nana-css-layout`）

```text
nana-layout
  model/     ← box_layout（或依赖 core）
  parse/     ← 从 css_map 剥离，无 class hints / HostValue
  cascade/   ← css_cascade 平移
  measure/   ← measure + 显式 LayoutEnvironment（替 thread-local）

nana-ui-vue
  shell_contract/  ← apply_class_layout_hints（已就位）
  bridge/          ← MatchContext 构建
  Scene host       ← Runtime/UiScene 适配（`scene-view`）
```

**验收：**

- `cargo test -p nana-css-parity` / `compare` 零回归
- `cargo test -p nana-ui-vue --features scene-view --lib`
- `cargo test -p nana-ui-core --lib`
- `nana-ui` / `nana-ui-core` **不**新增 CSS parse 依赖

## 明确不做（直到边界清晰）

- 完整 CSSOM / `@media` / `:hover` / sticky / 完整 2D grid / 完整 calc AST
- 产品路径引入 WebView
- 弃用 `measure` 产品回退前未保证首帧 JS 几何可接受
- 把 `region_views` / `reparent_orphans` 半迁到 demo 却留双路径
- 合并 `view_widget` / `view_widget_owned` 前无生命周期方案与测绿
- 本回合抽半吊子 `nana-layout` crate

## 相关文档

- [`css-layout-parity.md`](css-layout-parity.md) — 子集与 fixture 闸门
- [`vue-nana-renderer-system.md`](vue-nana-renderer-system.md) — L1/L2/L3
- [`architecture.md`](architecture.md) — 仓库总架构
