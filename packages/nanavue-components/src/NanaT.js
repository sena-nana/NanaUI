/**
 * NanaT — localized text: `<T id="files" :count="n" />` (Issue #267).
 *
 * Renders a `<nana-text>` that names a message in the application's catalog
 * (`Nana.i18n.setCatalog`). The Runtime resolves it in the locale of the
 * scope it is in, and again on every switch, so a switch re-renders nothing
 * here. `class`, `style`, `lang`, `dir`, `locale`, `data-*`, `aria-*` and
 * listeners stay on the element; every other attribute is an argument of
 * the message. `args` gives arguments by name, over those, including the
 * names the element keeps.
 */
import { h } from "@vue/runtime-core";
import { messageTextProps } from "./message-args.js";

export const NanaT = {
  name: "NanaT",
  inheritAttrs: false,
  props: {
    /** The message's name in the catalog. */
    id: { type: String, required: true },
    /** Arguments by name, over the ones given as attributes. */
    args: { type: Object, default: null },
  },
  setup(props, { attrs }) {
    return () => h("nana-text", messageTextProps(props.id, attrs, props.args));
  },
};

export const T = NanaT;

export default NanaT;
