/**
 * NanaTimeSeriesChart — semantic peer of Runtime `TimeSeriesChart`.
 */
import { h } from "@vue/runtime-core";

export const NanaTimeSeriesChart = {
  name: "NanaTimeSeriesChart",
  props: {
    values: { type: Array, default: () => [] },
    data: { type: Array, default: undefined },
    label: { type: String, default: "" },
  },
  setup(props, { attrs }) {
    return () => {
      const values = props.data || props.values;
      return h("nana-time-series-chart", {
        ...attrs,
        class: ["nana-time-series-chart", attrs.class].flat().filter(Boolean).join(" "),
        label: props.label,
        values,
        data: values,
        "data-agent-id": attrs["data-agent-id"] || "nana.time-series-chart",
      });
    };
  },
};

export default NanaTimeSeriesChart;
