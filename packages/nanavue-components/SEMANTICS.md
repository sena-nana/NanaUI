# nanavue-components ↔ NanaUI 语义对照

Issue #5 — Vue **基础组件与布局原语**经 `MessageBridge` 落到 Nana 布局原语与 Runtime 控件（`nana-ui-runtime` / `UiScene`）。  
本包对应三层兼容中的 **L2**（可与 L1 HTML/CSS 降维同树混合）。见 [`docs/vue-nana-renderer-system.md`](../../docs/vue-nana-renderer-system.md) §0。

> **架构（变体 / 组合）**  
> 所有可见 UI 最终都应落到 **NanaUI 基础能力**（布局原语 + 基础控件及其变体）。  
> Vue「自造组件」= 用基础组件 **组合、变体（variant / props / slots / token）** 表达，  
> **不是** 另起 CPU 简化 paint。CustomContent **已移除**。  
> 裸 `div` / class / role 由 Rust 降维映射到 Nana 布局盒与控件变体（L1 路径）。

## 架构边界

| 层 | 策略 |
|----|------|
| Style Model | **Tokens + Semantics + Layout**；L1/L2/L3 同模型（见 `nana_ui_core::style_model`） |
| L2（本包） | 语义 props → Semantics / Tokens；**跳过 CSS**；`createWidget` / `nana-*` |
| L1（同树可混） | HTML·class·role·style → `css_map`（Layout）+ `widget_map`（Semantics） |
| L3 | Runtime / `UiScene` 保留与绘制合同；Iced（`nana-ui`）是当前兼容视图适配器（见 `docs/lilia-component-parity.md`） |
| 自定义 | **组合与逻辑**；不另起 paint 引擎。CustomContent **已移除** |
| Token | 主题档位 → `ThemeMetrics` / 语义色；`nana-controls.css` 禁止独立 `#3867ff`；任意业务 CSS 色值不得污染正式 token |

## 消息桥

| 方向 | 机制 |
|------|------|
| Vue → Rust | `createWidget` / `createElement`（含 HTML 降维）+ `patchProp` → `MessageBridge` |
| Rust → Scene | `semantic_snapshot()` → `nana_ui_vue::view_semantic_tree*`；有活跃 Scene 时合格叶子走 Runtime Scene（`component_uses_runtime`） |
| 宿主 → Vue | `BridgeEvent` → `VueHost::dispatch_bridge_event` → `__nanaFireEvent` |
| Theme（单向） | `VueHost::inject_theme` |

验收：`cargo run -p vue-counter -- counter --semantic --clicks=2`；
`cargo check -p vue-counter --features windowed`；
`cargo test -p nana-ui-vue --features iced-view --lib`。

## 降维映射（节选）

| 来源 | Nana `WidgetKind` | Runtime 类型 |
|------|-------------------|--------------|
| `nana-button` / `<button>` / role=button | Button | `nana_ui::Button` |
| `nana-chip` / class `nana-chip` | Chip | Button Selected/Subtle 变体（非独立 catalog 身份） |
| `nana-switch` / role=switch | Switch | `nana_ui::Switch` |
| `nana-checkbox` / `input[type=checkbox]` | Checkbox | `nana_ui::Checkbox` |
| `nana-input` / `<input>` | Input | `nana_ui::TextInput` |
| `nana-tabs` / role=tablist / class `nana-tabs` | Tabs | `nana_ui::Tabs` |
| `nana-segmented` / class `nana-segmented` | Segmented | `nana_ui::SegmentedControl` |
| `nana-range` / role=slider | Range | `nana_ui::RangeField` |
| `nana-sidebar-row` / class sidebar-row | SidebarRow | `nana_ui::SidebarRow` |
| `div` / `section` / `main` / … | Column / Row | Nana Column / Row 布局盒 |
| `#text` / `span` / `p` / `h*` | Text | `nana_ui::Text` |
| `li` | ListItem | `nana_ui::ListItem` |
| class `card` / `nana-card` | Card | `nana_ui::Card` |
| `nana-select` / `<select>` | Select | Runtime `Select` |
| `nana-dropdown` | Select (`nana-dropdown`) | Runtime `Dropdown` |
| `nana-search` | Select (`nana-search`) | Runtime `SearchDropdown` |
| `nana-textarea` / `<textarea>` | Textarea | Runtime `Textarea` + EditorStore |
| `nana-dialog` / role=dialog | Dialog | Runtime `Dialog`（`open`/`active`） |
| `nana-dialog` + role=alertdialog / class confirm | Dialog | Runtime `ConfirmDialog` |
| `nana-drawer` / sheet | Drawer | Runtime `Drawer`（`side`/`width`/`footer`） |
| `nana-popover` | Popover | Runtime `Popover` |
| `nana-context-menu` | ContextMenu | Runtime `ContextMenu` / ActionMenuItem 列表 / MenuStore |
| `nana-toast` | Toast | Runtime `Toast` |
| `nana-tooltip` | Tooltip | Runtime `Tooltip`（无 StandardVisual） |
| `nana-action-menu` | ActionMenu | Runtime `ActionMenu` |
| `nana-action-menu-item` | ActionMenuItem | Runtime `ActionMenuItem` |
| `nana-xy-pad` / `nana-xypad` / `xy-pad` | XYPad | Runtime `XYPad` |
| `nana-qr-code` / `nana-qr` / `qr-code` | QrCode | Runtime `QrCode`（有 modules）或 LabeledValue 占位 |
| `nana-form-field` / `nana-form` | FormField | Runtime `FormField`（子控件走 composer） |
| `nana-interactive-card` | InteractiveCard | Runtime `InteractiveCard`（内容子树走 composer） |
| `nana-skeleton` | Skeleton | Runtime `Skeleton` / Scene leaf |
| `nana-level-meter` / `nana-level` | LevelMeter | Runtime `LevelMeter` / Scene leaf |
| `nana-command-palette` | CommandPalette | Runtime `CommandPalette` |
| `nana-tree-view` | TreeView | Runtime `TreeView` |
| `nana-calendar` | CalendarHeatmap | Runtime `CalendarHeatmap`（`data` 单元格；`options` 对象为热图度量，数组仍作单元格回退；无法解释时投影空热图） |
| `nana-image-viewer` | ImageViewer | Runtime `ImageViewer`（仅 host texture id，不解码） |
| `nana-markdown` | NativeMarkdown | Runtime `NativeMarkdown::from_source`（`mermaid`/`mmd`、`math`/`latex`/`tex` 栅栏落到块种类；可选 `mermaidRenderer`/`mathRenderer` 仅为 `mermaid:{source}` / `math:{source}` 身份，不实现排版） |
| `nana-graph-canvas` | GraphCanvas | Runtime `GraphCanvas`（`nodes`/`edges`/`model`/`viewport`/`selection`；无法解释时投影空模型） |
| `nana-workspace` | Workspace | Runtime `Workspace`（`region` / `data-region` 子节点为 slots） |
| `nana-dock` | Dock | Runtime `Dock`（子节点或 `layout`/`root`；有子节点时不是 dummy `item("dock", None)`） |
| `nana-split-pane` | SplitPane | Runtime `SplitPane`（前两子节点 + `axis`；可选 handle） |
| `nana-app-shell` | AppShell | Runtime `AppShell`（`title` → title bar，默认槽 → body） |

## NanaButton ↔ `Button`

| Prop / 行为 | Vue `NanaButton` | Rust `nana_ui::Button` |
|-------------|------------------|------------------------|
| 外观 | `kind` string | `ButtonKind` enum |
| 尺寸 | `size` small/medium/large | `ControlSize` |
| 禁用 | `disabled` | `.disabled(bool)` |
| 加载 | `loading` | `.loading(bool)` |
| 触发 | `@press` | `.on_press(Message)` |

## NanaChip ↔ Button 变体

| Prop | Vue | Rust |
|------|-----|------|
| 选中 | `selected` | `ButtonKind::Selected` |
| 触发 | `@select` | `BridgeEvent::Select` |

## NanaSwitch / NanaCheckbox / NanaInput

| Vue | Rust |
|-----|------|
| `NanaSwitch` `modelValue` | `Switch` + `BridgeEvent::Toggle` |
| `NanaCheckbox` | `Checkbox` |
| `NanaInput` | `Input` + `BridgeEvent::Input` |

## NanaRangeField / NanaTabs / NanaSegmented

| Vue | Rust |
|-----|------|
| `NanaRangeField` | `RangeField` + `BridgeEvent::Change` |
| `NanaTabs` `options` | `Tabs` + `SelectValue` |
| `NanaSegmented` | `SegmentedControl` |

## NanaSelect / NanaTextarea

| Vue | Rust |
|-----|------|
| `NanaSelect` `modelValue` + `options` | `Select` + `BridgeEvent::SelectValue` |
| `NanaTextarea` `modelValue` | `Textarea` + `BridgeEvent::Input` / `Editor` |
| `NanaTextarea` `language` | Runtime `HostedTextarea` + `"highlight"` presenter |

## NanaDialog / NanaDrawer / NanaPopover / NanaContextMenu

| Vue | Rust |
|-----|------|
| `NanaDialog` `open` / bool `modelValue` | `Dialog`；`role=alertdialog` 或 `confirm` → `ConfirmDialog` |
| `NanaDrawer` `side` / `width` / `footer` | `Drawer`；footer class `nana-drawer-footer` |
| `NanaPopover` trigger `label` | `Popover`（`active \|\| toggled`） |
| `NanaContextMenu` `options` / `anchorX`/`anchorY` | `ContextMenu` + MenuStore；嵌套 `parent/child` |
| `NanaContextMenuHost` | 绑定 Lilia `useContextMenu` → `NanaContextMenu`（禁 Teleport/`fixed`） |
| `NanaDropdown` / `@lilia/ui/search` alias | `NanaSelect` → Runtime `Select` / `SearchDropdown`（禁 CSS fixed 菜单） |
| `NanaToast` `title` / `description` / `tone` / `dismissible` | Runtime `Toast` |
| `NanaTooltip` `label` | Runtime `Tooltip` |
| `NanaActionMenu` trigger `label` + `nana-action-menu-item` | Runtime `ActionMenu` / `ActionMenuItem` |
| `NanaXyPad` `x`/`y` / `min`/`max` | Runtime `XYPad` |
| `NanaQrCode` `payload` / `label` | 仅载荷；不编码。有 `modules` 时 Runtime `QrCode` |
| `NanaCommandPalette` `open` / `query` / `options` | Runtime `CommandPalette` |
| `NanaTreeView` `nodes`/`options` / `size` | Runtime `TreeView` |
| `NanaCalendar` `data` / `options` | Runtime `CalendarHeatmap`；`options` 对象映射 `cellSize`/`cellGap`/`cellRadius`/`labelWidth`/`monthLabelHeight`/`weekStartsOn`；数组仍作单元格回退；无法解释时空热图 |
| `NanaImageViewer` `src`/`value` | Runtime `ImageViewer` HostTexture id；不解码 |
| `NanaMarkdown` `value`/`modelValue` | Runtime `NativeMarkdown::from_source`；mermaid/math 栅栏为块种类 |
| `NanaMarkdown` `mermaidRenderer` / `mathRenderer` | 应用身份，落到 Runtime `mermaid:{source}` / `math:{source}` 标签；不实现 mermaid/TeX |
| `NanaGraphCanvas` `nodes`/`edges`/`model` | Runtime `GraphCanvas`；无法解释时空模型 |
| `NanaGraphCanvas` `viewport` / `selection` | `GraphViewport`（`offset`/`zoom`）与 `GraphSelection`（node/edge/port） |
| `NanaWorkspace` children `region` / `data-region` | Runtime `Workspace` region slots；无子节点才投影空 slots |
| `NanaDock` children / `layout` / `root` | Runtime `Dock` items（`id`/`title`/`data-dock-id`）；有子节点时不是 `item("dock", None)` |
| `NanaSplitPane` `axis` / first two children | Runtime `SplitPane` `first`/`second`；可选 `nana-split-handle` / 第三子为 handle；`size`/`defaultSize`/`min`/`max` |
| `NanaAppShell` `title` + default slot | Runtime `AppShell`；无 title-bar 子节点时 `title` 生成 `nana-app-title-bar`；其余为 `body`；`data-slot=overlay` 为 overlay |
| `NanaContextMenu` option `icon` | Runtime `ContextMenuItem::icon`（`Icon::parse_name` 成功才设置） |

浮层关闭：宿主 `Toggle false` / `SelectValue` → Vue `change` + `update:modelValue` / `update:open`。

Lilia `UiDialog` / `.modal`（`aria-modal`）presence → Dialog open；**不**兑现 CSS `fixed`/`sticky`。

## FormField / InteractiveCard / Skeleton / LevelMeter

无独立 Vue 包装组件：用 `nana-*` 标签或 class 进入 Runtime 投影。

| Vue | Runtime |
|-----|---------|
| `nana-form-field` / `nana-form` `label` / `hint` / `invalid` / `size` | `FormField`；`invalid` 时 `hint` 为 error；控件 = 首个非 Text 或 input-like 子节点 |
| `nana-interactive-card` `active` / `disabled` | `InteractiveCard` `selected` / `disabled` |
| `nana-skeleton` 布局宽高 | `Skeleton` width/height |
| `nana-level-meter` / `nana-level` `progress`/`value`（0..=1）+ `tone` | `LevelMeter` |

FormField / InteractiveCard 承载子节点，不 Scene 路由。Skeleton / LevelMeter 是 Scene leaf。

## NanaSidebar* / Settings*

| Vue | Rust |
|-----|------|
| `NanaSidebarFrame` | Runtime `SidebarFrame` |
| `NanaSidebarRow` | `SidebarRow` |
| `NanaSettingsRow` / Card / Page | Settings 行/卡组合 |

圆角尺度：语义控件几何由共享 `ThemeMetrics` / `UI_METRICS` 决定（Runtime 与 Iced 适配器共用）。详见
[`docs/vue-nana-renderer-system.md`](../../docs/vue-nana-renderer-system.md)。
