import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { reactive, createRenderer } from "@vue/runtime-core";

// Load the workspace sources against this fixture's pinned Vue installation.
const sourceBase = new URL("../../../../packages/nanavue-components/src/", import.meta.url);
const vueUrl = import.meta.resolve("@vue/runtime-core");
const modules = {};
for (const name of ["NanaVirtualList", "NanaVirtualTree", "NanaVirtualTable"]) {
  const source = (await readFile(new URL(`${name}.js`, sourceBase), "utf8"))
    .replaceAll('"@vue/runtime-core"', JSON.stringify(vueUrl))
    .replaceAll('"./virtual-window.js"', JSON.stringify(new URL("virtual-window.js", sourceBase).href))
    .replaceAll('"./NanaVirtualList.js"', JSON.stringify(modules.NanaVirtualList));
  modules[name] = `data:text/javascript;base64,${Buffer.from(source).toString("base64")}`;
}

for (const name of Object.keys(modules)) {
  test(`${name} caches reactive dimensions while scrolling`, async () => {
    const Component = (await import(modules[name]))[name];
    const table = name === "NanaVirtualTable";
    for (const axis of table ? ["row", "column"] : ["item"]) {
      let reads = 0;
      const extents = reactive(new Proxy(Array(100_000).fill(32), {
        get(target, key, receiver) {
          if (/^\d+$/.test(String(key))) reads++;
          return Reflect.get(target, key, receiver);
        },
      }));
      const props = reactive(table ? {
        rowCount: axis === "row" ? 100000 : 10,
        columnCount: axis === "column" ? 100000 : 10,
        rowExtent: 32, columnExtent: 32,
        [`${axis}Extents`]: extents,
      } : { count: 100000, itemExtent: 32, extents });
      const host = { layoutBox: { width: 300, height: 600 } };
      // Mount once so lifecycle measurement uses the actual component hooks.
      const renderer = createRenderer({
        createElement: () => ({ ...host, props: {}, children: [] }),
        createText: text => ({ text }), createComment: text => ({ text }),
        insert: (child, parent) => parent.children.push(child), remove() {},
        setText(node, text) { node.text = text; }, setElementText() {},
        parentNode: () => null, nextSibling: () => null,
        patchProp(node, key, _old, value) { node.props[key] = value; },
      });
      const app = renderer.createApp(Component, props);
      app.mount({ children: [] });
      const instance = app._instance;
      let vnode = instance.render.call(instance.proxy);
      vnode.props.ref(host);
      vnode.props.onScroll({ scrollTop: 0, scrollLeft: 0 });
      vnode = instance.render.call(instance.proxy);
      reads = 0;
      for (const offset of [32, 32000, 3000000]) {
        vnode.props.onScroll({ scrollTop: axis === "column" ? 0 : offset, scrollLeft: axis === "column" ? offset : 0 });
        vnode = instance.render.call(instance.proxy);
      }
      host.layoutBox.height = 900;
      host.layoutBox.width = 900;
      vnode.props.onScroll({ scrollTop: axis === "column" ? 0 : 3000000, scrollLeft: axis === "column" ? 3000000 : 0 });
      vnode = instance.render.call(instance.proxy);
      assert.equal(reads, 0, "scroll and viewport changes must not reread extents");
      const leading = () => axis === "column"
        ? vnode.children[0].children[0].props.style.width
        : vnode.children[0].props.style.height;
      extents[0] = 64;
      vnode = instance.render.call(instance.proxy);
      assert.ok(reads >= 100000, "in-place size edits rebuild the index");
      assert.equal(leading(), "2999936px");
      extents.splice(0, extents.length);
      vnode = instance.render.call(instance.proxy);
      assert.equal(leading(), "2999936px", "empty extents fall back to uniform sizes");
      instance.props[`${axis}Extent`] = 40;
      vnode = instance.render.call(instance.proxy);
      assert.equal(leading(), "2999920px");
      app.unmount();
    }
  });
}
