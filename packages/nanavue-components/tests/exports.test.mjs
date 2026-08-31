/**
 * Export + host-tag inventory for L2 Nana* wrappers.
 * No Vue runtime required — static contract against bridge WidgetKind tags.
 */
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, test } from "node:test";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const indexSrc = readFileSync(join(root, "src/index.js"), "utf8");
const pkg = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));

/** New L2 overlays / form controls aligned with WidgetKind. */
const L2_OVERLAY_EXPORTS = [
  "NanaDialog",
  "NanaDrawer",
  "NanaDrawerFooter",
  "NanaPopover",
  "NanaContextMenu",
  "NanaContextMenuHost",
  "NanaToast",
  "NanaTooltip",
  "NanaActionMenu",
  "NanaXyPad",
  "NanaQrCode",
  "NanaSelect",
  "NanaDropdown",
  "NanaSearch",
  "NanaTextarea",
];

const HOST_TAGS = {
  NanaDialog: "dialog",
  NanaDrawer: "nana-drawer",
  NanaPopover: "nana-popover",
  NanaContextMenu: "nana-context-menu",
  NanaToast: "nana-toast",
  NanaTooltip: "nana-tooltip",
  NanaActionMenu: "nana-action-menu",
  NanaXyPad: "nana-xy-pad",
  NanaQrCode: "nana-qr-code",
  NanaSelect: "select",
  NanaDropdown: "nana-dropdown",
  NanaSearch: "search-dropdown",
  NanaTextarea: "textarea",
  NanaCommandPalette: "nana-command-palette",
  NanaTreeView: "nana-tree-view",
  NanaCalendar: "nana-calendar-heatmap",
  NanaImageViewer: "nana-image-viewer",
  NanaMarkdown: "nana-native-markdown",
  NanaGraphCanvas: "nana-graph-canvas",
  NanaWorkspace: "nana-workspace",
  NanaDock: "nana-dock",
  NanaSplitPane: "nana-split-pane",
  NanaAppShell: "nana-app-shell",
  NanaSettingsPage: "nana-settings-page",
  NanaIconButton: "nana-icon-button",
  NanaIcon: "nana-icon",
  NanaNumberInput: "input",
  NanaDivider: "hr",
  NanaThumbnail: "nana-thumbnail",
  NanaCard: "nana-card",
  NanaList: "ul",
  NanaListItem: "li",
  NanaScrollView: "nana-scroll-view",
  NanaProgress: "progress",
  NanaSpinner: "nana-spinner",
  NanaEmptyState: "nana-empty-state",
  NanaStatusBadge: "nana-status-badge",
  NanaValidationMessage: "nana-validation-message",
  NanaLabeledValue: "nana-labeled-value",
  NanaFormField: "nana-form-field",
  NanaInteractiveCard: "nana-interactive-card",
  NanaSkeleton: "nana-skeleton",
  NanaLevelMeter: "meter",
  NanaTable: "table",
  NanaTableRow: "tr",
  NanaTableCell: "td",
  NanaReorderList: "nana-reorder-list",
  NanaTimeSeriesChart: "nana-time-series-chart",
  NanaDesktopShell: "nana-desktop-shell",
  NanaAppTitleBar: "nana-app-title-bar",
  NanaPaneChrome: "nana-pane-chrome",
  NanaSidebarSection: "nana-sidebar-section",
  NanaSidebarFooter: "nana-sidebar-footer",
  NanaSettingsCollapsibleCard: "details",
  NanaGpu: "nana-gpu",
  NanaVirtualList: "nana-scroll-view",
  NanaVirtualTable: "nana-scroll-view",
  NanaVirtualTree: "nana-scroll-view",
};

const SOURCE_FILES = Object.fromEntries(
  Object.keys(HOST_TAGS).map((name) => [name, `src/${name}.js`]),
);

const CATALOG_EXPORTS = Object.keys(HOST_TAGS);

/** Professional first-knife wrappers → Runtime CommandPalette / TreeView / CalendarHeatmap / ImageViewer / NativeMarkdown / GraphCanvas. */
const PROFESSIONAL_EXPORTS = [
  "NanaCommandPalette",
  "NanaTreeView",
  "NanaCalendar",
  "NanaImageViewer",
  "NanaMarkdown",
  "NanaGraphCanvas",
  "NanaWorkspace",
  "NanaDock",
  "NanaSplitPane",
  "NanaAppShell",
];

describe("L2 overlay / form exports", () => {
  for (const name of L2_OVERLAY_EXPORTS) {
    test(`index.js re-exports ${name}`, () => {
      assert.match(
        indexSrc,
        new RegExp(`\\b${name}\\b`),
        `${name} must be exported from src/index.js`,
      );
    });
  }

  for (const [name, file] of Object.entries(SOURCE_FILES)) {
    test(`${name} host tag is ${HOST_TAGS[name]}`, () => {
      const src = readFileSync(join(root, file), "utf8");
      if (name === "NanaTableCell") {
        assert.match(src, /"th"\s*:\s*"td"/);
        return;
      }
      assert.match(
        src,
        new RegExp(`h\\(\\s*["']${HOST_TAGS[name]}["']`),
        `${name} must render host node ${HOST_TAGS[name]}`,
      );
    });
  }

  test("NanaDrawerFooter marks drawer footer for iced partition", () => {
    const src = readFileSync(join(root, "src/NanaDrawer.js"), "utf8");
    assert.match(src, /nana-drawer-footer/);
    assert.match(src, /contentinfo/);
  });

  test("package.json exports subpaths for new components", () => {
    for (const name of [
      "NanaDialog",
      "NanaDrawer",
      "NanaPopover",
      "NanaContextMenu",
      "NanaContextMenuHost",
      "NanaToast",
      "NanaTooltip",
      "NanaActionMenu",
      "NanaXyPad",
      "NanaQrCode",
      "NanaSelect",
      "NanaDropdown",
      "NanaSearch",
      "NanaTextarea",
    ]) {
      assert.ok(pkg.exports[`./${name}`], `missing exports["./${name}"]`);
    }
    assert.ok(pkg.exports["./search"], `missing exports["./search"]`);
  });

  test("Dialog confirm path exposes alertdialog / danger", () => {
    const src = readFileSync(join(root, "src/NanaDialog.js"), "utf8");
    assert.match(src, /alertdialog/);
    assert.match(src, /nana-confirm-dialog/);
    assert.match(src, /danger/);
  });

  test("ContextMenu forwards anchor-x / anchor-y", () => {
    const src = readFileSync(join(root, "src/NanaContextMenu.js"), "utf8");
    assert.match(src, /anchor-x/);
    assert.match(src, /anchor-y/);
  });

  test("NanaDropdown maps to nana-dropdown not CSS fixed", () => {
    const src = readFileSync(join(root, "src/NanaDropdown.js"), "utf8");
    assert.match(src, /nana-dropdown/);
    assert.doesNotMatch(src, /<Teleport/);
  });

  test("NanaContextMenuHost uses NanaContextMenu Overlay path", () => {
    const src = readFileSync(join(root, "src/NanaContextMenuHost.js"), "utf8");
    assert.match(src, /NanaContextMenu/);
    assert.doesNotMatch(src, /<Teleport/);
  });

  test("NanaCalendar forwards options", () => {
    const src = readFileSync(join(root, "src/NanaCalendar.js"), "utf8");
    assert.match(src, /options:\s*props\.options/);
  });

  test("NanaGraphCanvas forwards viewport and selection", () => {
    const src = readFileSync(join(root, "src/NanaGraphCanvas.js"), "utf8");
    assert.match(src, /viewport:\s*props\.viewport/);
    assert.match(src, /selection:\s*props\.selection/);
    assert.match(src, /slots\.default/);
  });

  test("NanaMarkdown forwards mermaid/math renderer identity", () => {
    const src = readFileSync(join(root, "src/NanaMarkdown.js"), "utf8");
    assert.match(src, /mermaidRenderer/);
    assert.match(src, /mathRenderer/);
    assert.match(src, /mermaid-renderer/);
    assert.match(src, /math-renderer/);
  });

  test("NanaSplitPane forwards axis and numeric size props", () => {
    const src = readFileSync(join(root, "src/NanaSplitPane.js"), "utf8");
    assert.match(src, /const axis = props\.axis === "vertical"/);
    assert.match(src, /size:\s*props\.size/);
    assert.match(src, /defaultSize/);
    assert.match(src, /min:\s*props\.min/);
    assert.match(src, /max:\s*props\.max/);
  });

  test("NanaDock forwards layout/root host trees", () => {
    const src = readFileSync(join(root, "src/NanaDock.js"), "utf8");
    assert.match(src, /layout:\s*props\.layout/);
    assert.match(src, /root:\s*props\.root/);
  });

  test("NanaAppShell emits a title-bar child from title", () => {
    const src = readFileSync(join(root, "src/NanaAppShell.js"), "utf8");
    assert.match(src, /nana-app-title-bar/);
    assert.match(src, /data-slot/);
    assert.match(src, /title-bar/);
  });
});

describe("L2 professional first-knife exports", () => {
  for (const name of PROFESSIONAL_EXPORTS) {
    test(`index.js re-exports ${name}`, () => {
      assert.match(
        indexSrc,
        new RegExp(`\\b${name}\\b`),
        `${name} must be exported from src/index.js`,
      );
    });
  }

  test("package.json exports subpaths for professional components", () => {
    for (const name of PROFESSIONAL_EXPORTS) {
      assert.ok(pkg.exports[`./${name}`], `missing exports["./${name}"]`);
    }
  });

  test("professional wrappers stay host tags (no Teleport / fetch / decode)", () => {
    for (const name of PROFESSIONAL_EXPORTS) {
      const src = readFileSync(join(root, SOURCE_FILES[name]), "utf8");
      assert.doesNotMatch(src, /<Teleport/);
      assert.doesNotMatch(src, /\bfetch\s*\(/);
      assert.doesNotMatch(src, /\bImage\s*\(/);
      assert.doesNotMatch(src, /createElementNS/);
    }
  });
});

describe("Runtime catalog Vue wrappers", () => {
  for (const name of CATALOG_EXPORTS) {
    test(`index.js re-exports ${name}`, () => {
      assert.match(
        indexSrc,
        new RegExp(`\\b${name}\\b`),
        `${name} must be exported from src/index.js`,
      );
    });
  }

  test("package.json exports subpaths for catalog wrappers", () => {
    for (const name of CATALOG_EXPORTS) {
      assert.ok(pkg.exports[`./${name}`], `missing exports["./${name}"]`);
    }
  });

  test("NanaScrollView forwards scrollbars and axes", () => {
    const src = readFileSync(join(root, "src/NanaScrollView.js"), "utf8");
    assert.match(src, /scrollbars:\s*props\.scrollbars/);
    assert.match(src, /axes:\s*props\.axes/);
  });

  test("NanaNumberInput maps onto input type=number", () => {
    const src = readFileSync(join(root, "src/NanaNumberInput.js"), "utf8");
    assert.match(src, /h\(\s*["']input["']/);
    assert.match(src, /type:\s*["']number["']/);
  });

  test("virtual wrappers and NanaGpu are exported", () => {
    for (const name of ["NanaGpu", "NanaVirtualList", "NanaVirtualTable", "NanaVirtualTree"]) {
      assert.match(indexSrc, new RegExp(`\\b${name}\\b`));
      assert.ok(pkg.exports[`./${name}`], `missing exports["./${name}"]`);
    }
  });

  test("virtual wrappers window through unique nana-scroll-view tag", () => {
    const tags = {
      "src/NanaVirtualList.js": "nana-scroll-view",
      "src/NanaVirtualTable.js": "nana-scroll-view",
      "src/NanaVirtualTree.js": "nana-scroll-view",
    };
    for (const [file, tag] of Object.entries(tags)) {
      const src = readFileSync(join(root, file), "utf8");
      assert.match(src, new RegExp(`h\\(\\s*["']${tag}["']`));
      assert.match(src, /virtualWindow/);
    }
  });

  test("NanaGpu forwards source onto nana-gpu", () => {
    const src = readFileSync(join(root, "src/NanaGpu.js"), "utf8");
    assert.match(src, /h\(\s*["']nana-gpu["']/);
    assert.match(src, /source/);
    assert.match(src, /data-nana-gpu/);
  });

  test("1:1 HTML wrappers emit native tags", () => {
    const tags = {
      "src/NanaButton.js": "button",
      "src/NanaInput.js": "input",
      "src/NanaCheckbox.js": "input",
      "src/NanaRangeField.js": "input",
      "src/NanaList.js": "ul",
      "src/NanaListItem.js": "li",
      "src/NanaNumberInput.js": "input",
      "src/NanaLevelMeter.js": "meter",
      "src/NanaSettingsCollapsibleCard.js": "details",
    };
    for (const [file, tag] of Object.entries(tags)) {
      const src = readFileSync(join(root, file), "utf8");
      assert.match(src, new RegExp(`h\\(\\s*["']${tag}["']`));
      assert.doesNotMatch(src, /h\(\s*["']nana-/);
    }
    const checkbox = readFileSync(join(root, "src/NanaCheckbox.js"), "utf8");
    assert.match(checkbox, /type:\s*["']checkbox["']/);
    const range = readFileSync(join(root, "src/NanaRangeField.js"), "utf8");
    assert.match(range, /type:\s*["']range["']/);
  });
});

