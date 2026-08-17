/**
 * NanaQrCode — QR payload / label host.
 * Semantic peer of Runtime `QrCode` (`nana-qr-code`).
 *
 * Vue supplies `payload` and `label` only. Encoding stays on the Scene host
 * `QrCodeCanvas`; this wrapper does not include a JS encoder.
 */
import { h } from "@vue/runtime-core";

export const NanaQrCode = {
  name: "NanaQrCode",
  props: {
    payload: { type: String, default: "" },
    label: { type: String, default: "" },
    value: { type: String, default: "" },
  },
  setup(props, { attrs }) {
    return () => {
      const payload = props.payload || props.value;
      const label = props.label || payload;
      return h("nana-qr-code", {
        ...attrs,
        class: ["nana-qr-code", attrs.class].filter(Boolean).join(" "),
        label,
        payload,
        value: payload,
        "data-agent-id": attrs["data-agent-id"] || "nana.qr-code",
      });
    };
  },
};

export default NanaQrCode;
