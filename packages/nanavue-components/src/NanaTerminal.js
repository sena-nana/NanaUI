/**
 * NanaTerminal — retained terminal grid. Semantic peer of Runtime `TerminalView`.
 * PTY, submit, and interrupt stay application-owned (`@input` bytes).
 */
import { h } from "@vue/runtime-core";

export const NanaTerminal = {
  name: "NanaTerminal",
  props: {
    columns: { type: Number, default: 80 },
    rows: { type: Number, default: 24 },
    disabled: { type: Boolean, default: false },
    readOnly: { type: Boolean, default: false },
  },
  emits: ["input", "resize", "selectionchange"],
  setup(props, { attrs, emit }) {
    return () =>
      h("nana-terminal", {
        ...attrs,
        class: ["nana-terminal", props.disabled ? "is-disabled" : "", attrs.class]
          .filter(Boolean)
          .join(" "),
        columns: String(props.columns),
        rows: String(props.rows),
        disabled: props.disabled,
        "read-only": props.readOnly ? "" : undefined,
        "data-agent-id": attrs["data-agent-id"] || "nana.terminal",
        onInput: (ev) => emit("input", ev),
        onResize: (ev) => emit("resize", ev),
        onSelectionchange: (ev) => emit("selectionchange", ev),
      });
  },
};

export default NanaTerminal;
