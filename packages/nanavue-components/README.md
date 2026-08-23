# @nanaui/nanavue-components

**L2**：Vue 直接调 Nana 组件接口（语义 props → Style Model Semantics / Tokens），
**可跳过 CSS 解析**。可与 L1 `createElement` 同树混合。

架构：**变体/组合** — 可见 UI 落到 Nana 布局原语 + 基础控件；Vue 自造组件用组合与变体表达。CustomContent paint **已移除**。详见 [`SEMANTICS.md`](./SEMANTICS.md) 与
[`docs/vue.md`](../../docs/vue.md)。

## Bridges (`src/appearance.js`)

| API | Replaces |
|-----|----------|
| `setLiliaUiConfig` | `@lilia/ui/shell` |
| `provideLiliaSettings` | `@lilia/ui/settings` |
| `installNativeAppearance` | Tauri adapter + `installNativeAppearance` |
| `installCornerStyle` | `@lilia/ui/runtime` |
| `installGlobalScrollbarVisibility` | ResizeObserver + window listeners |
| `installLiliaContextMenu` | context-menu directive stub |

Writes `document.documentElement.dataset` / CSS vars through the Nana web-api shim
(`localStorage` backed).

## Native controls

| Vue | NanaUI / core | Semantics |
|-----|---------------|-----------|
| `NanaButton` | `nana_ui::Button` | `kind` / `size` / `disabled` / `loading` · emit `press` |
| `NanaChip` | Button Selected/Subtle 变体 | `selected` · emit `select` |
| `NanaInput` | `nana_ui::Input` | `modelValue` · emit `input` |
| `NanaTextarea` | `nana_ui::Textarea` | `modelValue` · emit `input` |
| `NanaCheckbox` | `nana_ui::Checkbox` | `modelValue` · emit `change` |
| `NanaSelect` | `nana_ui::Select` | `modelValue` + `options` · emit `select` |
| `NanaDialog` | `Dialog` / `ConfirmDialog` | `open` · `role=alertdialog` / `confirm` · emit `close`/`confirm` |
| `NanaDrawer` | `Drawer` | `open` / `side` / `width` · `footer` slot |
| `NanaDrawerFooter` | Drawer footer partition | class `nana-drawer-footer` |
| `NanaPopover` | `Popover` | `open` + trigger `label` · default slot body |
| `NanaContextMenu` | `ContextMenu` / MenuStore | `open` / `options` / `anchorX`/`anchorY` |
| `NanaThemeToggle` | `ThemeMode` chips | Light/Dark · emit `change` |
| `NanaAppearancePanel` | Appearance settings tab | theme + corners + backdrop |
| `NanaWorkspaceShell` | Workspace Start/Primary regions | sidebar + primary slots |
| `NanaSidebarNav` | Global/Section navigation | `items` / `activeKey` · emit `select` |
| `NanaSidebarFrame` | `SidebarFrame` | top / body / footer |
| `NanaSidebarRow` | `SidebarRow` | label / active · emit `select` |
| `NanaSegmented` | `SegmentedControl` | `modelValue` + `options` |
| `NanaTabs` | `Tabs` | `modelValue` + `options` |
| `NanaCommandPalette` | `CommandPalette` | `open` / `query` / `options` · emit `select` |
| `NanaTreeView` | `TreeView` | `nodes`/`options` / `size` · emit `select`/`toggle` |
| `NanaCalendar` | `CalendarHeatmap` | `data` cells · `options` metrics object or cell-array fallback · emit `select` |
| `NanaImageViewer` | `ImageViewer` | `open` + host texture `src`/`value` · emit `close` |
| `NanaMarkdown` | `NativeMarkdown` | `value`/`modelValue` source · optional `mermaidRenderer`/`mathRenderer` identity |
| `NanaGraphCanvas` | `GraphCanvas` | `nodes`/`edges`/`model` · `viewport`/`selection` |

Shared CSS: `src/nana-controls.css` — **inherits** `lilia-tokens.css`（禁止独立 `#3867ff`）。

```js
import {
  NanaButton,
  NanaDialog,
  NanaDrawer,
  NanaPopover,
  NanaContextMenu,
  NanaSelect,
  NanaTextarea,
  NanaSidebarFrame,
  NanaSidebarRow,
  NanaSidebarNav,
  NanaAppearancePanel,
  NanaTabs,
} from "@nanaui/nanavue-components";
import "@nanaui/nanavue-components/controls.css";
```

### Overlay usage

```js
import { ref, h } from "@vue/runtime-core";
import { NanaDialog, NanaDrawer, NanaSelect } from "@nanaui/nanavue-components";

const open = ref(false);
const pick = ref("a");

h(NanaDialog, {
  open: open.value,
  title: "标题",
  description: "说明",
  "onUpdate:open": (v) => { open.value = v; },
}, { default: () => h("p", null, "内容") });

h(NanaDialog, {
  open: true,
  confirm: true,
  kind: "danger",
  title: "删除确认",
  description: "不可撤销",
});

h(NanaDrawer, {
  open: true,
  title: "侧栏",
  side: "right",
  width: 360,
}, {
  default: () => h("p", null, "body"),
  footer: () => h(NanaButton, { class: "drawer-footer-confirm", label: "确认" }),
});

h(NanaSelect, {
  modelValue: pick.value,
  options: [{ value: "a", label: "A" }, { value: "b", label: "B" }],
  "onUpdate:modelValue": (v) => { pick.value = v; },
});
```

示例：[`examples/appearance-workspace.js`](./examples/appearance-workspace.js)、
[`examples/overlay-controls.js`](./examples/overlay-controls.js)  
对照：[`SEMANTICS.md`](./SEMANTICS.md)

```bash
npm test --prefix packages/nanavue-components
```
