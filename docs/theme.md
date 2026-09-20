# 主题与样式 — Phase 0 审计基线与 Phase 1 ThemeDefinition

这篇覆盖 Theme 改造（[#100](https://github.com/sena-nana/NanaUI/issues/100)）的前两阶段：

- **§0–§6 是 Phase 0（[#101](https://github.com/sena-nana/NanaUI/issues/101)）**：在动架构之前，把当时的主题/样式合同、视觉基线和性能基线钉死。
- **§7 是 Phase 1（[#102](https://github.com/sena-nana/NanaUI/issues/102)）**：`ThemeDefinition` 与 typed token 体系落地，NanaLight/NanaDark 用它表达。

Phase 0 的章节保留成**当时**的审计记录，不回头重写——那份记录的价值就在于它说的是动手之前的样子。Phase 1 改掉的结论在原处标了「→ §7 已解决」，别的照旧。

没有新建 Theme package，没有 ThemeScope，也没有引入字符串 token map。

**没有改任何一个已有像素。** 新增的 29 个 fixture 拍的是以前没拍过的状态（语义基线逐文件确认是纯追加）；token 收敛在默认主题下逐字节不变（语义基线 146/146 MATCH），变的是「数值从哪里来」。

写应用请看 [视觉](look.md) 和 [控件](components.md)；这篇是给要动 Theme 架构的人看的。

## 0. 一分钟结论

| 问题 | 现状 |
| --- | --- |
| 有没有 Style Model | 有，而且是唯一一套：`nana_ui_core::style_model`，L1/L2/L3 共用 |
| 颜色权威 | `SemanticPalette`（24 个字段 + 15 个派生角色）；组件按角色表达意图，不写 RGB |
| 尺寸权威 | **收敛中**：radius、滚动条、control height、control/field/list inset、**panel inset、icon-button 方盒、多行 field 垂直 inset、Large padding** 已跟随安装的 `ThemeMetrics`；renderer 一处都不读常量。剩下的是局部几何 `const` 与少数菜单高度估算 |
| 运动权威 | ~~**分裂**：`ThemeMetrics::motion_*_ms` 是死字段（零消费方），实际时长是 `nana_ui_core::motion` 的 `const`~~ → **§7 已解决**：八个时长各成一个 `MotionRole`，`motion` 的 `const` 反过来读默认主题 |
| Theme 在哪解析 | 按值的大小分两处：**颜色在 extract**（16 字节，下游解析），**尺寸在样式写入时**（`LayoutStyle` 4808 字节，放读路径上会被每帧放大——见 §1.5）。都不在保留期 style 阶段 |
| Theme 切换代价 | 全文档 paint 失效；palette-only 不碰 Layout / Text（已验证） |
| Renderer 懂不懂 Theme | 基本不懂：`nana-ui-scene` 只有 3 处 `SemanticColorRole`，全在测试与 benchmark |
| 组件自带硬编码 RGB | 0 处 |
| 重复的 token 类型 | 已消除两份：`Colors`（= `SemanticPalette`）、独立的 `SCROLLBAR_METRICS`；§7 又消除两份：`space`/`type_scale`/`HAIRLINE`/motion `const` 全部改成读 token 默认值 |
| 状态矩阵 | 组件声明了 paint 的状态，32 个从未被捕获；本轮补了 29 个，剩 3 个说明了为什么补不了 |

## 1. 现有 Style Model inventory

### 1.1 唯一权威关系图

```text
                        AppearanceSettings              WindowMaterialMode
                     (radius ×4 / backdrop /            (Solid / Vibrancy /
                      workspace corners)                 Mica / Acrylic)
                              │                                │
                              └───────────┬────────────────────┘
                                          ▼
                              nana_ui::theme::ThemeTokens
                              （SemanticPalette + ThemeMetrics + titlebar
                                + with_backdrop 覆写 alpha）
                                          │
                              install_theme_tokens
                              → AppContext::set_style_tokens
                                          ▼
   L1 CSS 子集 ─┐                 StyleModelRef  ← ThemeMode
   L2 Vue props ─┼──► Semantics ──►  · theme_mode
   L3 Rust API ─┘   (WidgetKind/     · metrics : ThemeMetrics
                     ButtonKind/     · palette : SemanticPalette
                     ControlSize/    · titlebar: SemanticColor
                     StatusTone…)            │
                            │                │
                            ▼                ▼
                     NodeStyle           UiWorld.style_model
                  （layout: LayoutStyle,        │
                    foreground/background/      │ resolve_styles
                    border: SemanticColorRole,  │ （继承 + palette_paint_colors）
                    interaction: InteractionStyle）
                            │                   ▼
                            └────────────► ComputedStyle  (= 现有 ResolvedStyle)
                                                │
                                         extract_nodes
                                   （palette_epoch 过期时在这里重算颜色）
                                                ▼
                                          ExtractedNode
                                    · style: Arc<ComputedStyle>
                                    · standard_visual: StandardVisual
                                    · component_geometry
                                                ▼
                                      UiScene（ScenePrimitive）
                                                ▼
                                         SceneWgpuPainter
```

两条**旁路**必须一起看，否则关系图会骗人：

```text
UI_METRICS (编译期 const) ──────────────────► view_components.rs 等组件
   （panel / icon / 垂直 field 与模块级 const 仍在；
     world/geometry.rs 与 nana-ui-scene/primitives* 已不在此列）

nana_ui_core::motion::{HOVER_COLOR, …} ─────► 组件动画
   （const，不可主题化；ThemeMetrics.motion_*_ms 无人读）
```

### 1.2 类型清单

| 类型 | 位置 | 是什么 | 归宿 |
| --- | --- | --- | --- |
| `ThemeMode` | [theme/mod.rs](../crates/nana-ui-core/src/theme/mod.rs) | Dark / Light 二选一，可序列化 | **已演进**：§7 的 `ThemeDefinition.mode`，不再是主题的全部 |
| `SemanticPalette` | [style_model.rs](../crates/nana-ui-core/src/style_model.rs) | 24 个语义色字段，dark/light 两份常量 | **保留**，直接作为 semantic color token 层，禁止复制第二份 |
| `SemanticColorRole` | 同上 | 39 个角色：24 个对应字段，15 个在 `get()` 里派生（`WarningSoft*`、`DangerSoft*`、`Titlebar`、9 个代码 token 角色） | **保留**；派生规则是 recipe 的雏形，Phase 1 要把它显式化 |
| `SemanticColorMix` | 同上 | 两个角色的 premultiplied 混合 / 单角色 alpha，权重用 basis points | **保留**，这是「状态层」的现有表达 |
| `SemanticColor` | 同上 | 后端中立 RGBA 0..=1 | **保留** |
| `ThemeMetrics` | [theme/mod.rs](../crates/nana-ui-core/src/theme/mod.rs) | 非颜色 token：radius ×4、control height ×3、padding、icon size、panel padding、field/list padding，外加组合进来的 scrollbar。~~**motion ×2**~~ 已删（§7） | **已演进**：§7 的 `DesignTokens.metrics` 按值持有它本身 |
| `UI_METRICS` | 同上 | `ThemeMetrics` 的 `const` 默认值 | **废弃为唯一权威**：现在是 `DesignTokens::for_mode` 的初值；仍有 62 处产品读取点，见 §1.5 |
| `ScrollbarMetrics` / `SCROLLBAR_METRICS` | [scrollbar.rs](../crates/nana-ui-core/src/scrollbar.rs) | 滚动条独有的 5 个几何 token（thickness 12 / thumb 6 / min length 24 / inset 2 / page 0.9） | **保留**：本轮已由 `ThemeMetrics.scrollbar` 组合持有（§1.5），不再是第三份 metrics |
| `space` / `type_scale` | 同上 | 间距 10 档、字号 7 档（含 `HINT` 11 / `TITLE` 18）+ 字重 4 档，裸 `const` | **已演进**：§7 的 `SpacingTokens` / `TypographyTokens`；`const` 现在读它们的 `DEFAULT`，不再自己拿着数字 |
| `StyleModelRef` | [style_model.rs](../crates/nana-ui-core/src/style_model.rs) | mode + metrics + palette + titlebar 的只读视图，`color(role)` 是 token 读取入口 | **演进**为 compiled theme 的运行期句柄 |
| `ControlSemantics` | 同上 | size + button/card kind + status 的组合 | **保留**，是 recipe 的 selector |
| `ControlSize` / `ButtonKind` / `CardKind` / `StatusTone` / `ToastTone` / `ValidationIntent` | [semantics.rs](../crates/nana-ui-core/src/semantics.rs) | 组件意图枚举 | **保留**，Component Recipe 的 variant 轴 |
| `WidgetKind` | [bridge/semantic.rs](../crates/nana-ui-vue/src/bridge/semantic.rs) | L1/L2 tag/class → 控件类型 | **保留**，仅 Vue 适配层 |
| `ThemeTokens` / `ThemeModeExt` | [nana-ui/theme.rs](../crates/nana-ui/src/theme.rs) | L3 宿主侧 adapter：直接装 `SemanticPalette` + `ThemeMetrics` + titlebar | **保留**（`Colors` 副本本轮已删，见 §1.5）；§7 起 `ThemeTokens` 是 `ThemeDefinition` 的颜色+尺寸投影，`with_backdrop` 按 `SurfaceTokens` 决定给哪个角色上 alpha |
| `RadiusTier` | [theme.rs](../crates/nana-ui-core/src/theme.rs) | 圆角档位意图（Xs/Sm/Md/Lg），由 `NodeStyle.radius` 携带、样式写入时对安装的 `ThemeMetrics` 解析 | **保留**：这是「组件说意图、主题给数值」在尺寸侧的第一个落点，其余 metrics 按同一形状推进 |
| `ControlHeight` | [theme.rs](../crates/nana-ui-core/src/theme.rs) | 控件高度意图：`Min(ControlSize)` / `Exact(ControlSize)`，由 `NodeStyle.control_height` 携带、样式写入时解析 | **保留**：`Min` / `Exact` 的区分以前藏在调用点写 `min_height` 还是 `height` 里，现在是被说出来的意图 |
| `ControlPadding` | 同上 | 水平 inset 意图：`Compact` / `Standard` / `Roomy` / `Field` / `ListItem`，由 `NodeStyle.control_padding_x` 携带；多行 field 另用 `control_padding_y` | **保留**：不是 `ControlSize` 的包装——text field 与 list row 各有独立 metrics 字段 |
| `SurfacePadding` | 同上 | 面板 inset：`Panel` 写四边，`PanelX` 只写左右 | **保留**：卡片/设置组/表单表面用，不是 control |
| `SquareSize` | 同上 | 方盒：`IconButton` 或 `Control(ControlSize)`，写 `min_width`/`min_height` | **保留**：icon button 默认走 `icon_button_size`，`size()` 才换成 ControlSize |
| `ChromeRadii` | 同上 | 已解析的四档圆角，搭 `ExtractedNode` 运到 renderer | **保留**：让 renderer 只消费不解析，`nana-ui-scene` 因此一处都不再读 `UI_METRICS` |
| `AppearanceSettings` | [settings.rs](../crates/nana-ui-core/src/settings.rs) | 用户/系统 policy：radius ×4、window material、backdrop target/opacity、titlebar 跟随、workspace corners | **演进**为 #100 §11 的 policy overlay，不是第二套 theme |
| `NodeStyle` / `InteractionStyle` / `SemanticPaint` | [nana-ui-runtime](../crates/nana-ui-runtime/src/) | 节点本地样式意图（语义色角色 + 圆角档位）+ 7 个交互状态的 paint overlay | **保留**，是 explicit local style intent 与 state 输入；意图字段是尺寸/颜色收敛的落点 |
| `ComputedStyle` + `ResolvedStyle(Arc<ComputedStyle>, palette_epoch)` | [nana-ui-runtime](../crates/nana-ui-runtime/src/) | 继承解析结果 + 主题代 | **演进**为 #100 §6 的 `ResolvedStyle`（现在已经叫这个名字，但只覆盖继承与颜色，不覆盖 metrics/recipe） |
| `StandardVisual` / `ComponentGeometry` | 同上 | 保留期「这是个什么控件 / 它的几何」 | **保留**，是 recipe 的 selector 与 layout 产物 |

### 1.3 L1 / L2 / L3 默认样式来源

| 层 | 默认样式从哪来 | 冲突点 |
| --- | --- | --- |
| **L3 Rust** | `view_components.rs` 的 `impl ComponentView`：按 `ButtonKind` 等语义选 `SemanticColorRole`，写进 `NodeStyle.interaction`；几何与其余颜色由 `world/geometry.rs` + `world/extraction.rs` 决定 | 同一控件的外观分散在三处：组件体（角色 + 状态）、extraction（`StandardVisual` 配色）、scene primitives（圆角/描边） |
| **L2 NanaVue** | `packages/nanavue-components` 的 props → Semantics；`src/nana-controls.css` 仍有 532 行样式，含 `--lilia-*` / `--nana-*` 变量与 fallback 字面色（`#dfe2e7`、`#e2e2e2` 等） | CSS fallback 值与 `SemanticPalette` 是两份数字。`SEMANTICS.md` 已禁止独立 `#3867ff`，但 fallback 链还在 |
| **L1 Vue/CSS** | [`css_map.rs`](../crates/nana-ui-vue/src/css_map.rs) 把 CSS 子集映射到 Layout；[`style.rs`](../crates/nana-ui-vue/src/style.rs) 只做 paint 解析；已知 token 名经 `SemanticColorRole::from_css_token_name` 进 Tokens，未知 `#hex` 只能当受限 paint hint | 规则已经写死且有测试，是目前最干净的一层 |

`from_css_token_name` 认得的名字就是 L1 的正式 token 表面：`background`/`surface`/`subtle`/`hover`/`active`/`selected*`/`border*`/`text`/`muted`/`faint`/`accent*`/`success`/`warning*`/`danger*`/`titlebar`，以及 `--nana-` 前缀与 `var()` 包裹。9 个代码 token 角色（`Keyword`/`Function`/…）**不在** L1 表面，只在 L3 语义高亮里用。

### 1.4 审计发现

按影响排序。每条都有可复现的依据。

**F1 — Metrics 有两个权威，编译期常量赢。**
`AppearanceSettings::metrics()` 造出带自定义 radius 的 `ThemeMetrics`，经 `set_style_tokens` 装进 `StyleModelRef`，`apply_style_model` 也正确地把 metrics 变化升级成 LAYOUT 失效。但产品代码（排除 tests 与 bin）里读 **`UI_METRICS` 常量 116 行**（runtime 92 / scene 21 / host 3），读安装值只有 **16 行**，且全在 runtime。典型如 `view_components.rs` 的 `control_layout()` 直接写 `border_radius: Some(6.0)`，而同一文件另一处写 `layout.border_radius = Some(world.theme_metrics().radius_md)`。
→ #100 Phase 1 的第一件事应该是把 metrics 读取点收敛，否则 `ThemeDefinition` 装了也不生效。**本轮做了 radius、滚动条、control height 与 control/field/list 水平 inset 四条**（scene 0；runtime 里剩下 panel / icon / 垂直 field 与模块级 const），剩下的与阻塞原因见 §1.5。

**F2 — Motion token 是死字段。**（**§7 已解决**）
`ThemeMetrics::motion_fast_ms` (120) / `motion_standard_ms` (240) 全仓零消费方。实际时长是 `nana_ui_core::motion` 的 8 个 `const`：`HOVER_COLOR` 120、`OVERLAY_FADE` 140、`MENU_OPACITY` 160、`MENU_POP` 180、`SIDEBAR_COLLAPSE` 260、`SKELETON_PULSE` 1400、`SPINNER_ROTATION` 900、`LOADING_SPIN` 800。它们是 `const`，主题改不了。
→ #100 §8 / #87 对接时，这两个字段要么接上要么删掉，不能继续当「看起来已经有了」。**Phase 1 选了删掉**：那两个字段没了，八个时长各自成为一个 `MotionRole`，`SIDEBAR_COLLAPSE` 的 260 不再需要和谁对齐——它就是自己那一档。

**F3 — Theme 在 extract 阶段解析，不在保留期 style 阶段。**
`SetTheme` 只标 RENDER，不标 STYLE（[world/tests.rs](../crates/nana-ui-runtime/src/world/tests.rs) 的 `set_theme_marks_render_not_style_when_only_palette_roles_change` 已经钉住这个行为）。真正换色发生在 `extract_node` 里：`resolved_epoch != palette_epoch` 时重跑 `palette_paint_colors`，把新颜色打进 `ExtractedNode.style`，保留期 `ComputedStyle` 并不刷新。
这对 palette-only 切换是**便宜**的（不碰 Layout、不 reshape，见 §4），但它把 token 读取放在了每帧 extract 路径上，正是 #100 §6 要移走的形状。

本轮在尺寸侧试过把同一形状推广过去，**不成立**：颜色一个 16 字节，重算便宜；尺寸落在 4808 字节的 `LayoutStyle` 里，放同一条路上会被每帧放大（§1.5）。所以 #100 §6 把解析移进保留期时，不是把 extract 的做法照搬——**要按被解析值的大小分别定解析点**。

**F4 — Theme 安装 = 全文档失效。**
`apply_style_model` 遍历所有 live document roots 的整棵子树并逐节点 `mark`。1k 控件的 palette-only 切换 = 1001 个节点 paint 失效。#100 §7 的 dependency class 就是要把这个数字降下来；Phase 0 的职责是把它量出来（§4）。

**F5 — 组件不写 RGB，但把角色决策写在自己身上。**
122 个组件（`impl ComponentView for X` 加上 `impl X`）里：raw RGBA **0 处**，裸设计数字 **29 处**。`SemanticColorRole` 决策 232 处（`Button` 22、`SidebarRow` 10、`ListItem` 9…），另有 216 处在组件之外（`world/extraction.rs` 的 `StandardVisual` 配色、`menus.rs`、`popover.rs`、`select.rs`、`settings.rs`…）。
这**不是缺陷**——「primary 的 hover 是 AccentStrong」正是 #100 要组件表达的语义。它是 Component Recipe 的迁移人口：448 个角色决策要搬家。

> 本轮修过一次：最初只扫 `impl ComponentView` 函数体，于是 `ListItem` 报成「不声明任何状态」——它的 `InteractionStyle` 整个写在 `ListItem::new` 里。构造函数才是组件默认外观的所在，扫描器现在两种 impl 都算。

**F6 — 交互状态由 19 个组件自己声明。**
`InteractionStyle` 在 122 个组件里有 19 个用到：`Button`、`Checkbox`、`Chip`、`CommandPalette`、`Dropdown`、`HoverCard`、`IconButton`、`InteractiveCard`、`ListItem`、`NumberInput`、`RangeField`、`SearchDropdown`、`SettingsCollapsibleCard`、`SidebarRow`、`SidebarSection`、`Switch`、`TextArea`、`TextInput`、`XYPad`。其余控件的 hover/pressed/selected 视觉来自 `StandardVisual` 在 extraction / scene 的分支。
→ #100 §4 的统一状态位要同时覆盖这两条路径，只改 `InteractionStyle` 会漏掉一百来个控件。

**F7 — 217 个模块级设计常量。**
runtime crate 里 `const NAME: f32/u16/u64 = <数字>` 共 217 个（`ROW_HEIGHT`、`SWATCH_SIZE`、`SHORTCUT_TEXT_SIZE`、`DOCK_TITLE_BAR_HEIGHT`…）。把数字从行内搬进私有 `const` 不等于搬进 Theme。

**F8 — Renderer 已经基本不懂 Theme。**（本轮补完最后一个缺口，见 §1.5）
`nana-ui-scene` 整个 crate 只有 3 处 `SemanticColorRole`，全在 tests / benchmark bin；`nana-ui` 的 scene_paint 只有 1 处，也在 tests。颜色是以解析好的 `[f32; 4]` 随 `ExtractedNode` 到达的。
→ #100「Renderer != Theme resolver」这条边界**已经成立**，唯一的缺口是 scene 读 `UI_METRICS`（F1）——本轮已经补上，scene 现在一处都不读。

**F9 — `Colors` 是 `SemanticPalette` 的同构副本。**（本轮已解决，见 §1.5）
`nana_ui::theme::Colors` 有和 `SemanticPalette` 完全一致的 24 个字段，加上双向转换。它是历史适配层，不是第二套设计权威，但确实是「同一设计语义两处定义」。

**F10 — 滚动条有自己的 metrics 类型。**（本轮已解决，见 §1.5）
`ScrollbarMetrics` / `SCROLLBAR_METRICS`（thickness 12、thumb 6、min length 24、inset 2、page 0.9）住在 `nana-ui-core/scrollbar.rs`，和 `ThemeMetrics` 平级。CSS `::-webkit-scrollbar` 覆写也接在它上面。密度调节今天改不动滚动条。
→ Phase 1 把它并进 control metrics，不要在 `ThemeDefinition` 之外留下第三份。

**F11 — `SettingsCollapsibleCard` 声明的无障碍角色没到节点上。**
`SettingsCollapsibleCard::project` 写的是 `role: AccessibilityRole::Button` 与 `disabled: self.disabled`，但卡片节点上读到的始终是 `role=Generic disabled=false`——包括本轮之前就存在的 `expanded` / `collapsed` fixture。这是语义基线带出来的：像素快照看不见 role，把 a11y 一起记下来才会露出这种「声明了没落地」。
→ 不在 #101 范围内动它：改无障碍角色是产品决定，会牵动 a11y 验证。先记下来。这也是 `settings-collapsible-card` 的 disabled / focused 两个 fixture 补不了的直接原因（见 §3.3）。

**F12 — 样式路径上最大的一笔堆开销，计数器看不见。**（本轮已修，见 §1.5）
`style_allocated_bytes` 把「每节点 208 字节」报得很准，却漏掉了同一条路径上每节点 4808 字节的 `LayoutStyle` 拷贝——1k 控件那一行少报 23 倍。原因不是忘了记，是**记不到**：拷贝发生在 `Arc::make_mut` 里，独占时原地复用、共享时克隆，两种都不经过任何分配器计数器。
→ 这是 Phase 0 该交的东西：一份用瞎计数器录出来的基线，会让后面每一次「看起来没变」的判断都失效。已补 `layout_copies` / `layout_copied_bytes` 与两条必需不变量。**更一般的教训**：#100 每引入一个新的解析点，都要问「这个点的开销哪个 counter 会看见」，答案是「没有」时就补 counter，而不是等它在别处以时间的形式冒出来。

## 1.5 本轮做掉的收敛

审计给出结论之后就地做了六件事。**默认主题下渲染逐字节不变**——变的是机制，不是像素。

证据有两层：语义基线 146/146 MATCH；以及在同一台机器上、用同一个临时 adapter key 把全套 615 张像素在收敛前后各录一次，`diff -rq` 为 **0 个文件不同**。一个「让主题真正生效」的改动，在默认主题下本来就应该什么都不改。

### 一份调色板，不是两份（F9 已解决）

`nana_ui::theme::Colors` 删掉了。它有和 `SemanticPalette` 完全一样的 24 个字段和一对来回转换；`ThemeTokens` 现在直接装 `SemanticPalette`，`ThemeModeExt::colors()` 去掉（已有 `palette()`）。一种设计语言不需要两种拼写，第二种拼写正是两者开始漂移的地方。

消费方改动：`Colors` → `SemanticPalette`，`theme.colors()` → `theme.palette()`，`tokens.colors` → `tokens.palette`。

### 滚动条并入主题（F10 已解决）

`ThemeMetrics` 现在**组合**（不是摊平）一个 `scrollbar: ScrollbarMetrics`。五个滚动条形状的数字不该躺在 `control_height` 旁边，但它们确实属于**安装的**主题。`world/geometry.rs` 的那一行 base 从常量换成 `self.style_model.metrics.scrollbar`。

`#[serde(default)]`：本字段之前写下的 settings blob 照样能读。

钉住它的测试：`a_thicker_scrollbar_in_the_installed_theme_reaches_the_bar`。把那行换回常量，它报 `12 -> 12` 然后红。

### 圆角说档位，不说像素（F1 部分解决）

这是本轮的结构性改动。

`NodeStyle` 新增 `radius: Option<RadiusTier>`，和它上面两行的 `background: Option<SemanticColorRole>` **形状完全一样**：组件说出设计档位，主题决定数值。

```text
以前： Button::new()  ──► layout.border_radius = Some(6.0)   // 构造期花掉了 token
       set_style_tokens(radius_sm = 2.0)                      // 追不上，已经是数字了

现在： Button::new()  ──► style.radius = Some(RadiusTier::Sm) // 只说档位
       write_node_style ──► resolved_layout.border_radius = tier.resolve(installed.metrics)
```

**解析点定在写入，不在 extract——这一条是被性能数据推翻后改的。** 最初按颜色的先例放在 extract（下游解析，同一套失效），写完发现 `LayoutStyle` 是 **4808 字节**，而 extract 每帧每控件都要 `Arc::make_mut` 它一次。实测：palette-switch **+196%**、density **+240%**、controls-1k **+101%**。颜色能在 extract 解析是因为一个颜色是 16 字节；圆角落在一个 4.8 KB 的盒子里，同一条路就不成立了。

改成 `NodeRecord.resolved_layout`：写入节点样式时解析一次，读的人直接拿。代价从**每帧**挪到**每次变化**，而主题变化是罕见事件。代价是每个有意图的节点多持有一个 `Arc<LayoutStyle>`——见下面「这次收敛的真实代价」。

单一写入口 `write_node_style` 保证 authored style 与 resolved layout 不会各走各的。这不是小心翼翼，是补窟窿：改完之后有 3 个 EmptyState 测试红了，因为有两处写入（`world/text.rs`、`world/motion.rs`）绕过了 `SetStyle`。

`NodeStyle::radius_px()` 保留为一次性显式覆盖——#100 明说不删除 explicit local style 的能力。测试同时钉住两边：档位跟随主题，px 不跟随。

已迁移 25 处（Button / TextInput / TextArea / IconButton / ListItem / Select / Switch / Chip / Menu 行 / Popover / XYPad / ColorField / PathField / HoverCard / GraphMinimap / SidebarRow / SidebarSection / SidebarFooterButton 等）。`color_field.rs`、`hover_card.rs`、`path_field.rs`、`thumbnail.rs`、`menus.rs`、`select.rs`、`xy_pad.rs`、`media_transport.rs` 已经完全不再 import `UI_METRICS`——那是 token 真的离开组件的信号。

钉住它的测试：`an_installed_radius_reaches_a_control_that_named_the_tier`。

### Renderer 不再读常量（F1 的 scene 侧，已完成）

余下 20 处 scene 读取全部是 `corner_radii(UI_METRICS.radius_*)`，画的是节点**没有**声明圆角的框架 chrome：菜单面板、模态框、命令面板行、焦点板、日历格、图片查看器。

`ExtractedNode` 新增 `chrome_radii: ChromeRadii`——已解析的四档圆角，和它上面一行的 `standard_visual_foreground: Option<[f32; 4]>` 是同一个先例：主题决定的值，运到 renderer，免得 renderer 自己去找。

```text
以前： scene/primitives ──► UI_METRICS.radius_md     // renderer 查常量
现在： extract_node     ──► chrome_radii = metrics.into()
       scene/primitives ──► node.chrome_radii.md     // renderer 只消费
```

**为什么搭 `ExtractedNode` 而不是给 scene 一个帧级 theme 句柄**：metrics 变化会把所有节点标 RENDER → 全部重新 extract → primitive 重建。搭节点走，失效契约已经对了；给 scene 一个独立句柄则要自己保证顺序与失效，而且那更接近「给 renderer 一份主题」。

结果：**`nana-ui-scene` 现在一处都不读 `UI_METRICS`**（原 21 处），连 import 都删了。#100 的「Renderer != Theme resolver」这条边界从「基本成立」变成「完全成立」。

在绘制时挑哪一档仍然是留在 renderer 里的设计决定——那是 #100 §3 Component Recipe 要吸收的部分。本轮先做到「renderer 只消费不解析」。

钉住它的测试：`a_menu_surface_paints_the_radius_it_was_handed_not_the_constant`（scene 侧）与 `an_installed_radius_reaches_a_control_that_named_the_tier` 的 `chrome_radii` 断言（端到端）。

### 控件高度说档位，不说像素（方案 A，本轮已实现）

§1.6 比完之后按 A 做的第一波。`NodeStyle` 新增 `control_height: Option<ControlHeight>`，`ControlHeight::Min(ControlSize)` / `Exact(ControlSize)` 区分「最小高度」和「钉死高度」——这个区分以前藏在调用点写的是 `min_height` 还是 `height` 里。

`control_layout()` 不再写 `min_height`；`Button` / `TextInput` / `ListItem` 的 `size()` 改成设置意图而不是算像素。

**语义基线在这里抓到一个真 bug**：构造函数里硬编码的 `ControlHeight::Min(Medium)` 会盖掉 `Button::size()`，Small 和 Large 的按钮都按 Medium 高度渲染。像素基线抓不到它（gallery 没拍非 Medium 尺寸的按钮），语义基线抓到了，因为它记的是「主题解析出什么」而不是「画成什么样」。

`.layout(custom)` 会清掉 `control_height`、`control_padding_x` 与 `radius`：替换整个 layout 的语义一向是「我自己接管」，意图要是活过它就会和显式 layout 打架（`nana-ui-scene` 有一个 overlay 测试正好踩中）。

### 水平 inset 说档位，不说像素（方案 A 第二波）

`ControlSize` 盖不住 text field 和 list row：它们各有独立的 metrics 字段，不是「medium 控件换一套 padding」。所以这一波没有把 inset 包进 `ControlSize`，而是做成和 `RadiusTier` 同形状的 `ControlPadding`：`Compact` / `Standard` / `Roomy` / `Field` / `ListItem`。

`control_layout()` 不再写 `padding_left` / `padding_right`。`Button` / `TextInput` / `ListItem` / `IconButton` / `TableCell` / `Chip` / `Select` / `Switch` / `RangeField` / 菜单行在节点上命名档位，`write_node_style` 解析。`used_layout_padding` 的回退也改读 `resolved_layout`——否则 extract 在尚未 layout 时会把意图节点的 inset 报成 0，场景拿到的是花掉的数字而不是安装值。

几何计算（菜单文字起点、gutter `max`、pill bleed 的负 margin）继续用安装的 metrics，不新开意图字段。多行 `TextArea` 的 **垂直** inset 仍读 `UI_METRICS.field_padding_x`：`ControlPadding` 只写左右。

钉住它的测试：`an_installed_control_padding_reaches_a_control_that_named_its_inset`。Button / TextInput / ListItem 三个档位各自跟随安装值；把 `Field` 并进 `Standard` 会让这一条红。

### 面板、方盒、垂直 inset、Large padding（方案 A 第三波）

同一条路继续走完还开着的尺寸接口：

- `SurfacePadding::{Panel, PanelX}`：卡片 / 设置组 / 表单表面 / pane 页签。`Panel` 写四边，`PanelX` 只写左右——页签不能吃进垂直 panel padding，否则会变高。
- `SquareSize::{IconButton, Control(size)}`：icon button 默认走 `icon_button_size`；`size()` 才换成 ControlSize。显式边长（sidebar 20px 工具）会清掉 `square`，避免意图把花掉的盒子写回去。
- `control_padding_y`：多行 TextArea 用 `Field` 做块轴 inset，单行 field 不写，避免撑开行盒。
- `ThemeMetrics.large_control_padding_x`：`serde(default)` 仍是 `space::XXL`，旧 settings blob 能读；Large / Roomy 终于能被安装主题移动。
- 工作区主区圆角改成 `RadiusTier::Lg`；L1 card 缺 CSS radius 时命名 `Md`，不再花 `UI_METRICS.radius_md`。
- extract / 菜单 / Select / TreeView / ReorderList 几何改读 `height_in` / `padding_x_in` 对安装值。

钉住它们的测试：`an_installed_panel_padding_reaches_a_card_that_named_the_surface`、`an_installed_icon_button_size_reaches_the_square`、`an_installed_field_padding_reaches_a_textarea_block_inset`、`an_installed_roomy_padding_is_no_longer_a_spacing_constant`。

### 这次收敛的真实代价

「authored intent 一份、resolved value 一份」不是免费的：有意图的节点会持有两个 `Arc<LayoutStyle>`，每个 4808 字节。

一次 density 变化（1000 个控件）要重解析 **1000 个盒子 / 4.59 MB**。这是**故意**的一次性代价，换掉的是每帧代价。因果是直接量过的，不是推断：把拷贝关掉重测，density 从 2.306 ms 回到 1.239 ms（基线 1.239）。palette-switch 与 accent-only 拷贝 **0 个盒子**，实测也确实回到/低于基线——最初那个 3× 回归消失了。

**这里要记一条方法教训**：这个代价 `style_allocated_bytes` **完全没看见**，少报 23 倍。因为它发生在 `Arc::make_mut` 里——独占时原地复用、共享时克隆，两种都不经过任何分配器计数器。一个看不见路径上最大一笔堆事件的计数器，会让基线读起来很健康。已补 `ThemeWorkCounters.layout_copies` / `layout_copied_bytes` 单列（不并进 `style_allocations`：那个字段有精确语义和建立在它上面的不变量，掺进去会让既有断言变成谎话），并给 palette-switch / accent-only 加了 `does_not_copy_layout` 必需不变量——**这正是当初那个回归会触发而其它计数器都不会触发的那一条**：在读路径上解析尺寸时，所有失效计数器都照样是 0。

### 还没收敛的（F1 剩余）

`UI_METRICS` 读取行 116 → 62（产品代码，排除 tests 与 bin）。剩下的分三类，各有各的阻塞原因：

| 剩余 | 为什么还没做 |
| --- | --- |
| 运动 token（F2） | ~~`motion_*_ms` 仍是死字段~~ → **§7 做掉了**：八个 duration 各归一个 role，`motion` 的 `const` 反过来读主题 |

剩余字面量已经接到 `space` / `type_scale` / `ControlSize` 上（命令面板行高 = Large + `space::XS` = 40，色板/标题 loading 宽 = `PAGE_TIGHT + XXS` = 22，sidebar 工具边 = `PAGE_TIGHT`）。对不齐的档位按角色评估过，不是就近 1px 平移：

| 原值 | 决策 | 理由 |
| --- | --- | --- |
| 11px 字号 | **新增** `type_scale::HINT` | 紧凑 chrome 说明，和 `META` 12 成对（compact badge / section title / toast 描述） |
| 18px 字号 | **新增** `type_scale::TITLE` | 设置页标题；离 `HEADING` 16 和 `DISPLAY` 20 都是 2px，不是一档能吞的 |
| 10px 字号 | **接到** `HINT` | pane 操作、图/日历标注，和 11 不是两套设计，不另开 MICRO |
| 7px | **接到** `space::SM` | 2px 网格外（tooltip 水平内边距、toast 指示点、compact badge 水平） |
| 5px | **接到** `space::XS` | 同上（表单 Medium 间距、compact badge/校验 gap） |
| 3px | **接到** `space::XXS` | compact badge 垂直内边距，保持比 rest `XS` 更紧 |
| 1px 行距 | **接到** `space::XXS` | 发丝分隔；1px **描边**仍留在 `border_width`，不硬接到间距 |
| 34px pane chrome | **合成** Medium + `XXS` | 正好 32+2，不是新高度档 |
| 10px 快捷键 | **接到** `HINT` | 命令面板曾用 `META - XXS` 合成 10 |
| 8px 未保存点 | **接到** `space::MD` | `FileTab::DOT_SIZE`；标记不是 caption |
| 9px markdown 块距 | **组件参数** `BLOCK_GAP` | 单点、离网格 1px |
| 54px viewer 边 | **合成** `PAGE*2 + SM` | ImageViewer 表面内边距 |
| 设置行高 13 | **接到** `BODY` | 单行省略贴字号，不用 `LINE` 16 |
| 1px 描边 | **`HAIRLINE`** | 不是 spacing |
| 日历格子 11/3/42 | **组件参数** `CalendarHeatmap::CELL_*` | 热图几何 |
| 图端口 5 | **合成** `PORT_RADIUS + HAIRLINE` | 相对 idle 4 的 +1 |

测试调用 `height_in(UI_METRICS)`，不再走已删除的 `ControlSize::height()` / `padding_x()`。产品不用的无 metrics 重载（`ImageViewer::geometry()` 无参、`row_bounds()` 无 metrics）已去掉，调用点一律带安装 metrics。

**下一步的判断**：布局输入那一类的两个方案已经拿证据比过了，见 §1.6——结论是走意图下沉（方案 A）。尺寸侧能命名的档位已命名；还没接上的只有 #100 的 Motion。

## 1.6 布局输入怎么收敛：两个方案的对比

radius 走通之后，剩下的大头是 **control height / padding** 这类**布局输入**——它们走不了 extract 那条路。有两条路可选，这一节是拿证据做的对比，不是倾向。

### 共同前提

两条路都要求组件**不再在构造期把 token 花成像素**。谁也不免费。真正的差别在于「之后谁来解析」。

### 方案 A：意图留在节点上，下游解析

和 radius 同一形状：`NodeStyle` 带意图字段，读的人在拿到安装主题的地方解析。

- **钩子已经存在**：`UiWorld::effective_layout_style(&self, id)` 是布局读取节点 `LayoutStyle` 的唯一漏斗，而且它**本来就在派生值**（hidden、overlay 的 position/width/z-index），`self.style_model` 就在手边。
- **意图词汇大部分已经有了**。runtime 里 `.height()` 共 58 处，**全部**是 `ControlSize` 派生（36 `size.height()` + 11 `Small` + 10 `Medium` + 1 `Large`，没有一处是别的类型）；`.padding_x()` 22 处。**80 个调用点早就在说档位**，坏只坏在 `ControlSize::height()` 内部拿 `UI_METRICS` 解析——而 `height_in(metrics)` 已经存在。
- 需要新命名的只有 ~38 处裸 `UI_METRICS.<字段>`（panel_padding ×18、list_item_padding ×6、icon_button_size ×5、field_padding ×4…）。
- **失效已经是对的**：`apply_style_model` 在 metrics 变化时就标 LAYOUT，现成的 LAYOUT 通道会重跑 `effective_layout_style`。
- **每次主题变化的额外开销：0**。
- **已被验证**：radius 就是这条路，25 处，0 像素变化，2 个测试。

代价与未决：Scene 读的是 `ExtractedNode.source_style.layout`，**不**经过 `effective_layout_style`。所以解析要放进两边共用的一个 helper（extraction 已经有为 padding 打补丁的先例）。

### 方案 B：主题安装触发重新投影

- **机制已经存在**，我上一轮说「`project` 不是对象安全可调用」是**错的**：`ChildReprojectFn = fn(&mut AppContext, StableNodeId) -> Result<(), FrameworkError>` 在 stamp 时捕获，经 typed `update_component` 管道重投影类型擦除的 view。今天由 `wants_child_reproject` 按组件选入（6 个组件用了）。
- **实测代价**（1000 个 Button，release）：重投影 1000 个 view **0.86 ms**；而一次 metrics 安装本身（失效 1001 节点 + layout inputs + extract）**4.2–4.7 ms**。即 B 是在已有开销上 **+19%**。
- **决定性的一条**：那 0.86 ms 产出的 mutation 是 **0**（`style=0 render=0`）。因为 `effective_style(&self)` 仍然读 `UI_METRICS`——**重投影本身什么都不改**。所以 B **不能替代** A 的逐组件工作，它是在 A 的工作之上再加一套机制和一份每次开销。
- **B 能买到而 A 买不到的**：投影期基于 metrics 做**条件**判断（按密度换结构、换视觉）。**证据：今天一个都没有**——对 metric 值的 `if` 0 处，比较 0 处。唯一的反向映射 `ControlSize::nearest_text(px)` 是把字号映回档位，不依赖安装的 metrics。
- **风险**：`update_component` 会 clone 组件并重跑 `project`，后者发 `set_standard_visual` / `set_accessibility` / text（都有 diff，但会真的跑）。那些在 project 期探测保留期子树并记忆结果的组件（`wants_child_reproject` 存在的原因）会在每次主题变化时重跑那次探测。
- **边界**：它把一次主题变化变成**结构性事件**，与 #100 §7「按 dependency class 精确失效」相冲。

### 结论：选 A

按重要性排序：

1. **B 不替代 A，只是加在 A 上面。** 重投影产出 0 mutation，除非组件同时改成读安装 metrics——那就是 A 的逐组件工作。
2. **意图词汇已经有 80/118。** A 的大头不是发明词汇，是让既有的 `ControlSize` 在下游解析。
3. **A 的失效现成且零额外开销**；B 每次变化 +19% 换 0 mutation。
4. **B 唯一多出的能力今天没有用户。**
5. **A 在本仓库已经跑通过一次**（radius，0 像素变化）。

### 实现 A 之后：三件里第一件的答案和预期之外的那条

**「共用的解析点放哪」有答案了，而且不是当初列的两个选项。** 原本以为要在 `effective_layout_style` 与 extraction 之间共享一个 helper——前提是「读的时候解析」。实测推翻了这个前提（§1.5「这次收敛的真实代价」）：`LayoutStyle` 4808 字节，放在任何读路径上都会被每帧放大。最后是**写的时候解析一次**，存进 `NodeRecord.resolved_layout`，两个读者都直接拿，不需要共享 helper。

这也是对 §1.6 原文的一处修正：那一节把「下游解析」当成 A 的定义，**下游解析不等于读路径解析**。A 真正的内容是「组件不把 token 花成像素、意图活到拿得到安装主题的地方」，至于那个地方是读还是写，要看被解析的值有多大。颜色 16 字节，读时解析；盒子 4.8 KB，写时解析。

剩下两件仍未定：

- ~38 处非 `ControlSize` 的 metrics 怎么命名：并进 `ControlSize`，还是像 `RadiusTier` 那样各自成档。
- `ControlSize::height()` 这个「对常量解析」的便捷方法要不要弃用——留着就会有人不小心再走回常量。

### B 什么时候才是对的

当某个组件真的需要**按 metric 改变它投影出什么**（不同密度下换结构）。那时让**那一个组件**选入重投影——机制已经在了，按组件选入远好过把它变成全局行为。

## 2. 硬编码设计值清单与迁移矩阵

### 2.1 清单怎么来的

[`scripts/audit-theme-hardcoding.py`](../scripts/audit-theme-hardcoding.py) 按**组件**扫描：每个 `impl ComponentView for X` 的函数体算 X 自己的设计权威，剩下的算「组件体之外的共享权威」。它是文本清点不是 parser，宁可多数不少数。

```bash
python3 scripts/audit-theme-hardcoding.py --format markdown
python3 scripts/audit-theme-hardcoding.py --check docs/performance-data/theme-audit-2026-09-19/theme-hardcoding.json
```

`--check` 在任一「本地设计值」类别变多时退出 1。这是 #100 Phase 9「防止重新引入 hard-coded default style authority」的种子：现在它守的是不要变得更糟。

扫描单位是**组件拥有的全部 impl**：`impl ComponentView for X` 加上 `impl X`。只扫前者会漏掉大部分——`ListItem` 的整套 `InteractionStyle` 写在 `ListItem::new` 里，只看 trait body 会把它报成「不声明任何状态」。这个错误是审计初稿真犯过的，现在有测试钉住。

分类规则本身有测试（`scripts/tests/test_theme_hardcoding.py`，CI 的 `python3 -m unittest discover -s scripts/tests` 会跑）：注释/字符串/测试模块不算设计值，`border_radius: Some(6.0)` 算而 `let width = 0.0` 不算，读了 token 权威的行进分母而不进分子。没有这些，「数字涨了」只会教人去重录基线。

基线（2026-09-19，122 个组件）：

| 位置 | color_role | color_literal | design_number | motion | elevation |
| --- | ---: | ---: | ---: | ---: | ---: |
| 组件自己的 impl | 232 | 0 | 29 | 2 | 0 |
| 组件之外的共享权威 | 216 | 0 | 139 | 18 | 1 |
| 模块级设计常量 | — | — | 217 | — | — |

`color_role` 是**意图**，迁进 recipe；`design_number` / `motion` 是**要消失**的那部分。

读 `design_number` 时要知道它混了两类东西：真正可 token 化的（字号、字重、间距、圆角），和结构性开关（`border_width: Some(0.0)`、`opacity: 1.0`、`border_radius: Some(999.0)` 这种「关掉/胶囊」哨兵）。后者不会因为有了 `ThemeDefinition` 就消失，所以这个数字的目标不是 0。

本轮顺手清掉了其中 13 处「把 token 的值抄成字面量」的（`Some(500)`→`type_scale::MEDIUM`、`6.0`→`space::SM`、`13.0`→`type_scale::BODY` 等）：值完全相同，像素零变化，但从此是可 grep 的 token 读取而不是隐形数字。组件内的 `design_number` 因此从 42 降到 29。

### 2.2 迁移矩阵

「Current authority」列用缩写：**V**=`view_components.rs` 组件体，**X**=`world/extraction.rs`，**G**=`world/geometry.rs`，**S**=`nana-ui-scene` primitives，**C**=模块级 `const`，**M**=`nana_ui_core::motion` const。

| Component | Current authority | Tokenizable | Recipe needed | Motion needed | Layout impact |
| --- | --- | --- | --- | --- | --- |
| Button | V（20 roles, hover/pressed/focused）+ X + S(radius) | color ✓ / radius ✗(F1) / padding ✓ | **是**：variant×size×state 全矩阵 | hover color (M) | size→height/padding |
| IconButton | V（14 roles, 5 states）+ S | 同上 + icon size | **是** | hover color (M) | size→方形边长 |
| Input / TextArea | V（focus/hover border）+ X + G(文本区) + S | color ✓ / field padding ✓ / radius ✗ | **是**：含 invalid / read-only | focus ring | size→height、多行按内容 |
| Checkbox / Radio / Switch | V（6 states on Checkbox）+ S(indicator 几何) | indicator size / gap 在 `ControlSize`，读常量 | **是**：checked×hover×disabled | switch 拨动 | indicator 影响行高 |
| Slider / Range | V（disabled）+ G(track/thumb) + S | track/thumb 尺寸全在 S 的裸数字 | **是** | 无 | 固定高度 |
| Card / Surface | V（4 roles）+ X + S(elevation) | color ✓ / radius ✗ / elevation 仅 1 处枚举 | **是**：`CardKind` 已是 variant | 无 | padding |
| ListItem | X（无 `InteractionStyle`）+ S | selection/hover 色 ✓ | **是**：selection row 通用 recipe | 无 | `list_item_padding_x` |
| Tabs / Segmented | X + G + S | indicator 几何在 S | **是** | indicator 滑动 | 分段宽度 |
| Menu / ContextMenu | `menus.rs`(14 roles) + S(overlays.rs radius) | color ✓ / radius ✗ / padding ✗ | **是** | `MENU_OPACITY`/`MENU_POP` (M) | 菜单项高度 |
| Tooltip / Dialog / Popover | `TooltipConfig` 常量 + `popover.rs`(10 roles) + S | tooltip 的 padding/radius/font 是 `TooltipConfig` 上的 `const` | **是** | `OVERLAY_FADE` (M) | 浮层尺寸 |
| Scrollbar | `nana-ui-core/scrollbar.rs` 的 `SCROLLBAR_METRICS` + G + S | **独立 metrics 类型**，不在 `ThemeMetrics` 里 | **是** | 淡入淡出 | 影响内容宽度 |
| Window / Titlebar / Sidebar | `AppTitleBar` + `sidebar.rs` + `AppearanceSettings`(backdrop) | titlebar 已是独立角色；backdrop alpha 已是 policy | **是** | `SIDEBAR_COLLAPSE` (M) | titlebar 高度、侧栏宽度 |
| Tree / Table / List selection row | `tree.rs` / `virtual_table.rs` + X + S | selection 三态色 ✓ | **是**：与 ListItem 共用 | 无 | 行高 |

读法：**Tokenizable ✗** 的格子是 F1 的具体落点——那些值今天读常量，`ThemeDefinition` 装进去也不会生效。**Recipe needed 全为「是」**不是敷衍：#100 §3 点名的首批 recipe 就是这张表，Phase 0 的结论是没有一行可以跳过。

## 3. Component Gallery 基线

两套基线，都在 [`examples/component-gallery/snapshots/`](../examples/component-gallery/snapshots/)。

### 3.1 像素基线（已有）

`snapshots/<adapter-key>/component-migration/<component>/<dark|light>/<state>.png`：73 个组件 532 张，加上 shell / dock / titlebar 等整窗快照 82 张，提交树共 614 张，一次完整运行全部渲染。零容差，按 GPU adapter 分目录；规则见该目录的 [README](../examples/component-gallery/snapshots/README.md)——那篇现在还记着**一张基线对不上另一张基线时该怎么读**，比单张对不上committed更常见也更难发现。

```bash
cargo run --release -p component-gallery --bin ui-snapshots --features snapshots --locked
```

它的限制正是 #101 §3 要补的：只有录制它的那台机器能验。

### 3.2 语义基线（本轮新增）

`snapshots/semantic/component-migration/<component>/<dark|light>.txt`，146 个文件，**不按 adapter 分目录**——里面没有一个像素是光栅化出来的。

每个 fixture 记录：目标节点的 layout box、无障碍状态（role / disabled / checked / selected / mixed）、以及该帧全部 `ScenePrimitive` 按场景顺序的解析结果——背景、描边、边宽、四角圆角、阴影、文字颜色/字号/字重/行高/字距/斜体/下划线、opacity。

```bash
cargo run --release -p component-gallery --bin ui-snapshots --features snapshots --locked -- --semantic
cargo run --release -p component-gallery --bin ui-snapshots --features snapshots --locked -- --semantic --bless
```

它在 CI 里跑：`cargo test -p component-gallery --bin ui-snapshots --features snapshots` 这一步本来就是「adapter 无关的那一半」，语义基线正是那一半。像素门禁在任何 hosted runner 上都跳过，没有这条的话状态矩阵在 CI 里没人看。

**它依赖 shaping 可复现，不依赖光栅化。** 里面的几何（含文本框宽度）来自 `nana-text` 的排版，不来自 GPU；同一份捆绑字体 + 锁定的 shaper 在三个平台上给同一组数字——`crates/nana-text/corpus/golden/` 早就把精确到 `advance_px: 11.664` 的排版结果当跨平台 golden 提交并在 ubuntu/macos/windows 上验。所以某个平台单独红了，意思是 **shaping 变了**（换字体、换 shaper 版本、换 feature 组合），不是「浮点噪声」，不要靠 bless 压过去。

一行长这样：

```text
state: primary
  target: 20.00,20.00 81.67x32.00
  a11y: role=Button disabled=false checked=None selected=None mixed=false
  quad #2/0 20.00,20.00 81.67x32.00 opacity=1.00 bg=#7bb9f024 border=none border_width=0.00 radius=6.00/6.00/6.00/6.00 shadow=none
  text #2/2 31.00,21.00 59.67x30.00 opacity=1.00 color=#7bb9f0ff size=13.00 weight=500 ...
```

这就是「Theme 解析成了什么」的可读形式：改一档 token，diff 直接说是哪个角色动了，而不是「15 张图 max_channel_delta=1」。

两套都留着：语义快照看不见光栅化 bug，像素快照说不出颜色**为什么**变。

### 3.3 状态矩阵覆盖情况

判据用组件自己的代码，不用印象：一个状态「应该有 fixture」，当且仅当组件在 `InteractionStyle` 里为它声明了 paint。审计初稿按家族目测，查出 32 个声明了却从未被捕获的状态；本轮补齐了其中 29 个。

补齐后（`--check` 的口径是「声明了的状态都有 fixture」）：

| 组件 | 本轮补的状态 |
| --- | --- |
| `checkbox` | selected-hover, selected-pressed |
| `chip` | hover, pressed |
| `dropdown` | hover, focused, disabled |
| `icon-button` | selected-hover, selected-pressed |
| `interactive-card` | hover, pressed, disabled, selected-hover, selected-pressed |
| `search-dropdown` | hover, focused, disabled |
| `settings-collapsible-card` | hover, pressed |
| `sidebar-row` | hover, pressed, focused, disabled, selected-hover, selected-pressed |
| `sidebar-section` | focused |
| `textarea` | hover |
| `xy-pad` | hover, focused |

29 个 fixture × light/dark = 58 张新像素键 + 58 段新语义基线。语义基线本轮已录并验；像素是新键（不是「像素变了」），2026-09-20 已补录。

**每一段都验过确实进入了状态**，方式是拿语义基线做逐字节比对：与该组件任何其他状态都不相同，或相同时能说清为什么。

这条比对当时只按家族抽查，漏掉了它本可以一次说清的事。后来把 615 张基线**全部按 md5 分组**再看，结论要大得多：**14 个有 `focused` fixture 的组件里，11 个把焦点渲染得和另一个状态逐字节相同**——`button` / `icon-button` / `sidebar-section` 与静息态一样（什么都没画），`dropdown` / `search-dropdown` / `xy-pad` 与 hover 一样，`tabs` 与选中一样，`sidebar-row` 写了 `border: Accent` 却配 `border_width: 0`。这不是 Phase 4 的待办，已经在本分支修掉：焦点数值收进 `AccentRamp.focus` 与三个语义角色，只在键盘焦点时出现，并由
`a_focused_fixture_never_looks_like_a_state_the_keyboard_did_not_cause` 守着（拿修之前的基线跑，它报 18 处碰撞）。

**方法本身比这条结论更值得留下**：把基线按内容分组，看哪些"不同状态"其实是同一张图。它不需要 adapter、不需要构建，一次读一遍文件，而本树里每一个真缺陷最后都是这么浮出来的。

剩下**没补**的，以及为什么：

| 组件 / 状态 | 为什么没补 |
| --- | --- |
| `settings-collapsible-card` 的 disabled / focused | 两者都在卡片的 summary 行上，卡片根节点的 a11y 仍报 `disabled=false`。按 fixture 现在的挂法拍不到这两个状态，拍出来会是一张「静止态冒充状态」的假基线 |
| `sidebar-section` 的 pressed | section 自己声明了 pressed，但它的 header 与 body 行都会先吃掉按下事件；这份 fixture 里没有一个落点能解析到 section 本身 |
| `CommandPalette` 的 selected | **已经被覆盖**：`open` fixture 的第一行就是 `bg=#353535`（`palette.selected`），只是 fixture 没叫这个名字 |
| `HoverCard` / `NumberInput` | 不在 `component_catalog()` 里，加 fixture 要先把它们变成一等目录组件——那是产品 API 改动，不是基线 |
| `Scrollbar` | 同上：`ScrollView` 不是目录组件，整个家族没有 fixture 目录 |

浮层与 chrome 家族（`tooltip` / `dialog` / `popover` / `context-menu` / `tree-view`）依然只有单态 fixture，但这不是缺口：它们在代码里就**不声明**交互状态，状态属于它们内部的行/按钮。

## 4. Theme / Style 性能基线

### 4.1 counters

新增 [`ThemeWorkCounters`](../crates/nana-ui-core/src/work.rs)，由 `UiWorld::last_theme_work_counters()` 发布。它不是 `WorkCounters`：后者的 `style_processed` 只说「这次 drain 排了多少个节点」，Theme 基线要的是那次 pass **里面**发生了什么。

| counter | 含义 |
| --- | --- |
| `style_nodes_considered` | pass 真正求值过的节点（含被跳过的）。父链上的节点每次 pass 只算一次 |
| `style_nodes_resolved` | 其中发布了新解析样式的 |
| `style_nodes_skipped` | 其中值与主题代都没变、走快路径返回的 |
| `theme_reads` | 向 token 权威（`StyleModelRef`）发起的读取次数。一次 mix 算一次 |
| `style_allocations` / `style_allocated_bytes` | style 路径自己造成的堆事件：每个发布的 resolved style 一次、每次主题安装的失效列表一次 |
| `layout_copies` / `layout_copied_bytes` | 为了让「已解析的值」和「组件写下的 layout」并存而拷贝的 `LayoutStyle`。**单列，不并进 `style_allocations`**：那是不同事件、不同成因、大一个数量级，而且任何分配器计数器都看不见它（发生在 `Arc::make_mut` 里）。只动颜色的主题变化必须为 0 |
| `layout_nodes_from_style` | theme/style 权威判定需要重新布局的节点 |
| `text_nodes_from_style` | 解析样式变化导致 shaping 或行约束失效的节点（**纯颜色变化不计**，否则 palette 切换会读成 reshape） |
| `paint_nodes_from_style` | theme/style 权威判定需要重绘的节点 |

不变式：`considered == resolved + skipped`（extractor 会校验，不成立直接拒绝报告）。

一次主题变化不止一个事件（安装 → drain → style pass），所以要在 `begin_frame_counters` / `end_frame_counters` 之间量；框外每个事件会覆盖上一个，与 `last_text_work_counters` 行为一致。

### 4.2 workload 与基线数字

```bash
cargo run --release --locked -p nana-ui-runtime --features benchmark --bin nana-theme-benchmark -- --output target/performance/issue101/theme.json
python3 perf/runners/nana/run.py --scenario theme-palette-switch --from-report target/performance/issue101/theme.json
```

`perf/scenarios/theme-*.json` 共 11 行，在 `catalog.json` 的 `nana_theme_ids` 里，**不在** `harness_ids`——判据是 work count，不是公共 CI 的时序。

extractor 与这些门禁本身由 `perf/contract.py --self-test` 的 `theme_baseline_tests` 覆盖（CI 已经在跑）：它用合成报告逐条证明每个门禁能单独变红，并拒绝 workload 不匹配、scale 不回显、`considered != resolved + skipped`、counter 缺失的报告。这些负例是真实报告测不到的。

基线（release，1k 控件 = 1000 个产品 `Button` + 一个列容器）：

| id | nodes | considered | resolved | skipped | theme_reads | style_alloc | layout_copies | layout_from | text_from | paint_from | p50 ms |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `theme-controls-1` | 2 | 2 | 2 | 0 | 2 | 2 | 0 | 0 | 1 | 0 | 0.008 |
| `theme-controls-100` | 101 | 101 | 101 | 0 | 101 | 101 | 0 | 0 | 100 | 0 | 0.377 |
| `theme-controls-1k` | 1001 | 1001 | 1001 | 0 | 1001 | 1001 | 0 | 0 | 1000 | 0 | 4.311 |
| `theme-controls-10k` | 10001 | 10001 | 10001 | 0 | 10001 | 10001 | 0 | 0 | 10000 | 0 | 66.345 |
| `theme-static-idle` | 1001 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0.000 |
| `theme-hover-one` | 1001 | 2 | 0 | 2 | 6 | 0 | 0 | 0 | 0 | 1 | 0.004 |
| `theme-focus-one` | 1001 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0 | 0.004 |
| `theme-palette-switch` | 1001 | 0 | 0 | 0 | 1001 | 1 | 0 | 0 | 0 | 1001 | 1.316 |
| `theme-accent-only` | 1001 | 0 | 0 | 0 | 1001 | 1 | 0 | 0 | 0 | 1001 | 0.817 |
| `theme-density` | 1001 | 0 | 0 | 0 | 1001 | 1 | 1000 | 1001 | 0 | 1001 | 2.681 |
| `theme-head-style-mutation` | 10000 | 10000 | 10000 | 0 | 1 | 1 | 0 | 0 | 0 | 0 | 14.154 |

**怎么读 `p50 ms`：只能同场 A/B，不能跨次录制比。** counter 列是确定性的——折进这份存档的 4 次运行逐字段完全相同，换台机器也一样。`elapsed_ms` 不是：同一个二进制在这台机器上，`theme-head-style-mutation` 因为旁边在不在编译，能在 7.7 ms 到 14.2 ms 之间摆动，接近 2×。表里的值是 4 次里的逐行最小值，仅用于量级参考。任何「变快/变慢了」的结论都要在一次坐下的时间里 A/B 出来，本文所有性能结论都是这么得到的。

原始报告存档在 [`docs/performance-data/theme-audit-2026-09-19/`](performance-data/theme-audit-2026-09-19/)。

### 4.3 数字说明了什么

- **static idle 全零。** 稳定帧不 walk、不读 token、不分配。#100 §15 的 zero-work fast path 这一条**已经成立**。
- **palette-only / accent-only：`layout_from = 0`、`text_from = 0`，`paint_from = 1001`。** 颜色切换不触发布局、不 reshape，这是对的；全文档 paint 失效是 F4，是 #100 §7 要收窄的数字。
- **density：`layout_from = 1001`、`layout_copies = 1000`。** metrics 变化正确地升级成 LAYOUT，并且**这条不再是半真的**：1000 次盒子重解析就是那 1000 个控件的高度真的跟着安装的 metrics 走了（本轮之前它们的尺寸在构造期就花成了像素，失效跑完尺寸照旧）。代价也在同一个数字里——4.59 MB，一次性，换掉每帧代价。
- **palette-only / accent-only：`layout_copies = 0`。** 只动颜色不许碰盒子。这条现在是必需不变量，而且它是**唯一**能抓到「在读路径上解析尺寸」的那条：那个写法会把每帧每控件的 `LayoutStyle` 重建一遍，而上面所有失效计数器照样全是 0。
- **`considered = 0` 而 `theme_reads = 1001`。** 这一对数字就是 F3 的指纹：没有 style pass，token 却被读了 1001 次——发生在 extract 里。
- **hover one：`considered = 2`、`resolved = 0`。** 只走了目标节点与其父；两个都走快路径。`resolved = 0` 是因为 hover 过渡从 progress 0 起步，第一帧的解析值与原值相同，颜色由后续动画帧推进。
- **focus one：全零。** 焦点环今天完全不经过 theme/style 路径，由 extraction/scene 直接画。这解释了 F6。
- **control scale 线性。** 1/100/1k/10k 的 considered、theme_reads、style_allocations 都等于节点数：首次解析一个文档 = 一遍。`text_from` 等于控件数是首次解析把字体从默认值改成控件值。
- **head scope：10000 considered、`theme_reads = 1`。** 头部一个继承色变化重解析整棵子树，但只问了主题一次——继承缓存是有效的。这是 ThemeScope 存在之前，一次 scope patch 的真实代价。

## 5. 边界

#100 的约束，逐条对照现状。

```text
Tokens + Semantics + Layout = existing Style Model    ← 成立
ThemeDefinition = Tokens/recipes 的设计系统 authority  ← 未建立（Phase 1）    → §7 已建立
Theme Resolver  = retained StyleSystem 工作            ← 未成立：解析在 extract（F3）
ResolvedStyle   = Layout/Text/Paint 下游合同           ← 部分：ComputedStyle 只覆盖继承与颜色
Renderer       != Theme resolver                       ← 成立（F8），除 scene 读 UI_METRICS（F1）
Motion         != Theme runtime                        ← 成立：Motion IR 是唯一执行器；但 Theme 也没有 transition policy（F2） → §7 补上了 policy，执行器仍只有一个
CSSOM          != Nana core                            ← 成立：CSS 解析只在 nana-ui-vue
```

各层职责，本轮确认的说法：

| 层 | 负责 | 不负责 |
| --- | --- | --- |
| **Theme** | 设计目标：token 值、component recipe、transition policy | 逐帧求值、动画时钟、布局算法 |
| **Semantics** | 组件意图：kind / variant / size / status / 交互状态 | 具体 RGB、具体像素 |
| **Layout** | 盒模型与几何：flex/gap/padding/尺寸 | 颜色、状态分层 |
| **Motion** | 从当前呈现状态过渡到目标状态（#87 Motion IR） | 目标状态是什么 |
| **Text** | shaping 与行布局（#88 nana-text） | 文字颜色（那是 Paint） |
| **Renderer** | 消费已解析数据画出来 | 理解 token 名、recipe、scope |

两条容易被违反的细则，本轮明确：

1. **颜色变化不是文本工作。** `text_nodes_from_style` 只在 `TextDirty::work()` 含 SHAPE 或 LAYOUT 时计数；颜色走 `TextWork::SCENE_PAINT`。这个分类由 `TextDirty::work()` 单点回答，不在 style 路径复制第二份映射。
2. **`SemanticPalette` / `ThemeMetrics` 是演进基础，不是复制对象。** Phase 1 不得出现 `ThemeColors2` / `ThemeMetrics2`。反过来，`nana_ui::theme::Colors`（F9）是**已经存在**的第二份，Phase 1 要收掉它，而不是再加一份。（§7 的 `DesignTokens` 按值持有这两个类型本身，没有第三个拼写。）

## 6. 复现

```bash
# 类型与数据流（这篇文档的 F1 / F8 数字）
rg -c "\bUI_METRICS\b" -g '!*tests.rs' -g '!*/bin/*' crates/nana-ui-runtime/src crates/nana-ui-scene/src crates/nana-ui/src
rg -c "style_model\.metrics|theme_metrics\(\)" -g '!*tests.rs' -g '!*/bin/*' crates/nana-ui-runtime/src crates/nana-ui-scene/src crates/nana-ui/src
rg -c "SemanticColorRole::" crates/nana-ui-scene/src crates/nana-ui/src/scene_paint*

# 硬编码清单
python3 scripts/audit-theme-hardcoding.py --format markdown
python3 scripts/audit-theme-hardcoding.py --check docs/performance-data/theme-audit-2026-09-19/theme-hardcoding.json

# Gallery 基线
cargo run --release -p component-gallery --bin ui-snapshots --features snapshots --locked -- --semantic
cargo run --release -p component-gallery --bin ui-snapshots --features snapshots --locked   # 需要录过基线的 adapter

# 性能基线
cargo run --release --locked -p nana-ui-runtime --features benchmark --bin nana-theme-benchmark -- --output target/performance/issue101/theme.json
python3 perf/contract.py --self-test
for id in theme-static-idle theme-controls-1k theme-palette-switch theme-density theme-head-style-mutation; do
  python3 perf/runners/nana/run.py --scenario "$id" --from-report target/performance/issue101/theme.json
done

# counters 行为
cargo test -p nana-ui-core --lib work::
cargo test -p nana-ui-runtime --lib world::tests::

# §1.5 的收敛：安装的主题真的到达控件与 chrome
cargo test -p nana-ui-runtime --lib an_installed_radius_reaches_a_control_that_named_the_tier
cargo test -p nana-ui-runtime --lib a_thicker_scrollbar_in_the_installed_theme_reaches_the_bar
cargo test -p nana-ui-scene --lib a_menu_surface_paints_the_radius_it_was_handed_not_the_constant

# 意图解析的代价被计数器看见（F12）
cargo test -p nana-ui-runtime --lib resolving_layout_intent_reports_the_layout_it_had_to_copy
# 解析点决策所依赖的那个尺寸前提
cargo test -p nana-ui-runtime --test layout_style_size

# renderer 不再读主题常量（应为 0）
rg -c "UI_METRICS" -g '!*tests.rs' crates/nana-ui-scene/src

# 门禁与分类规则本身
python3 -m unittest discover -s scripts/tests
cargo test -p component-gallery --bin ui-snapshots --features snapshots --locked
```

本轮在 Windows + Vulkan (RTX 5060) 上验的：counters 与 scenario 门禁全绿、语义基线 146/146 MATCH、像素套件在临时 adapter key 下 557/557 录制并复验通过（提交树里的 `metal-apple-m4` 基线**不能**在这台机器上验，这正是语义基线存在的理由）。

## 7. Phase 1：ThemeDefinition 与 typed token 体系

这一节是 [#102](https://github.com/sena-nana/NanaUI/issues/102) 的交付物。Phase 0 的结论是「token 权威散在常量里，装了主题也不生效」；Phase 1 建立那个**被装的东西**：一个版本化、强类型、可校验的 `ThemeDefinition`。

**同样没有改任何一个已有像素。** 证据不是「跑了一遍快照觉得没事」，而是三条各自独立的：

1. **语义基线逐字节比对。** 在同一台机器上、用同一个二进制路径，分别对干净 HEAD 和本轮各录一次全部 146 个 fixture（666 个输出文件，含 baseline 副本），`diff -rq` 为 **0 个文件不同**。
2. **编译结果等值断言。** `the_built_in_definitions_compile_to_exactly_the_tokens_already_rendered` 断言 `ThemeDefinition::NANA_DARK.compile()` 产出的 `SemanticPalette` 与 `ThemeMetrics` 和树上现在渲染用的**完全相等**。等值成立时，任何 fixture 都没有可动的余地——一次需要重录 615 张图才能证明自己安全的迁移，等于没有证明。
3. **work counter 逐字段比对。** 11 个 theme 场景对 Phase 0 存档**全部 11 项 counter 逐字段相同**（见 §7.7）。

> ⚠️ **本轮之前语义基线就已经和 HEAD 对不上了。** 干净 HEAD 上跑 `--semantic` 有 **46 个 fixture 报 CHANGED**，`cargo test -p component-gallery --bin ui-snapshots` 因此在 main 上就是红的，与 #102 无关——上面第 1 条正是为了把这两件事分开才那样做。这 46 张已在随后一轮里逐条对因后重录（现 146/146 MATCH），其中 3 类查出来是回归而不是常量收敛；像素套件也在同一轮补齐到 615/615。

### 7.1 类型全景

```text
                     ThemeDefinition            ← 作者写的，可 diff
                     ├─ id / schema / generation
                     ├─ mode
                     ├─ tokens: DesignTokens
                     │  ├─ foundation: FoundationTokens  ← 作者私有，组件够不到
                     │  │  └─ accent: AccentRamp
                     │  ├─ palette:  SemanticPalette     ← 就是原来那个类型
                     │  ├─ metrics:  ThemeMetrics        ← 就是原来那个类型
                     │  ├─ spacing:  SpacingTokens
                     │  ├─ border:   BorderTokens
                     │  ├─ opacity:  OpacityTokens
                     │  └─ titlebar: Option<SemanticColor>
                     ├─ typography: TypographyTokens
                     ├─ motion:     MotionTokens
                     ├─ effects:    EffectTokens
                     ├─ surfaces:   SurfaceTokens
                     └─ components: ComponentThemeRegistry   ← 槽位是 Option
                                │
                                │  compile()  ← 校验 + 降级，失败就整个不装
                                ▼
                     CompiledTheme              ← 运行期读的，槽位不再是 Option
                     ├─ identity: ThemeIdentity
                     ├─ style_model: StyleModelRef   ← 热路径切片
                     ├─ spacing / border / typography / motion / effects / surfaces
                     └─ recipes: CompiledRecipes     ← 定长数组，enum 下标
                                │
                                ▼
                     UiWorld.theme: Arc<CompiledTheme>
                     UiWorld.style_model: StyleModelRef   ← 上面那个的缓存投影
```

`DesignTokens` 不叫 `ThemeTokens`：后者是 `nana_ui::theme::ThemeTokens`，宿主侧的安装包。**一个名字一个类型**，这是 F9 那条教训的直接后果。

### 7.2 为什么 authoring 和 compiled 是两个类型

两条理由，都不是洁癖：

**fail-closed。** `ComponentThemeRegistry` 的槽位是 `Option`，`CompiledRecipes` 的不是。作者漏掉一个 family，`compile()` 返回 `MissingRecipe { component, slot }`，整个主题装不上；而不是那一处悄悄退回 `Text`，等三屏之后被人发现某个角落颜色不对。校验覆盖：schema 不兼容、generation 为 0、非有限值、负长度、alpha 越界、字号越界、字重越界、时长为 0、recipe 槽位缺失。每条都有一个从「能编译的主题」出发只动一个字段的测试。

**热路径不认字符串。** 名字只活在 authoring 侧（`ThemeId`、错误里的 token 名）。跨进 runtime 的全是 enum 索引定长数组。`theme.get("button.primary.background")` 这种写法在类型上就不存在。

### 7.3 `StyleModelRef` 为什么没有变成主题句柄

`CompiledTheme` 约 1 KB，**不是 `Copy`**，挂在 `Arc` 上。`StyleModelRef` 还是那个小的、按值传的读句柄，本轮只给它加了 24 字节的 `OpacityTokens`（下面 §7.5 要用）。

这条是照抄 §1.5 的教训：`LayoutStyle` 4808 字节放在读路径上被每帧放大，palette-switch 实测 +196%。「把整套主题做成 `Copy` 交给每个读者」是同一个形状，只是这次 1 KB。所以 `CompiledTheme` 也**没有**预计算 39 个角色的颜色表：解析一个角色本来就是一次 `match` 加最多一次 alpha 替换，一张表要多背 ~600 字节，买不到东西。缓存要配得上它省掉的开销。

### 7.4 Foundation → Semantic：只在真的有派生的地方

#100 §2 要 foundation 层。本轮**只**给了 accent 一族，因为那是调色板里唯一真的在派生的地方：`accent_soft` / `accent_soft_hover` / `accent_soft_pressed` 就是 base accent 的三个 alpha，以前是三个字面 RGBA 常量。

后果是一个真 bug 消失了：以前只设 `palette.accent`（`theme-accent-only` 场景、`--nana-custom-accent`）会留下上一个色相的三个软填充——派生规则只存在于「当初谁选的那几个常量」里。现在 `ThemeDefinition::with_accent(ramp)` 一次移动整族。

反过来，`with_palette(p)` 仍然逐字写入 `p`，只是**顺带把 foundation 的 ramp 按 `p` 重读一遍**（`AccentRamp::from_palette`）——记录，不修正。一个 `accent_soft` 和 `accent` 色相不一致的调色板，读出来的 ramp 就照实说它不一致。

`base` 之外的四个颜色不是算出来的：能在 accent 上读清的前景是设计决定不是公式，light 模式就是证据（`on_soft` 比 `base` 深，`text` 是白的）。

组件够不到 foundation：从组件到 `FoundationTokens` 没有路径，这就是「不得依赖 `blue500`」的执行方式。

### 7.5 本轮接上的 token，和各自的消费方

**不接消费方的 token 就是下一个 F2。** 每一类都有真实读者：

| 类别 | 类型 | 谁在读 | 接上之前是什么 |
| --- | --- | --- | --- |
| Color | `SemanticPalette` | 全树 | 同左（直接持有，没有第二份） |
| Control metrics | `ThemeMetrics` | 全树 | 同左 |
| Spacing | `SpacingTokens` | `space::*` 读它的 `DEFAULT` | 10 个裸 `const` |
| Typography | `TypographyTokens` | `type_scale::*` / `UI_BASE_TEXT_SIZE` 读它的 `DEFAULT` | 13 个裸 `const` |
| Border | `BorderTokens` | `HAIRLINE` 读它的 `DEFAULT` | 1 个裸 `const` |
| Opacity | `OpacityTokens` | `StyleModelRef::color` 解析五个 soft 角色 | `SemanticPalette::get` 里的字面量 + `background.r > 0.5` 亮度嗅探 |
| Motion | `MotionTokens` | hover 交叉淡入、switch 拨动读**安装值**；`motion::*` 八个 `const` 读 `DEFAULT` | 八个裸 `const` + 两个死字段 |
| Effect / Elevation | `EffectTokens` | 菜单/浮层阴影、模态框阴影读**安装值** | `surface_shadow` 里的 `match mode` + 模态框的亮度嗅探 |
| Surface / material | `SurfaceTokens` | `ThemeTokens::with_backdrop` 决定 backdrop 给哪个角色上 alpha | `match target` 写死在宿主适配层 |
| Focus | `AccentRamp.focus` + `FocusSurface` / `FocusBorder` / `FocusText` | 11 个组件的 `InteractionStyle::focused`，以及 radio 焦点环的颜色 | 三个组件写 `border: Accent` 配零宽度边（画不出来）、三个与 hover 同值、一个与选中同值 |
| Component recipe | `ComponentThemeRegistry` | extraction 的 family 前景表、`Button` 的 variant×state 表、status tone 表 | 25 臂 `match StandardVisual` + `Button::project` 里五张内联表 |

方向很关键：**`const` 读 token，不是 token 读 `const`**。F2 之所以修不动，正是因为当时方向是反的——时长是权威，主题只挂着两个没人读的字段。

亮度嗅探消失了两处（warning soft alpha、模态框阴影）。它们本来就不是「省事」，是错的：背景是中灰的主题会选到反的那一支，而且任何主题都改不动那个数。

### 7.6 Component recipe：两张表，两种解析点

只搬了两张，都挑在**安装主题已经在手边**的地方解析：

**family 前景表**（`world/extraction.rs` 那 25 臂）→ `ComponentRecipe`。它每次 extract 解析，所以换了 recipe 的主题**不需要重投影**就能到达活节点。测试：`an_installed_family_recipe_reaches_a_live_node_without_reprojecting_it`。

**Button 的 variant×state 表**（`Button::project` 里五张内联 `match self.kind`）→ `ButtonRecipe`。这张在**投影期**解析，因为它决定的是组件往自己节点上写哪个 `SemanticColorRole`——那个字段只有组件自己写得了，下游谁都改不了。所以 `Button` 选入了一个新钩子 `wants_recipe_reproject()`。

这看起来和 §1.6「选 A，不选 B」矛盾，其实是 §1.6 自己留的口子：

> B 什么时候才是对的：当某个组件真的需要**按 metric 改变它投影出什么**。那时让**那一个组件**选入重投影。

§1.6 拒绝 B 的理由是「重投影产出 0 mutation，因为 `effective_style` 仍读 `UI_METRICS`」。recipe 让这条不再成立：重投影确实会写出不同的角色。而且钩子和 `wants_metrics_reproject` **分开**，不是合并——合并会让 palette-only 切换白白重投影一批组件，而那正是 §4 量到 0 次重投影的那个常见场景。两个测试各自钉住：去掉 `Button` 的选入，`an_installed_button_recipe_reaches_a_button_that_is_already_on_screen` 报红（验过）。

`invalid` 是本轮第一个**叠加态**而不是互斥态：invalid 的 primary 按钮保留 primary 的填充，另外加一道 danger 描边。#100 §4 要求 recipe 说清哪些状态叠加，这是第一条。

`ComponentRecipeId` 有 14 个 family，其中好几个今天都解析到 `Text`。它们仍各占一格：registry 存在的意义就是主题能单独移动一个 family 而不牵动其余。

### 7.7 代价

work counter 对 Phase 0 存档**逐字段相同**，11 个场景 × 11 项：

```bash
cargo run --release --locked -p nana-ui-runtime --features benchmark \
  --bin nana-theme-benchmark -- --output target/performance/issue102/theme.json
```

存档在 [`docs/performance-data/theme-phase1-2026-09-20/`](performance-data/theme-phase1-2026-09-20/)。要点：

- `theme-palette-switch` / `theme-accent-only` 的 `layout_copies` 仍是 **0**，`layout_nodes_from_style` 仍是 **0**。只动颜色不许碰盒子，这条没被新 token 破坏。
- 这两个场景也**不触发**任何重投影：recipe 没变，`reproject_recipe_views` 不跑。
- `theme-static-idle` 全零：稳定帧不 walk、不读 token、不分配。
- `theme-density` 的 `layout_copies = 1000` 照旧。

一次主题安装现在多做两件事：`compile()`（约五十个浮点的校验 + 建一个定长 recipe 数组）和 `*self.theme == *next` 的整体比较（约 1 KB）。相对同一次安装本身就要做的 1001 个节点失效，量级上不构成一行。

### 7.8 硬编码清单的变化

同一把尺子（`scripts/audit-theme-hardcoding.py`），对干净 HEAD 和本轮各跑一次：

| | HEAD | Phase 1 | 说明 |
| --- | ---: | ---: | --- |
| 组件自己的 `color_role` | 232 | **208** | Button 的 24 个角色决策搬进主题 |
| 组件之外的 `color_role` | 216 | **211** | extraction 的 family 表搬进主题 |
| 组件之外的 `design_number` | 87 | **81** | 两组阴影字面量 + 两处亮度嗅探 |
| 组件之外的 `motion` | 18 | **16** | hover / switch 改读安装值 |
| 组件之外的 `elevation` | 1 | **0** | |
| `token_read`（分母） | 681 | **683** | |

**扫描器本轮也改了两处，否则这张表会撒谎。** `TOKEN_READ` 加上了 `MotionRole::` / `ElevationRole::` / `ComponentRecipeId::` / `recipes()` 等新的命名方式——理由和当初加 `RadiusTier::` 一字不差：组件从 `motion::HOVER_COLOR` 换成 `MotionRole::HoverColor` 是在**读** token，不加的话分母会缩、迁移会被读成回归。`ELEVATION` 排除了 `ComponentElevation::from_shadow`，那是在消费主题解析好的阴影，跟「组件自己挑了个阴影」正相反；不排除的话这个数会随着迁移成功而上涨。两条都有测试（`scripts/tests/test_theme_hardcoding.py`）。

`--check` 的基线改指 Phase 1 存档：对着一份树上早就超过的数字设门禁，等于门禁有旷量。

### 7.9 没做的，和为什么

| 项 | 状态 |
| --- | --- |
| ThemeScope / 子树 override | #102 明确非范围 |
| 组件全量迁移到 recipe | #102 明确非范围。搬了 Button + family 前景表 + status tone 表，其余 11 个 family 的 recipe 只有前景一槽 |
| Theme package / 文件格式加载 | 非范围。`ThemeId` 因此是 `&'static str`；要从文件装主题时它得先变 |
| F3（解析点在 extract 不在保留期） | 未动。这是 #100 §6，要改的是 resolver 的形状，不是 token 的形状 |
| F4（安装 = 全文档失效） | 未动。这是 #100 §7 的 dependency class |
| typography / spacing 的调用点收敛 | 未做。合同建立了，但 `ControlSize::text_size()` 这类仍对 `type_scale` 常量解析——和 Phase 0 对 metrics 做的那一轮是同一形状的工作，只是换一个类别，留给 #107 |
| focus ring 的 2px 描边与 4px 外扩 | **几何**仍是 `nana-ui-scene` 里的字面量；颜色已经是 `palette.focus_border`。`BorderTokens` 只有 `hairline` 一档，没有替这两个尺寸发明档位——按 `ChromeRadii` 的先例搬运需要动 `ExtractedNode`，那是 chrome recipe 的活。实际是**五处、两种外扩**（3.0 与 4.0），不是文里写的一种 |

### 7.9.1 §7 之后补做的（consumer 驱动）

一个 L3 消费方（NanaLive）装完整 `ThemeDefinition` 时撞上的三件事，都是 §7 留下的洞而不是新需求，所以就地补了，没有等 #105 / #110：

| 补做 | 原来的样子 |
| --- | --- |
| **L3 re-export 面** | `ThemeDefinition` / `CompiledTheme` 导出了，它们**由之构成的 token 结构体一个都没有**。消费方能拿着一份定义调 `.compile()`，却没法构造或修改它：每个 `with_*` builder 收的类型都叫不出名字，`ThemeId` / `ThemeSchemaVersion` / `ThemeGeneration` 三个 identity 字段全够不到。`nana_ui::theme` 现在镜像 `nana_ui_core::lib` 的那一份清单，外加 `SemanticColorMix` / `PaintStyle` / `BoxShadowSpec`。 |
| **`ThemeMetrics::switch`** | switch 轨道的 30×16 与 8 的标签间距是 `world/geometry.rs` 里四个字面量，主题够不到；scene 画笔另有一份手抄的 `38.0`（= 30+8）算标签内缩，改轨道会静默失配。`SwitchMetrics` 按 `ScrollbarMetrics` 的先例组合进 `ThemeMetrics`。同时 `Switch::project` 无条件写 `control_padding_x`，盖掉调用方的内边距——这是唯一一个没法退出的 intent 字段，没有标签的 switch 因此只剩 2pt 内容盒、轨道画成一条缝。清掉 `control_padding_x` 也不算退出:它本来就是 `None`,那行会立刻写回去。所以判据换成**调用方是否已经在这条边上花了数字**——花了就说明它想要那个值,组件默认不该盖掉,这正是 `Button::layout` 对「调用方交出整个盒子」讲的同一条规则。 |
| **`SemanticPaint::foreground_secondary`** | 行内次要文字（detail、hint、placeholder、单位）由 `world/geometry.rs` 直接读 `palette.muted`，不看节点状态。聚焦行铺 `focus_surface`、标签跟到 `focus_text`，而那行灰字仍然是对着**原来**那层表面解出来的：浅色 1.09:1。单一 `muted` 取值无解——它同时要在白卡上够安静。所以状态自己说：`FOCUS_SURFACE` 一并给出 secondary，默认 `None` 仍解析成 `Muted`，静息渲染逐字节不变。 |

顺手把 `world/geometry.rs` 的 15 个 design number 收到 10，并把该文件 **全部** 直读 `palette.<field>` 改成走 `style_model.color(role)`——语义意图从此只有一条解析路径。落到已有档位、不发明新档位：key badge 28/8/12/64 → `ControlSize::Small` / `space::MD` / `type_scale::META` / `space::XXXL * 4`，chart strip 2 → `XXS`，ListItem 间距 8 与折叠下限 16 → `MD` / `XXXL`，Range 私有的 6/8/10 三档 → `SM`/`MD`/`LG`，Progress 6 → `space::SM` 且圆角改成 `girth / 2.0`（原本是与之相等却可以各走各的 `3.0`），LevelMeter 4 → `XS`，EmptyState 22 → `PAGE_TIGHT + XXS`，XYPad 轴线 1 → `HAIRLINE`，ReorderList 8 → `MD`。StatusBadge 与 ValidationMessage 各自那份 `indicator_slot * 10.0 / 24.0` 合并成一个具名 `STATUS_DOT_FRACTION`——24 是幻影分母，两份幻影分母就是两个 badge 开始漂移的方式。

剩在 `world/geometry.rs` 的 10 个都需要**新**的 token 类别，不属于就地补做：交互状态的 alpha（0.55 / 0.78 / 0.70 / 0.42 / 0.12）归 #105 的状态矩阵，`(size - 1.0).max(10.0)` 这类字阶降级与 `* 1.2` 的行高比例归 #107，TextInput 编辑器 chrome 那一簇归 #110。

全程 614 张像素基线与 146 条语义基线**零变化**——这一轮是取值不变的搬运，不是视觉改动。
| `sidebar-section` 焦点底色覆盖整个 200x86 区块 | 可聚焦的节点就是整个 section，所以这是如实渲染。要只高亮 header，得把可聚焦节点从 section 根移到 header——那是行为变更（焦点顺序、命中），不是指示器变更 |

### 7.10 复现

```bash
# 定义、校验、fail-closed、accent ramp、状态层 alpha
cargo test -p nana-ui-core --all-features --lib theme::

# 安装的 recipe / motion / elevation 真的到达运行期
cargo test -p nana-ui-runtime --all-features --lib an_installed_button_recipe
cargo test -p nana-ui-runtime --all-features --lib an_installed_family_recipe
cargo test -p nana-ui-runtime --all-features --lib an_installed_hover_duration
cargo test -p nana-ui-runtime --all-features --lib a_theme_that_fails_validation

# 硬编码清单与扫描规则
python3 scripts/audit-theme-hardcoding.py --check \
  docs/performance-data/theme-phase1-2026-09-20/theme-hardcoding.json
python3 -m unittest discover -s scripts/tests

# work counters（应与 Phase 0 存档逐字段相同）
cargo run --release --locked -p nana-ui-runtime --features benchmark \
  --bin nana-theme-benchmark -- --output target/performance/issue102/theme.json
for id in theme-static-idle theme-controls-1k theme-palette-switch theme-accent-only \
          theme-density theme-head-style-mutation; do
  python3 perf/runners/nana/run.py --scenario "$id" --from-report target/performance/issue102/theme.json
done

# 语义基线：本轮与干净 HEAD 逐字节相同（当时两边都有 46 个 pre-existing CHANGED，现已重录）
cargo run --release -p component-gallery --bin ui-snapshots --features snapshots --locked -- --semantic
```
