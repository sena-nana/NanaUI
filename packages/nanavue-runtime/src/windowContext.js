export function contextForWindow(windowId) {
  const id = Number(windowId || 0);
  if (id && typeof globalThis.__nanaGetWindowContext === "function") {
    const context = globalThis.__nanaGetWindowContext(id);
    if (context) return context;
  }
  return { window: globalThis.window, document: globalThis.document };
}
