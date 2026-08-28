# @nanaui/nanavue-components

NanaUI 控件的 Vue 面。语义 props 进同一棵原生树，可与普通标签 / CSS 子集混用。

系统说明：[Vue](../../docs/vue.md)、[控件](../../docs/components.md)。

```js
import {
  NanaButton,
  NanaDialog,
  NanaDrawer,
  NanaInput,
  NanaSelect,
  NanaScrollView,
  NanaSidebarFrame,
  NanaSidebarRow,
  NanaTabs,
} from "@nanaui/nanavue-components";
import "@nanaui/nanavue-components/controls.css";
```

| Vue | Runtime |
| --- | --- |
| `NanaButton` | `Button` |
| `NanaIconButton` | `IconButton` |
| `NanaIcon` | `IconGlyph` |
| `NanaChip` | Button Selected/Subtle 变体 |
| `NanaInput` | `TextInput` |
| `NanaNumberInput` | `NumberInput` |
| `NanaTextarea` | `TextArea` |
| `NanaCheckbox` | `Checkbox` |
| `NanaSwitch` | `Switch` |
| `NanaRangeField` | `RangeField` |
| `NanaSelect` | `Select` |
| `NanaDropdown` | `Dropdown` |
| `NanaSearch` | `SearchDropdown` |
| `NanaTabs` | `Tabs` |
| `NanaSegmented` | `SegmentedControl` |
| `NanaXyPad` | `XYPad` |
| `NanaCard` | `Card` |
| `NanaList` / `NanaListItem` | `List` / `ListItem` |
| `NanaScrollView` | `ScrollView` |
| `NanaVirtualList` / `NanaVirtualTable` / `NanaVirtualTree` | `ScrollView` + 可见窗口（对齐 `materialize_virtual_*`） |
| `NanaGpu` | `GpuTextureView`（`<nana-gpu>`） |
| `NanaFormField` | `FormField` |
| `NanaEmptyState` | `EmptyState` |
| `NanaProgress` | `Progress` |
| `NanaSkeleton` | `Skeleton` |
| `NanaSpinner` | `Spinner` |
| `NanaStatusBadge` | `StatusBadge` |
| `NanaValidationMessage` | `ValidationMessage` |
| `NanaLabeledValue` | `LabeledValue` |
| `NanaDivider` | `Divider` |
| `NanaThumbnail` | `Thumbnail` |
| `NanaInteractiveCard` | `InteractiveCard` |
| `NanaLevelMeter` | `LevelMeter` |
| `NanaTable` / `NanaTableRow` / `NanaTableCell` | `Table` / `TableRow` / `TableCell` |
| `NanaReorderList` | `ReorderList` |
| `NanaTimeSeriesChart` | `TimeSeriesChart` |
| `NanaDialog` | `Dialog` / `ConfirmDialog` |
| `NanaDrawer` | `Drawer` |
| `NanaPopover` | `Popover` |
| `NanaContextMenu` | `ContextMenu` |
| `NanaToast` | `Toast` |
| `NanaTooltip` | `Tooltip` |
| `NanaActionMenu` | `ActionMenu` |
| `NanaCommandPalette` | `CommandPalette` |
| `NanaTreeView` | `TreeView` |
| `NanaQrCode` | `QrCode` |
| `NanaSidebarFrame` / `NanaSidebarSection` / `NanaSidebarRow` / `NanaSidebarFooter` | `SidebarFrame` / `SidebarSection` / `SidebarRow` / `SidebarFooter` |
| `NanaWorkspace` | `Workspace` |
| `NanaWorkspaceShell` | Desktop 壳组合 |
| `NanaAppearancePanel` | 外观设置 |
| `NanaCalendar` | `CalendarHeatmap` |
| `NanaImageViewer` | `ImageViewer` |
| `NanaMarkdown` | `NativeMarkdown` |
| `NanaGraphCanvas` | `GraphCanvas` |
| `NanaDock` | `Dock` |
| `NanaSplitPane` | `SplitPane` |
| `NanaAppShell` / `NanaAppTitleBar` | `AppShell` / `AppTitleBar` |
| `NanaDesktopShell` | `DesktopShell` |
| `NanaPaneChrome` | `PaneChrome` |
| `NanaSettingsRow` / `NanaSettingsCard` / `NanaSettingsPage` / `NanaSettingsCollapsibleCard` | 设置行 / 卡 / 页 / 折叠卡 |

浮层用 `open` + `onUpdate:open`。`NanaScrollView` 用 `scrollbars="always|hidden"`（默认 auto-hide）。大列表用 `NanaVirtualList` / `Table` / `Tree`：只把当前窗口做成 live 节点。GPU 画面用 `NanaGpu` 的 `source`，对应宿主 `HostTexture` 槽，不是 2D canvas。示例：[`examples/overlay-controls.js`](./examples/overlay-controls.js)、[`examples/appearance-workspace.js`](./examples/appearance-workspace.js)。props 细则见 [`SEMANTICS.md`](./SEMANTICS.md)。
