(function () {
  const host = globalThis.__nanaHost;
  const page = host.call("createWidget", ["nana-settings-page", {
    settings: { tabs: [{ id: "appearance", label: "Appearance" }], defaultTab: "appearance", hideHeader: true },
    "hide-header": true, "data-agent-id": "page"
  }]);
  const content = host.call("createWidget", ["column", { style: "width:100%;gap:16px;align-items:stretch" }]);
  host.call("insert", [page, host.call("mountRoot", []), null]);
  host.call("insert", [content, page, null]);
  for (let i = 0; i < 5; ++i) {
    const card = host.call("createWidget", ["nana-card", { style: "width:100%;height:80px", "data-agent-id": "card-" + i }]);
    const body = host.call("createWidget", ["column", { style: "width:100%;gap:8px;align-items:stretch" }]);
    const title = host.call("createWidget", ["text", { label: "Section " + (i + 1), style: "width:100%;font-family:sans-serif;white-space:nowrap", "data-agent-id": "title-" + i }]);
    host.call("insert", [card, content, null]);
    host.call("insert", [body, card, null]);
    host.call("insert", [title, body, null]);
  }
  globalThis.__nanaFireEvent = () => true;
  globalThis.spacingScroll = (id) => host.call("setScrollOffset", [id, 0, 10000]);
  globalThis.spacingPadding = (padding) => host.call("patchProp", [page, "content-padding", padding]);
})();
