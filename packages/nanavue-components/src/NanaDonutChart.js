/** Retained Runtime donut chart; colors use SemanticColorRole names. */
import { h } from "@vue/runtime-core";
export const NanaDonutChart = {
  name: "NanaDonutChart",
  props: {
    slices: { type: Array, default: () => [] },
    labels: { type: Array, default: () => [] },
    label: { type: String, default: "" },
    cutout: { type: Number, default: 0.62 },
  },
  setup(props, { attrs }) {
    return () => h("nana-donut-chart", {
      ...attrs,
      slices: props.slices,
      labels: props.labels,
      label: props.label,
      cutout: props.cutout,
    });
  },
};
export default NanaDonutChart;
