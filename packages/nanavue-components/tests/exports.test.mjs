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
  "NanaTextarea",
];

const HOST_TAGS = {
  NanaDialog: "nana-dialog",
  NanaDrawer: "nana-drawer",
  NanaPopover: "nana-popover",
  NanaContextMenu: "nana-context-menu",
  NanaToast: "nana-toast",
  NanaTooltip: "nana-tooltip",
  NanaActionMenu: "nana-action-menu",
  NanaXyPad: "nana-xy-pad",
  NanaQrCode: "nana-qr-code",
  NanaSelect: "nana-select",
  NanaTextarea: "nana-textarea",
};

const SOURCE_FILES = {
  NanaDialog: "src/NanaDialog.js",
  NanaDrawer: "src/NanaDrawer.js",
  NanaPopover: "src/NanaPopover.js",
  NanaContextMenu: "src/NanaContextMenu.js",
  NanaToast: "src/NanaToast.js",
  NanaTooltip: "src/NanaTooltip.js",
  NanaActionMenu: "src/NanaActionMenu.js",
  NanaXyPad: "src/NanaXyPad.js",
  NanaQrCode: "src/NanaQrCode.js",
  NanaSelect: "src/NanaSelect.js",
  NanaTextarea: "src/NanaTextarea.js",
};

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

  test("NanaDropdown maps to NanaSelect not CSS fixed", () => {
    const src = readFileSync(join(root, "src/NanaDropdown.js"), "utf8");
    assert.match(src, /NanaSelect/);
    assert.match(src, /nana-select|NanaSelect/);
    assert.doesNotMatch(src, /<Teleport/);
  });

  test("NanaContextMenuHost uses NanaContextMenu Overlay path", () => {
    const src = readFileSync(join(root, "src/NanaContextMenuHost.js"), "utf8");
    assert.match(src, /NanaContextMenu/);
    assert.doesNotMatch(src, /<Teleport/);
  });
});
