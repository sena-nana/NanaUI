/**
 * Vue example — Dialog / Drawer / Popover / ContextMenu / Select / Textarea.
 *
 *   import { createOverlayControlsDemo } from "@nanaui/nanavue-components/examples/overlay-controls";
 */
import { h, ref } from "@vue/runtime-core";
import { NanaButton } from "../src/NanaButton.js";
import { NanaDialog } from "../src/NanaDialog.js";
import { NanaDrawer } from "../src/NanaDrawer.js";
import { NanaPopover } from "../src/NanaPopover.js";
import { NanaContextMenu } from "../src/NanaContextMenu.js";
import { NanaSelect } from "../src/NanaSelect.js";
import { NanaTextarea } from "../src/NanaTextarea.js";

export function createOverlayControlsDemo(options = {}) {
  const dialogOpen = ref(false);
  const confirmOpen = ref(false);
  const drawerOpen = ref(false);
  const popoverOpen = ref(false);
  const menuOpen = ref(false);
  const selectValue = ref(options.selectValue || "alpha");
  const notes = ref(options.notes || "");

  return {
    name: "NanaOverlayControlsDemo",
    setup() {
      return () =>
        h("nana-column", { class: "nana-demo-panel", "data-agent-id": "demo.overlays" }, [
          h("h2", null, options.title || "Overlay & form controls"),
          h("nana-row", { style: "gap: 8px" }, [
            h(NanaButton, {
              kind: "primary",
              label: "Dialog",
              onPress: () => {
                dialogOpen.value = true;
              },
            }),
            h(NanaButton, {
              kind: "danger",
              label: "Confirm",
              onPress: () => {
                confirmOpen.value = true;
              },
            }),
            h(NanaButton, {
              kind: "secondary",
              label: "Drawer",
              onPress: () => {
                drawerOpen.value = true;
              },
            }),
            h(NanaPopover, {
              open: popoverOpen.value,
              label: "More",
              description: "Popover body",
              "onUpdate:open": (v) => {
                popoverOpen.value = v;
              },
            }, {
              default: () => h("p", null, "Anchored popover content."),
            }),
            h(NanaButton, {
              kind: "ghost",
              label: "Context menu",
              onPress: () => {
                menuOpen.value = true;
              },
            }),
          ]),
          h(NanaSelect, {
            modelValue: selectValue.value,
            placeholder: "Pick one",
            options: [
              { value: "alpha", label: "Alpha" },
              { value: "beta", label: "Beta" },
            ],
            "onUpdate:modelValue": (v) => {
              selectValue.value = v;
            },
          }),
          h(NanaTextarea, {
            modelValue: notes.value,
            placeholder: "Notes…",
            height: 96,
            "onUpdate:modelValue": (v) => {
              notes.value = v;
            },
          }),
          h(
            NanaDialog,
            {
              open: dialogOpen.value,
              title: "Example dialog",
              description: "Body via description when no slot.",
              "onUpdate:open": (v) => {
                dialogOpen.value = v;
              },
            },
            {
              default: () => h("p", null, "Dialog slot body."),
            },
          ),
          h(NanaDialog, {
            open: confirmOpen.value,
            confirm: true,
            kind: "danger",
            title: "删除确认",
            description: "此操作不可撤销",
            "onUpdate:open": (v) => {
              confirmOpen.value = v;
            },
            onConfirm: () => {
              confirmOpen.value = false;
            },
          }),
          h(
            NanaDrawer,
            {
              open: drawerOpen.value,
              title: "侧栏",
              side: "right",
              width: 360,
              "onUpdate:open": (v) => {
                drawerOpen.value = v;
              },
            },
            {
              default: () => h("p", null, "Drawer body."),
              footer: () => [
                h(NanaButton, {
                  kind: "ghost",
                  label: "取消",
                  class: "drawer-footer-cancel",
                  onPress: () => {
                    drawerOpen.value = false;
                  },
                }),
                h(NanaButton, {
                  kind: "primary",
                  label: "确认",
                  class: "drawer-footer-confirm",
                }),
              ],
            },
          ),
          h(NanaContextMenu, {
            open: menuOpen.value,
            anchorX: options.anchorX ?? 120,
            anchorY: options.anchorY ?? 160,
            options: [
              { value: "cut", label: "Cut" },
              { value: "copy", label: "Copy" },
              { value: "file/rename", label: "Rename" },
            ],
            "onUpdate:open": (v) => {
              menuOpen.value = v;
            },
            onSelect: () => {
              menuOpen.value = false;
            },
          }),
        ]);
    },
  };
}

export default createOverlayControlsDemo;
