import DefaultTheme from "vitepress/theme";
import { h } from "vue";
import ApiStyleSwitch from "./ApiStyleSwitch.vue";
import "./style.css";

export default {
  extends: DefaultTheme,
  Layout() {
    return h(DefaultTheme.Layout, null, {
      "sidebar-nav-before": () => h(ApiStyleSwitch),
    });
  },
};
