import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import { reactive, createRenderer, nextTick } from "@vue/runtime-core";

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


for (const name of ["NanaVirtualList", "NanaVirtualTree"]) {
  for (const automatic of [false, true]) {
  test(`${name} ${automatic ? "automatically" : "explicitly"} retains keyed editors across distant navigation and releases them`, async () => {
    const Component = (await import(modules[name]))[name];
    const writes = [];
    const renderer = createRenderer({
      createElement: tag => ({tag, props: {}, children: [], layoutBox: {width: 300, height: 100},
        scrollTo(x, y) { writes.push({x, y, mounted: items(this).length}); },
      }),
      createText: text => ({text}), createComment: text => ({text}),
      insert(child, parent, anchor) {
        if (child.parent) child.parent.children.splice(child.parent.children.indexOf(child), 1);
        child.parent = parent;
        const at = anchor ? parent.children.indexOf(anchor) : -1;
        if (at < 0) parent.children.push(child); else parent.children.splice(at, 0, child);
      },
      remove(child) { child.parent.children.splice(child.parent.children.indexOf(child), 1); child.parent = null; },
      setText(node, text) { node.text = text; }, setElementText() {},
      parentNode: node => node.parent,
      nextSibling: node => node.parent?.children[node.parent.children.indexOf(node) + 1] ?? null,
      patchProp(node, key, _old, value) { node.props[key] = value; },
    });
    const items = host => host.children.filter(node => /__(item|row)$/.test(node.props?.class ?? ""));
    const root = {children: []};
    const app = renderer.createApp(Component, {
      count: 1_000_000, itemExtent: 20, overscan: 0,
      keyAt: index => `business-${index}`, indexOfKey: key => Number(key.slice(9)),
      retainedKeys: automatic ? [] : ["business-2"],
    });
    const api = app.mount(root);
    await nextTick();
    const host = root.children[0];
    const editor = items(host)[2];
    editor.draft = "uncommitted input";
    const input = {};
    if (automatic) {
      editor.props.onFocusCapture({target: input});
      editor.props.onCompositionstartCapture({target: input});
    }
    assert.equal(await api.scrollToIndex(500_000, "start"), true);
    assert.deepEqual(writes.at(-1), {x: 0, y: 10_000_000, mounted: 6});
    assert.equal(items(host).length, 6);
    assert.equal(items(host)[0], editor);
    assert.equal(editor.draft, "uncommitted input");
    assert.equal(host.children[0].props.style.height, "40px");
    assert.equal(host.children[2].props.style.height, "9999940px");
    const beforeInvalid = writes.length;
    assert.equal(await api.scrollToIndex(1_000_000), false);
    assert.equal(writes.length, beforeInvalid);
    if (automatic) {
      editor.props.onBlurCapture({target: input});
      await nextTick();
      assert.ok(items(host).includes(editor), "composition survives blur until its own end");
      editor.props.onFocusCapture({target: input});
      editor.props.onCompositionendCapture({target: input});
      await nextTick();
      assert.ok(items(host).includes(editor), "focused editor survives composition end");
    }
    // Data reorder resolves the same key at a new index without replacing its view.
    app._instance.props.keyAt = index => `business-${index === 2 ? 8 : index === 8 ? 2 : index}`;
    app._instance.props.indexOfKey = key => Number(key.slice(9)) === 2 ? 8 : Number(key.slice(9));
    await nextTick();
    assert.equal(items(host)[0], editor);
    assert.equal(host.children[0].props.style.height, "160px");
    // A stale inverse after deletion must not retain an unrelated replacement.
    app._instance.props.keyAt = index => `replacement-${index}`;
    await nextTick();
    assert.equal(items(host).length, 5);
    assert.ok(!items(host).includes(editor));
    app._instance.props.retainedKeys = [];
    await nextTick();
    assert.equal(items(host).length, 5);
    assert.ok(!items(host).includes(editor));
    app.unmount();
    assert.equal(root.children.length, 0);
  });
  }
}


test("table navigation materializes both axes before one scroll write", async () => {
  const Component = (await import(modules.NanaVirtualTable)).NanaVirtualTable;
  const writes = [];
  const renderer = createRenderer({
    createElement: () => ({props: {}, children: [], layoutBox: {width: 200, height: 100},
      scrollTo(x, y) { writes.push([x, y]); },
    }),
    createText: text => ({text}), createComment: text => ({text}),
    insert(child, parent) { parent.children.push(child); }, remove() {},
    setText() {}, setElementText() {}, parentNode: () => null, nextSibling: () => null,
    patchProp(node, key, _old, value) { node.props[key] = value; },
  });
  const app = renderer.createApp(Component, {
    rowCount: 1_000_000, columnCount: 1_000_000, rowExtent: 20, columnExtent: 40, overscan: 0,
  });
  const api = app.mount({children: []});
  await nextTick();
  assert.equal(await api.scrollToCell(500_000, 800_000, "start"), true);
  assert.deepEqual(writes, [[32_000_000, 10_000_000]]);
  const vnode = app._instance.render.call(app._instance.proxy);
  assert.equal(vnode.children.length, 7);
  assert.equal(vnode.children[1].children.length, 7);
  assert.equal(await api.scrollToCell(2, 1_000_000), false);
  assert.equal(writes.length, 1);
  app.unmount();
});


test("host resize publication fills the initial window and disconnects on unmount", async () => {
  const Component = (await import(modules.NanaVirtualList)).NanaVirtualList;
  const previous = globalThis.ResizeObserver;
  let callback, observed = null;
  globalThis.ResizeObserver = class {
    constructor(cb) { callback = cb; }
    observe(node) { observed = node; }
    disconnect() { observed = null; }
  };
  try {
    const renderer = createRenderer({
      createElement: () => ({props: {}, children: [], layoutBox: {width: 0, height: 0}}),
      createText: text => ({text}), createComment: text => ({text}),
      insert(child, parent) { parent.children.push(child); }, remove() {},
      setText() {}, setElementText() {}, parentNode: () => null, nextSibling: () => null,
      patchProp(node, key, _old, value) { node.props[key] = value; },
    });
    const app = renderer.createApp(Component, {count: 1_000_000, itemExtent: 20, overscan: 0});
    app.mount({children: []});
    assert.ok(observed);
    observed.layoutBox.height = 100;
    callback();
    await nextTick();
    let vnode = app._instance.render.call(app._instance.proxy);
    assert.equal(vnode.children.filter(node => node.props.class.endsWith("__item")).length, 5);
    observed.layoutBox.height = 200;
    callback();
    await nextTick();
    vnode = app._instance.render.call(app._instance.proxy);
    assert.equal(vnode.children.filter(node => node.props.class.endsWith("__item")).length, 10);
    app.unmount();
    assert.equal(observed, null);
  } finally { globalThis.ResizeObserver = previous; }
});


test("frozen table automatically retains the active cell on both axes until focus and IME end", async () => {
  const Component = (await import(modules.NanaVirtualTable)).NanaVirtualTable;
  const renderer = createRenderer({
    createElement: () => ({props: {}, children: [], layoutBox: {width: 200, height: 100}, scrollTo() {}}),
    createText: text => ({text}), createComment: text => ({text}),
    insert(child, parent, anchor) {
      if (child.parent) child.parent.children.splice(child.parent.children.indexOf(child), 1);
      child.parent = parent;
      const at = anchor ? parent.children.indexOf(anchor) : -1;
      if (at < 0) parent.children.push(child); else parent.children.splice(at, 0, child);
    },
    remove(child) { child.parent.children.splice(child.parent.children.indexOf(child), 1); child.parent = null; },
    setText() {}, setElementText() {}, parentNode: node => node.parent,
    nextSibling: node => node.parent?.children[node.parent.children.indexOf(node) + 1] ?? null,
    patchProp(node, key, _old, value) { node.props[key] = value; },
  });
  const root = {children: []};
  const app = renderer.createApp(Component, {
    rowCount: 1000000, columnCount: 1000000, rowExtent: 20, columnExtent: 40,
    overscan: 0, frozenRows: 1, frozenColumns: 1,
  });
  const api = app.mount(root);
  await nextTick();
  const rows = () => root.children[0].children.filter(node => node.props.class === "nana-virtual-table__row");
  const cells = row => row.children.filter(node => node.props.class === "nana-virtual-table__cell");
  const count = () => rows().flatMap(cells).length;
  const editor = cells(rows()[2])[2];
  const input = {};
  editor.props.onFocusCapture({target: input});
  editor.props.onCompositionstartCapture({target: input});
  assert.equal(await api.scrollToCell(500000, 800000, "start"), true);
  assert.equal(count(), 36);
  assert.ok(rows().flatMap(cells).includes(editor));
  editor.props.onBlurCapture({target: {}});
  editor.props.onCompositionendCapture({target: input});
  await nextTick();
  assert.equal(count(), 36, "an unrelated blur cannot release the focused cell");
  editor.props.onBlurCapture({target: input});
  await nextTick();
  assert.equal(count(), 25);
  assert.ok(!rows().flatMap(cells).includes(editor));
  app.unmount();
  assert.equal(root.children.length, 0);
});
