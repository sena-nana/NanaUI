/**
 * NanaLevelMeter — semantic peer of Runtime `LevelMeter` (`meter`).
 */
import { h } from "@vue/runtime-core";

export const NanaLevelMeter = {
  name: "NanaLevelMeter",
  props: {
    modelValue: { type: Number, default: 0 },
    value: { type: Number, default: undefined },
    progress: { type: Number, default: undefined },
    tone: { type: String, default: "neutral" },
  },
  setup(props, { attrs }) {
    return () => {
      const value = props.value ?? props.progress ?? props.modelValue;
      return h("meter", {
        ...attrs,
        class: ["nana-level-meter", attrs.class].flat().filter(Boolean).join(" "),
        value,
        progress: value,
        tone: props.tone,
        "data-agent-id": attrs["data-agent-id"] || "nana.level-meter",
      });
    };
  },
};

export default NanaLevelMeter;
