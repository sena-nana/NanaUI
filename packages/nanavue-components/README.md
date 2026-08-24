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
  NanaSidebarFrame,
  NanaSidebarRow,
  NanaTabs,
} from "@nanaui/nanavue-components";
import "@nanaui/nanavue-components/controls.css";
```

| Vue | Runtime |
| --- | --- |
| `NanaButton` | `Button` |
| `NanaInput` | `TextInput` |
| `NanaTextarea` | `TextArea` |
| `NanaCheckbox` | `Checkbox` |
| `NanaSelect` | `Select` |
| `NanaDialog` | `Dialog` / `ConfirmDialog` |
| `NanaDrawer` | `Drawer` |
| `NanaPopover` | `Popover` |
| `NanaContextMenu` | `ContextMenu` |
| `NanaTabs` | `Tabs` |
| `NanaSegmented` | `SegmentedControl` |
| `NanaCommandPalette` | `CommandPalette` |
| `NanaTreeView` | `TreeView` |
| `NanaSidebarFrame` / `NanaSidebarRow` | `SidebarFrame` / `SidebarRow` |
| `NanaWorkspaceShell` | `Workspace` |
| `NanaAppearancePanel` | 外观设置 |
| `NanaCalendar` | `CalendarHeatmap` |
| `NanaImageViewer` | `ImageViewer` |
| `NanaMarkdown` | `NativeMarkdown` |
| `NanaGraphCanvas` | `GraphCanvas` |

浮层用 `open` + `onUpdate:open`。示例：[`examples/overlay-controls.js`](./examples/overlay-controls.js)、[`examples/appearance-workspace.js`](./examples/appearance-workspace.js)。props 细则见 [`SEMANTICS.md`](./SEMANTICS.md)。
