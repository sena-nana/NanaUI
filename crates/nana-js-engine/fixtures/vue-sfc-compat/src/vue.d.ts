declare module "*.vue" {
  import type { Component } from "vue";
  const component: Component;
  export default component;
}

declare module "@nanaui/nanavue-components" {
  import type { Component } from "vue";
  export const NanaAppShell: Component;
}
