/**
 * NanaCalendar — week-column heatmap host.
 * Semantic peer of Runtime `CalendarHeatmap` (`nana-calendar`).
 *
 * `data` / `options` pass through. An `options` object maps heatmap metrics:
 * `cellSize`, `cellGap`, `cellRadius`, `labelWidth`, `monthLabelHeight`,
 * `weekStartsOn` (default Monday / 1), `weekdayLabels` (`[day, label]` / `{day, label}` / strings),
 * `levelStrategy` (`{type:"relative",levels}`, `{type:"thresholds",thresholds}`,
 * or a number array), and string templates `monthFormat` / `titleFormat`
 * (`{year}`, `{month}`, `{monthPad}`, `{date}`, `{value}`). Function-valued
 * formatters stay on the JS side and are ignored. An `options` array stays a
 * cell-data fallback. Host projects an empty heatmap when the payload cannot
 * be interpreted.
 */
import { h } from "@vue/runtime-core";

function cellValue(ev) {
  if (ev == null) return undefined;
  if (typeof ev === "object") {
    if (ev.value !== undefined && ev.value !== null) return ev.value;
    if (ev.date !== undefined && ev.date !== null) return ev.date;
    return undefined;
  }
  return ev;
}

export const NanaCalendar = {
  name: "NanaCalendar",
  props: {
    data: { default: undefined },
    options: { default: undefined },
  },
  emits: ["select"],
  setup(props, { emit, attrs }) {
    function onSelect(ev) {
      const value = cellValue(ev);
      if (value === undefined) return;
      emit("select", value, ev);
    }

    return () =>
      h("nana-calendar", {
        ...attrs,
        class: ["nana-calendar", attrs.class].filter(Boolean).join(" "),
        data: props.data !== undefined ? props.data : props.options,
        options: props.options,
        "data-agent-id": attrs["data-agent-id"] || "nana.calendar",
        onSelect,
      });
  },
};

export default NanaCalendar;
