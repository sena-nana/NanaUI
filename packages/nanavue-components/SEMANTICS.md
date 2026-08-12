# nanavue-components ↔ NanaUI 语义对照

Issue #5 — Vue **基础组件与布局原语**经 `MessageBridge` 落到真正的 Iced / `nana_ui` 绘制。  
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
| L3 / Iced | `nana-ui` 唯一绘制；与 LiliaUI 公开组件 1:1（见 `docs/lilia-component-parity.md`） |
| 自定义 | **组合与逻辑**；不另起 paint 引擎。CustomContent **已移除** |
| Token | 主题档位 → `ThemeMetrics` / 语义色；`nana-controls.css` 禁止独立 `#3867ff`；任意业务 CSS 色值不得污染正式 token |

## 消息桥

| 方向 | 机制 |
|------|------|
| Vue → Rust | `createWidget` / `createElement`（含 HTML 降维）+ `patchProp` → `MessageBridge` |
| Rust → Iced | `semantic_snapshot()` → `nana_ui_vue::view_semantic_tree*` (`iced_app`) → `Button` / `Switch` / … |
| Iced → Vue | `BridgeEvent` → `VueHost::dispatch_bridge_event` → `__nanaFireEvent` |
| Theme（单向） | `VueHost::inject_theme` |

验收：`cargo run -p vue-counter -- counter --semantic --clicks=2`；
`cargo check -p vue-counter --features windowed`；
`cargo test -p nana-ui-vue --features iced-view --lib`。

## 降维映射（节选）

| 来源 | Nana `WidgetKind` | Iced 绘制 |
|------|-------------------|-----------|
| `nana-button` / `<button>` / role=button | Button | `nana_ui::Button` |
| `nana-chip` / class `nana-chip` | Chip | Button Selected/Subtle 变体 |
| `nana-switch` / role=switch | Switch | `Switch` |
| `nana-checkbox` / `input[type=checkbox]` | Checkbox | `Checkbox` |
| `nana-input` / `<input>` | Input | `Input` |
| `nana-tabs` / role=tablist / class `nana-tabs` | Tabs | `Tabs` |
| `nana-segmented` / class `nana-segmented` | Segmented | `SegmentedControl` |
| `nana-range` / role=slider | Range | `RangeField` |
| `nana-sidebar-row` / class sidebar-row | SidebarRow | `SidebarRow` |
| `div` / `section` / `main` / … | Column / Row | iced column/row 布局盒 |
| `#text` / `span` / `p` / `h*` | Text | iced text |
| `li` | ListItem | `ListItem` |
| class `card` / `nana-card` | Card | `Card` |
| `nana-select` / `<select>` | Select | `nana_ui::Select` |
| `nana-textarea` / `<textarea>` | Textarea | `Textarea` + EditorStore |
| `nana-dialog` / role=dialog | Dialog | `Dialog`（`open`/`active`） |
| `nana-dialog` + role=alertdialog / class confirm | Dialog | `ConfirmDialog` |
| `nana-drawer` / sheet | Drawer | `Drawer`（`side`/`width`/`footer`） |
| `nana-popover` | Popover | `Popover` |
| `nana-context-menu` | ContextMenu | ActionMenuItem 列表 / MenuStore |

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

## NanaDialog / NanaDrawer / NanaPopover / NanaContextMenu

| Vue | Rust |
|-----|------|
| `NanaDialog` `open` / bool `modelValue` | `Dialog`；`role=alertdialog` 或 `confirm` → `ConfirmDialog` |
| `NanaDrawer` `side` / `width` / `footer` | `Drawer`；footer class `nana-drawer-footer` |
| `NanaPopover` trigger `label` | `Popover`（`active \|\| toggled`） |
| `NanaContextMenu` `options` / `anchorX`/`anchorY` | `ContextMenu` + MenuStore；嵌套 `parent/child` |
| `NanaContextMenuHost` | 绑定 Lilia `useContextMenu` → `NanaContextMenu`（禁 Teleport/`fixed`） |
| `NanaDropdown` / `@lilia/ui/search` alias | `NanaSelect` → iced `Select`（禁 CSS fixed 菜单） |

浮层关闭：宿主 `Toggle false` / `SelectValue` → Vue `change` + `update:modelValue` / `update:open`。

Lilia `UiDialog` / `.modal`（`aria-modal`）presence → Dialog open；**不**兑现 CSS `fixed`/`sticky`。

## NanaSidebar* / Settings*

| Vue | Rust |
|-----|------|
| `NanaSidebarFrame` | 布局 Column（槽位组合） |
| `NanaSidebarRow` | `SidebarRow` |
| `NanaSettingsRow` / Card / Page | Settings 行/卡组合 |

圆角尺度：语义控件几何由 Iced `UI_METRICS` 决定。详见
[`docs/vue-nana-renderer-system.md`](../../docs/vue-nana-renderer-system.md)。
