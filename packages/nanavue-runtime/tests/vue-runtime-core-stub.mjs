/** Test stub — only `createRenderer` is needed to load hostOps. */
export function createRenderer(_options) {
  return {
    render() {},
    createApp() {
      return { mount() {}, unmount() {}, use() { return this; }, provide() { return this; } };
    },
  };
}

export * from "./vue-runtime-core-stub-empty.js";
