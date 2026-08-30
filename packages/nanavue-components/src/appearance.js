/**
 * Nana adapters replacing Tauri native appearance / Lilia runtime installs.
 * Operates on web-api documentElement.dataset/style + localStorage.
 */
import { ref, computed } from "@vue/runtime-core";

const STORAGE_THEME = "lilia-github.theme";
const STORAGE_CORNER = "lilia-github.cornerStyle";
const STORAGE_CORNER_RADIUS = "lilia-github.cornerRadius";
const STORAGE_BACKDROP = "lilia-github.backdropMode";
const STORAGE_BACKDROP_TARGET = "lilia-github.backdropTarget";
const STORAGE_BACKDROP_OPACITY = "lilia-github.backdropOpacity";
const STORAGE_TITLEBAR_FOLLOW = "lilia-github.titlebarFollowsSidebar";
const STORAGE_WORKSPACE_CORNERS = "lilia-github.workspaceCorners";

export const CORNER_RADIUS_MIN = 8;
export const CORNER_RADIUS_MAX = 28;
export const CORNER_RADIUS_DEFAULT = 16;
export const BACKDROP_OPACITY_MIN = 0.28;
export const BACKDROP_OPACITY_MAX = 0.92;
export const BACKDROP_OPACITY_DEFAULT = 0.64;

let uiConfig = {
  appName: "LiliaGithub",
  productTitle: "LiliaGithub",
  version: "0.0.0-nana",
  identifier: "app.lilia.github.nana",
  storageKeyPrefix: "lilia-github",
  appearance: { backdropTarget: "sidebar", backdropOpacity: BACKDROP_OPACITY_DEFAULT },
};

let settingsModel = {
  path: "/settings",
  defaultTab: "appearance",
  description: "Manage appearance, account, workspace, and about.",
  aliases: {},
  hideHeader: false,
  fullPageTabs: [],
  sectionProps: {},
  tabs: [
    { key: "appearance", label: "Appearance" },
    { key: "account", label: "Account" },
    { key: "repositories", label: "Workspace" },
    { key: "about", label: "About" },
  ],
  sections: {},
};

const themeRef = ref("light");
const cornerRef = ref("round");
const cornerRadiusRef = ref(CORNER_RADIUS_DEFAULT);
const backdropRef = ref("solid");
const backdropTargetRef = ref("sidebar");
const backdropOpacityRef = ref(BACKDROP_OPACITY_DEFAULT);
const titlebarFollowRef = ref(true);
const workspaceCornersRef = ref(true);
const platformRef = ref("nana");
const contextMenuOpen = ref(false);

function docEl() {
  return globalThis.document?.documentElement;
}

function readStorage(key, fallback) {
  try {
    const v = globalThis.localStorage?.getItem(key);
    return v == null ? fallback : v;
  } catch (_err) {
    return fallback;
  }
}

function writeStorage(key, value) {
  try {
    globalThis.localStorage?.setItem(key, String(value));
  } catch (_err) {}
}

function clampCornerRadius(value) {
  const n = Number(value);
  if (!Number.isFinite(n)) return CORNER_RADIUS_DEFAULT;
  return Math.min(CORNER_RADIUS_MAX, Math.max(CORNER_RADIUS_MIN, Math.round(n)));
}

function clampOpacity(value) {
  const n = Number(value);
  if (!Number.isFinite(n)) return BACKDROP_OPACITY_DEFAULT;
  return Math.min(BACKDROP_OPACITY_MAX, Math.max(BACKDROP_OPACITY_MIN, n));
}

export function setLiliaUiConfig(config) {
  uiConfig = { ...uiConfig, ...config };
  return uiConfig;
}

export function getLiliaUiConfig() {
  return uiConfig;
}

export function provideLiliaSettings(_app, model) {
  settingsModel = { ...settingsModel, ...model };
  if (_app && typeof _app.provide === "function") {
    _app.provide("liliaSettings", settingsModel);
  }
  globalThis.__nanaLiliaSettings = settingsModel;
  return settingsModel;
}

export function getLiliaSettings() {
  return settingsModel;
}

const NATIVE_BACKDROPS = new Set(["mica", "acrylic", "vibrancy"]);

function normalizeBackdropMode(raw) {
  const value = String(raw || "solid").toLowerCase();
  if (value === "solid") return "solid";
  if (NATIVE_BACKDROPS.has(value)) return value;
  if (value === "translucent" || value === "transparent" || value === "system") {
    return "translucent";
  }
  return "solid";
}

export function backdropModeIsNative(mode) {
  return NATIVE_BACKDROPS.has(String(mode || "").toLowerCase());
}

function applyAppearance() {
  const el = docEl();
  if (!el) return;
  el.dataset.theme = themeRef.value;
  el.dataset.backdrop = backdropRef.value;
  el.dataset.backdropTarget = backdropTargetRef.value;
  el.dataset.titlebarFollowsSidebar = String(titlebarFollowRef.value);
  el.dataset.workspaceCorners = String(workspaceCornersRef.value);
  el.dataset.platform = platformRef.value;
  el.dataset.corners = cornerRef.value;
  if (el.style?.setProperty) {
    el.style.setProperty(
      "--lilia-backdrop-opacity",
      String(backdropOpacityRef.value),
    );
    el.style.setProperty("--app-corner-radius", `${cornerRadiusRef.value}px`);
  }
}

/** Nana replacement for installTauriNativeAppearanceAdapter + installNativeAppearance. */
export function installNativeAppearance() {
  themeRef.value = readStorage(STORAGE_THEME, "light") === "dark" ? "dark" : "light";
  backdropRef.value = normalizeBackdropMode(readStorage(STORAGE_BACKDROP, "solid"));
  const storedTarget = readStorage(
    STORAGE_BACKDROP_TARGET,
    uiConfig.appearance?.backdropTarget || "sidebar",
  );
  backdropTargetRef.value = storedTarget === "main" ? "main" : "sidebar";
  backdropOpacityRef.value = clampOpacity(
    readStorage(
      STORAGE_BACKDROP_OPACITY,
      uiConfig.appearance?.backdropOpacity ?? BACKDROP_OPACITY_DEFAULT,
    ),
  );
  titlebarFollowRef.value = readStorage(STORAGE_TITLEBAR_FOLLOW, "true") !== "false";
  workspaceCornersRef.value = readStorage(STORAGE_WORKSPACE_CORNERS, "true") !== "false";
  platformRef.value = "nana";
  applyAppearance();
  globalThis.__nanaOnTheme = function __nanaOnTheme(mode) {
    themeRef.value = String(mode || "light").toLowerCase() === "dark" ? "dark" : "light";
    writeStorage(STORAGE_THEME, themeRef.value);
    applyAppearance();
    return themeRef.value;
  };
  return {
    theme: themeRef,
    backdropMode: backdropRef,
    backdropTarget: backdropTargetRef,
    backdropOpacity: backdropOpacityRef,
    titlebarFollowsSidebar: titlebarFollowRef,
    workspaceCorners: workspaceCornersRef,
    platform: platformRef,
    setTheme(next) {
      themeRef.value = next === "dark" ? "dark" : "light";
      writeStorage(STORAGE_THEME, themeRef.value);
      // Writes documentElement.dataset.theme → web-api documentElementSet;
      // VueHost::semantic_snapshot syncs it into bridge ThemeMode (JS→Rust).
      // Rust→JS uses VueHost::inject_theme → __nanaApplyTheme → __nanaOnTheme.
      applyAppearance();
    },
    setBackdropMode(next) {
      backdropRef.value = normalizeBackdropMode(next);
      writeStorage(STORAGE_BACKDROP, backdropRef.value);
      applyAppearance();
    },
    setBackdropTarget(next) {
      backdropTargetRef.value = next === "main" ? "main" : "sidebar";
      writeStorage(STORAGE_BACKDROP_TARGET, backdropTargetRef.value);
      applyAppearance();
    },
    setBackdropOpacity(next) {
      backdropOpacityRef.value = clampOpacity(next);
      writeStorage(STORAGE_BACKDROP_OPACITY, backdropOpacityRef.value);
      applyAppearance();
    },
    setTitlebarFollowsSidebar(next) {
      titlebarFollowRef.value = !!next;
      writeStorage(STORAGE_TITLEBAR_FOLLOW, String(titlebarFollowRef.value));
      applyAppearance();
    },
    setWorkspaceCorners(next) {
      workspaceCornersRef.value = !!next;
      writeStorage(STORAGE_WORKSPACE_CORNERS, String(workspaceCornersRef.value));
      applyAppearance();
    },
  };
}

export function installCornerStyle() {
  const stored = readStorage(STORAGE_CORNER, "round");
  // Lilia uses smooth/round; legacy Nana sharp maps to smooth.
  cornerRef.value =
    stored === "smooth" || stored === "sharp" ? "smooth" : "round";
  cornerRadiusRef.value = clampCornerRadius(
    readStorage(STORAGE_CORNER_RADIUS, CORNER_RADIUS_DEFAULT),
  );
  applyAppearance();
  return {
    cornerStyle: cornerRef,
    cornerRadius: cornerRadiusRef,
    setCornerStyle(next) {
      // Accept Lilia smooth/round and legacy sharp → smooth.
      if (next === "sharp") cornerRef.value = "smooth";
      else cornerRef.value = next === "smooth" ? "smooth" : "round";
      writeStorage(STORAGE_CORNER, cornerRef.value);
      applyAppearance();
    },
    setCornerRadius(next) {
      cornerRadiusRef.value = clampCornerRadius(next);
      writeStorage(STORAGE_CORNER_RADIUS, cornerRadiusRef.value);
      applyAppearance();
    },
  };
}

export function installGlobalScrollbarVisibility() {
  if (typeof globalThis.ResizeObserver === "function" && globalThis.document?.body) {
    const ro = new globalThis.ResizeObserver(function () {});
    try {
      ro.observe(globalThis.document.body);
    } catch (_err) {}
    globalThis.__nanaScrollbarObserver = ro;
  }
  if (globalThis.window?.addEventListener) {
    globalThis.window.addEventListener("resize", function () {});
    globalThis.window.addEventListener("wheel", function () {}, { passive: true });
  }
  return function uninstall() {
    globalThis.__nanaScrollbarObserver?.disconnect?.();
  };
}

export function installLiliaContextMenu(app) {
  contextMenuOpen.value = false;
  const state = {
    open: false,
    x: 96,
    y: 96,
    anchorX: 96,
    anchorY: 96,
    items: [],
    searchable: false,
  };
  globalThis.__nanaContextMenuState = state;

  function openAt(x, y, items, options) {
    state.x = Number(x) || 96;
    state.y = Number(y) || 96;
    state.anchorX = state.x;
    state.anchorY = state.y;
    state.items = Array.isArray(items) ? items : [];
    state.searchable = !!options?.searchable || state.items.length >= 6;
    state.open = state.items.length > 0;
    contextMenuOpen.value = state.open;
  }

  function close() {
    state.open = false;
    state.items = [];
    contextMenuOpen.value = false;
  }

  function select(value) {
    const flat = [];
    const walk = (items, prefix) => {
      for (const item of items || []) {
        const id = item.id ?? item.label;
        const path = prefix ? `${prefix}/${id}` : String(id);
        flat.push({ path, item });
        if (item.children?.length) walk(item.children, path);
      }
    };
    walk(state.items, "");
    const hit = flat.find((f) => f.path === value || f.item.label === value);
    close();
    if (hit?.item?.onSelect) void hit.item.onSelect();
  }

  if (app && typeof app.directive === "function") {
    const directive = {
      mounted(el, binding) {
        el.__nanaCtxProvider = binding.value;
        el.addEventListener("contextmenu", onContextMenu);
      },
      updated(el, binding) {
        el.__nanaCtxProvider = binding.value;
      },
      unmounted(el) {
        el.removeEventListener("contextmenu", onContextMenu);
        delete el.__nanaCtxProvider;
      },
    };
    function onContextMenu(event) {
      const provider = event.currentTarget?.__nanaCtxProvider;
      if (!provider) return;
      event.preventDefault();
      const content = typeof provider === "function" ? provider(event) : provider;
      const items = Array.isArray(content) ? content : content?.items;
      if (!items?.length) return;
      openAt(event.clientX, event.clientY, items, content);
    }
    app.directive("contextMenu", directive);
    app.directive("context-menu", directive);
  }

  globalThis.__nanaContextMenu = {
    open: openAt,
    openAt,
    close,
    select,
    isOpen: () => contextMenuOpen.value,
    state,
  };

  // Prefer Nana Overlay path when Lilia composable is also present.
  try {
    const g = globalThis;
    if (typeof g.openContextMenuAt !== "function") {
      g.openContextMenuAt = openAt;
    }
    if (typeof g.closeContextMenu !== "function") {
      g.closeContextMenu = close;
    }
  } catch (_err) {}

  return globalThis.__nanaContextMenu;
}

export function useNativeAppearance() {
  return installNativeAppearance();
}

export function themeModeLabel() {
  return computed(() => (themeRef.value === "dark" ? "Dark" : "Light"));
}

export function resetAppearanceDefaults() {
  const appearance = installNativeAppearance();
  const corner = installCornerStyle();
  appearance.setTheme("light");
  appearance.setBackdropMode("solid");
  appearance.setBackdropTarget("sidebar");
  appearance.setBackdropOpacity(BACKDROP_OPACITY_DEFAULT);
  appearance.setTitlebarFollowsSidebar(true);
  appearance.setWorkspaceCorners(true);
  corner.setCornerStyle("round");
  corner.setCornerRadius(CORNER_RADIUS_DEFAULT);
}
