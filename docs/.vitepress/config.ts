import { defineConfig } from "vitepress";
import { apiStylePlugin } from "./plugins/api-style.ts";

const restoreStyle = `(function(){try{var s=localStorage.getItem('nanaui-docs-style');document.documentElement.classList.add(s==='rust'?'prefer-rust':'prefer-view')}catch(e){document.documentElement.classList.add('prefer-view')}})();`;

// Labels are written once in Chinese; `EN` gives the English text. English
// lives at the site root (sources in docs/en/), Chinese under /zh/ (sources
// at docs/<path> so existing repository links keep working).
const EN: Record<string, string> = {
  ".vue 方言": ".vue dialect",
  "CSS 引擎": "CSS engine",
  "GPU 与帧": "GPU and frames",
  "L3 建树": "L3 tree building",
  "Markdown 绘制": "Markdown rendering",
  "从哪里开始": "Where to start",
  "组件内容布局与迁移审计": "Component content layout and migration audit",
  "原生图表引擎": "Native diagram engine",
  "iOS 实验宿主": "iOS experimental host",
  "通用片元材质": "General fragment materials",
  "局部呈现": "Partial present",
  "移动宿主": "Mobile host",
  "Windows GPU 共享纹理输入": "Windows GPU shared-texture input",
  "Runtime 与 Scene": "Runtime and Scene",
  "Scene 提取": "Scene extraction",
  "一帧": "A frame",
  "两阶段启动": "Two-phase startup",
  "主题审计记录": "Theme audit log",
  "事件": "Events",
  "介绍": "Introduction",
  "从 Vue 过来": "Coming from Vue",
  "代码阅读指南": "Reading the code",
  "作者与工具": "Authoring and tools",
  "作者入口": "Authoring paths",
  "依赖提供": "Provide / inject",
  "侦听": "Watchers",
  "保留树": "Retained tree",
  "具名 slot": "Named slots",
  "内置": "Built-ins",
  "列表": "Lists",
  "创建应用": "Creating an app",
  "参考": "Reference",
  "双向绑定": "Two-way binding",
  "句柄": "Handles",
  "合同与不变量": "Contracts and invariants",
  "响应式": "Reactivity",
  "基础": "Essentials",
  "基础合同层": "Foundation contracts",
  "声明式视图": "Declarative views",
  "壳层": "Shell",
  "外围 crate": "Supporting crates",
  "实时画面": "Live GPU content",
  "宿主、绘制与分发": "Host, painting and distribution",
  "展示": "Display",
  "属性": "Props",
  "工作区": "Workspace",
  "工作台层": "Workspace, Shell and Dock",
  "工具链": "Toolchain",
  "布局与文本": "Layout and text",
  "布局与界面合同": "Layout and UI contracts",
  "布局引擎": "Layout engine",
  "布局结果合同": "Layout result contract",
  "布局（CSS 子集）": "Layout (CSS subset)",
  "布局（Rust）": "Layout (Rust)",
  "应用 API": "Application API",
  "应用与作者面": "Application and authoring",
  "开发期热重载": "Hot reload in development",
  "开始": "Getting started",
  "异步": "Async",
  "快速开始": "Quick start",
  "性能": "Performance",
  "总览": "Overview",
  "打包": "Packaging",
  "打包与分发": "Packaging and distribution",
  "扩展控件": "Custom controls",
  "挂载与卸载": "Mount and unmount",
  "指南": "Guide",
  "控件": "Controls",
  "操作与输入": "Actions and input",
  "文本": "Text",
  "文本引擎": "Text engine",
  "无障碍": "Accessibility",
  "条件": "Conditionals",
  "架构": "Architecture",
  "架构合同": "Architecture contracts",
  "架构学习": "Learn",
  "核心合同": "Core contracts",
  "框架如何运行": "How it works",
  "框架文案": "Framework strings",
  "模块与分层": "Modules and layers",
  "浮层": "Overlays",
  "渲染与宿主": "Rendering and host",
  "热重载": "Hot reload",
  "版式与组合": "Layout and composition",
  "相对几何": "Relative geometry",
  "示例": "Examples",
  "窗口": "Window",
  "窗口与平台": "Window and platform",
  "窗口外输出（ExternalSurface）": "Output outside the window (ExternalSurface)",
  "端到端数据流": "End-to-end flows",
  "第三方代码与许可证": "Third-party code and licenses",
  "类与样式": "Classes and styles",
  "组件": "Components",
  "组件深入": "Components in depth",
  "组合视图": "Composing views",
  "维护者": "Maintainers",
  "维护者笔记": "Maintainer notes",
  "表单绑定": "Form bindings",
  "表格与树": "Tables and trees",
  "覆盖矩阵": "Coverage matrix",
  "测试与门禁": "Testing and gates",
  "视图写法": "Writing views",
  "视觉": "Look",
  "计算值": "Computed values",
  "证据清单": "Evidence checklist",
  "诊断与应用路径": "Diagnostics and app paths",
  "诊断码": "Diagnostic codes",
  "贯穿视角": "Cross-cutting views",
  "资源预算": "Resource budgets",
  "Taffy 对照基准": "Taffy reference benchmark",
  "输入成本": "Input cost",
  "输入链路": "Input pipeline",
  "输入验收矩阵": "Input acceptance matrix",
  "进阶": "Scaling up",
  "附录": "Appendix"
};

type Lang = "en" | "zh";

const zhNav = [
  { text: "指南", link: "/guide/introduction", activeMatch: "/guide/" },
  { text: "组件", link: "/components/", activeMatch: "/components/" },
  { text: "架构", link: "/architecture/", activeMatch: "/architecture/" },
  { text: "架构学习", link: "/learn/", activeMatch: "/learn/" },
  {
    text: "参考",
    link: "/reference/how-it-works",
    activeMatch: "/reference/(?!theme|text-engine|input-conformance|input-cost|third-party|testing)",
  },
  {
    text: "维护者",
    link: "/reference/theme",
    activeMatch: "/reference/(theme|text-engine|input-conformance|input-cost|third-party|testing)",
  },
];

const zhSidebar = {
  "/guide/": [
    {
      text: "开始",
      items: [
        { text: "介绍", link: "/guide/introduction" },
        { text: "快速开始", link: "/guide/quick-start" },
        { text: "从 Vue 过来", link: "/guide/from-vue" },
        { text: "示例", link: "/guide/examples" },
      ],
    },
    {
      text: "基础",
      items: [
        { text: "创建应用", link: "/guide/essentials/application" },
        { text: "视图写法", link: "/guide/essentials/view" },
        { text: "响应式", link: "/guide/essentials/reactivity" },
        { text: "计算值", link: "/guide/essentials/computed" },
        { text: "侦听", link: "/guide/essentials/watch" },
        { text: "类与样式", link: "/guide/essentials/class-and-style" },
        { text: "条件", link: "/guide/essentials/conditional" },
        { text: "列表", link: "/guide/essentials/list" },
        { text: "事件", link: "/guide/essentials/events" },
        { text: "表单绑定", link: "/guide/essentials/forms" },
        { text: "句柄", link: "/guide/essentials/refs" },
        { text: "挂载与卸载", link: "/guide/essentials/lifecycle" },
      ],
    },
    {
      text: "组件深入",
      items: [
        { text: "组合视图", link: "/guide/essentials/view-functions" },
        { text: "版式与组合", link: "/guide/composition" },
        { text: "依赖提供", link: "/guide/components/provide" },
        { text: "扩展控件", link: "/guide/components/registry" },
        { text: "属性", link: "/guide/components/props" },
        { text: "事件", link: "/guide/components/events" },
        { text: "双向绑定", link: "/guide/components/v-model" },
        { text: "具名 slot", link: "/guide/components/slots" },
        { text: "异步", link: "/guide/components/async" },
      ],
    },
    {
      text: "内置",
      items: [
        { text: "Transition", link: "/guide/built-ins/transition" },
        { text: "KeepAlive", link: "/guide/built-ins/keep-alive" },
        { text: "Teleport", link: "/guide/built-ins/teleport" },
        { text: "Suspense", link: "/guide/built-ins/suspense" },
      ],
    },
    {
      text: "进阶",
      items: [
        { text: ".vue 方言", link: "/guide/scaling/sfc" },
        { text: "热重载", link: "/guide/scaling/hot-reload" },
        { text: "Store", link: "/guide/scaling/store" },
        { text: "打包", link: "/guide/scaling/packaging" },
        { text: "性能", link: "/guide/scaling/performance" },
        { text: "无障碍", link: "/guide/scaling/accessibility" },
        { text: "Fetch", link: "/guide/scaling/fetch" },
      ],
    },
  ],
  "/components/": [
    {
      text: "组件",
      items: [{ text: "总览", link: "/components/" }],
    },
    {
      text: "操作与输入",
      items: [
        { text: "Button", link: "/components/button" },
        { text: "IconButton", link: "/components/icon-button" },
        { text: "TextInput", link: "/components/text-input" },
        { text: "TextArea", link: "/components/text-area" },
        { text: "NumberInput", link: "/components/number-input" },
        { text: "ScrubField", link: "/components/scrub-field" },
        { text: "Checkbox", link: "/components/checkbox" },
        { text: "Switch", link: "/components/switch" },
        { text: "RangeField", link: "/components/range-field" },
        { text: "RangeSpanField", link: "/components/range-span-field" },
        { text: "CurveField", link: "/components/curve-field" },
        { text: "Select", link: "/components/select" },
        { text: "Dropdown", link: "/components/dropdown" },
        { text: "SearchDropdown", link: "/components/search-dropdown" },
        { text: "FindReplaceBar", link: "/components/find-replace-bar" },
        { text: "SegmentedControl", link: "/components/segmented-control" },
        { text: "Tabs", link: "/components/tabs" },
        { text: "XYPad", link: "/components/xy-pad" },
        { text: "ColorField", link: "/components/color-field" },
        { text: "PathField", link: "/components/path-field" },
        { text: "DatePicker", link: "/components/date-picker" },
        { text: "PropertyGrid", link: "/components/property-grid" },
        { text: "TimelineEditor", link: "/components/timeline-editor" },
      ],
    },
    {
      text: "布局与文本",
      items: [
        { text: "Text", link: "/components/text" },
        { text: "RichTextView", link: "/components/rich-text-view" },
        { text: "RichTextEditor", link: "/components/rich-text-editor" },
        { text: "Column", link: "/components/column" },
        { text: "Row", link: "/components/row" },
        { text: "Stack", link: "/components/stack" },
        { text: "Divider", link: "/components/divider" },
        { text: "IconGlyph", link: "/components/icon-glyph" },
        { text: "ScrollView", link: "/components/scroll-view" },
        { text: "heading_row", link: "/components/heading-row" },
        { text: "settings_group", link: "/components/settings-group" },
        { text: "scroll_foot_fade", link: "/components/scroll-foot-fade" },
      ],
    },
    {
      text: "表格与树",
      items: [
        { text: "Table", link: "/components/table" },
        { text: "TreeView", link: "/components/tree-view" },
        { text: "ReorderList", link: "/components/reorder-list" },
      ],
    },
    {
      text: "展示",
      items: [
        { text: "Card", link: "/components/card" },
        { text: "List", link: "/components/list" },
        { text: "ListItem", link: "/components/list-item" },
        { text: "FormField", link: "/components/form-field" },
        { text: "DynamicForm", link: "/components/dynamic-form" },
        { text: "InteractionRequestCard", link: "/components/interaction-request-card" },
        { text: "EmptyState", link: "/components/empty-state" },
        { text: "callout", link: "/components/callout" },
        { text: "Progress", link: "/components/progress" },
        { text: "Skeleton", link: "/components/skeleton" },
        { text: "Spinner", link: "/components/spinner" },
        { text: "StatusBadge", link: "/components/status-badge" },
        { text: "Chip", link: "/components/chip" },
        { text: "Avatar", link: "/components/avatar" },
        { text: "Thumbnail", link: "/components/thumbnail" },
        { text: "Texture", link: "/components/texture" },
        { text: "Tooltip", link: "/components/tooltip" },
        { text: "ValidationMessage", link: "/components/validation-message" },
        { text: "QrCode", link: "/components/qr-code" },
        { text: "ImageViewer", link: "/components/image-viewer" },
        { text: "NativeMarkdown", link: "/components/native-markdown" },
        { text: "CalendarHeatmap", link: "/components/calendar-heatmap" },
        { text: "Chart", link: "/components/chart" },
        { text: "GraphCanvas", link: "/components/graph-canvas" },
        { text: "GraphMinimap", link: "/components/graph-minimap" },
      ],
    },
    {
      text: "浮层",
      items: [
        { text: "Dialog", link: "/components/dialog" },
        { text: "ConfirmDialog", link: "/components/confirm-dialog" },
        { text: "Drawer", link: "/components/drawer" },
        { text: "Popover", link: "/components/popover" },
        { text: "ActionMenu", link: "/components/action-menu" },
        { text: "ContextMenu", link: "/components/context-menu" },
        { text: "AppMenuBar", link: "/components/app-menu-bar" },
        { text: "CommandPalette", link: "/components/command-palette" },
        { text: "Toast", link: "/components/toast" },
        { text: "ToastStack", link: "/components/toast-stack" },
      ],
    },
    {
      text: "壳层",
      items: [
        { text: "AppShell", link: "/components/app-shell" },
        { text: "DesktopShell", link: "/components/desktop-shell" },
        { text: "AppTitleBar", link: "/components/app-title-bar" },
        { text: "Toolbar", link: "/components/toolbar" },
        { text: "ToolGroup", link: "/components/tool-group" },
        { text: "StatusBar", link: "/components/status-bar" },
        { text: "MediaTransportBar", link: "/components/media-transport-bar" },
        { text: "Workspace", link: "/components/workspace" },
        { text: "SidebarFrame", link: "/components/sidebar-frame" },
        { text: "SidebarSection", link: "/components/sidebar-section" },
        { text: "SidebarRow", link: "/components/sidebar-row" },
        { text: "SettingsRow", link: "/components/settings-row" },
        { text: "Dock", link: "/components/dock" },
        { text: "AreaLayout", link: "/components/area-layout" },
        { text: "SplitPane", link: "/components/split-pane" },
        { text: "PaneChrome", link: "/components/pane-chrome" },
      ],
    },
  ],
  "/architecture/": [
    {
      text: "架构",
      items: [
        { text: "总览", link: "/architecture/" },
        { text: "一帧", link: "/architecture/frame" },
        { text: "实时画面", link: "/architecture/gpu" },
        { text: "文本", link: "/architecture/text" },
        { text: "窗口", link: "/architecture/window" },
      ],
    },
  ],
  "/learn/": [
    {
      text: "开始",
      items: [
        { text: "总览", link: "/learn/" },
        { text: "模块与分层", link: "/learn/modules-and-layers" },
      ],
    },
    {
      text: "基础合同层",
      items: [
        { text: "核心合同", link: "/learn/core-contracts" },
        { text: "CSS 引擎", link: "/learn/css-engine" },
        { text: "文本引擎", link: "/learn/text-engine" },
      ],
    },
    {
      text: "Runtime",
      items: [
        { text: "保留树", link: "/learn/runtime-tree" },
        { text: "布局引擎", link: "/learn/runtime-layout" },
        { text: "输入链路", link: "/learn/runtime-input" },
        { text: "工作台层", link: "/learn/workspace-shell-dock" },
      ],
    },
    {
      text: "渲染与宿主",
      items: [
        { text: "Scene 提取", link: "/learn/scene-extraction" },
        { text: "GPU 与帧", link: "/learn/gpu-frame" },
        { text: "窗口与平台", link: "/learn/window-platform" },
      ],
    },
    {
      text: "作者与工具",
      items: [
        { text: "作者入口", link: "/learn/authoring-paths" },
        { text: "工具链", link: "/learn/toolchain" },
        { text: "外围 crate", link: "/learn/supporting-crates" },
      ],
    },
    {
      text: "贯穿视角",
      items: [
        { text: "合同与不变量", link: "/learn/contracts-and-invariants" },
        { text: "端到端数据流", link: "/learn/flows" },
        { text: "代码阅读指南", link: "/learn/reading-the-code" },
      ],
    },
    {
      text: "附录",
      items: [
        { text: "覆盖矩阵", link: "/learn/coverage-matrix" },
        { text: "证据清单", link: "/learn/evidence-checklist" },
      ],
    },
  ],
  "/reference/": [
    {
      text: "应用与作者面",
      items: [
        { text: "从哪里开始", link: "/reference/start" },
        { text: "框架如何运行", link: "/reference/how-it-works" },
        { text: "应用 API", link: "/reference/application-api" },
        { text: "声明式视图", link: "/reference/reactive-view" },
        { text: "L3 建树", link: "/reference/l3-authoring" },
        { text: "框架文案", link: "/reference/framework-strings" },
        { text: "诊断码", link: "/reference/errors" },
      ],
    },
    {
      text: "布局与界面合同",
      items: [
        { text: "布局（CSS 子集）", link: "/reference/layout" },
        { text: "布局（Rust）", link: "/reference/rust-layout" },
        { text: "布局结果合同", link: "/reference/layout-result" },
        { text: "相对几何", link: "/reference/relative-geometry" },
        { text: "控件", link: "/reference/components" },
        { text: "InlineContent", link: "/reference/inline-content" },
        { text: "工作区", link: "/reference/workspace" },
        { text: "视觉", link: "/reference/look" },
        { text: "窗口", link: "/reference/window" },
        { text: "两阶段启动", link: "/reference/startup" },
      ],
    },
    {
      text: "宿主、绘制与分发",
      items: [
        { text: "实时画面", link: "/reference/gpu" },
        { text: "通用片元材质", link: "/reference/materials" },
        { text: "局部呈现", link: "/reference/partial-present" },
        { text: "窗口外输出（ExternalSurface）", link: "/reference/output" },
        { text: "Runtime 与 Scene", link: "/reference/runtime-scene" },
        { text: "架构合同", link: "/reference/architecture" },
        { text: "资源预算", link: "/reference/resource-budgets" },
        { text: "Taffy 对照基准", link: "/reference/taffy-reference" },
        { text: "Markdown 绘制", link: "/reference/markdown-rendering" },
        { text: "原生图表引擎", link: "/reference/diagram-engine" },
        { text: "诊断与应用路径", link: "/reference/diagnostics" },
        { text: "打包与分发", link: "/reference/packaging" },
        { text: "开发期热重载", link: "/reference/hot-reload" },
        { text: "Android", link: "/reference/android" },
        { text: "HarmonyOS / OpenHarmony", link: "/reference/ohos" },
        { text: "iOS 实验宿主", link: "/reference/ios" },
        { text: "移动宿主", link: "/reference/mobile-host" },
        { text: "Windows GPU 共享纹理输入", link: "/reference/native-input" },
      ],
    },
    {
      text: "维护者笔记",
      collapsed: true,
      items: [
        { text: "主题审计记录", link: "/reference/theme" },
        { text: "组件内容布局与迁移审计", link: "/reference/component-layout" },
        { text: "文本引擎", link: "/reference/text-engine" },
        { text: "输入验收矩阵", link: "/reference/input-conformance" },
        { text: "输入成本", link: "/reference/input-cost" },
        { text: "第三方代码与许可证", link: "/reference/third-party" },
        { text: "测试与门禁", link: "/reference/testing" },
      ],
    },
  ],
};

function localize<T>(value: T, lang: Lang): T {
  const prefix = lang === "zh" ? "/zh" : "";
  const walk = (v: any): any => {
    if (Array.isArray(v)) return v.map(walk);
    if (!v || typeof v !== "object") return v;
    const out: any = {};
    for (const [k, x] of Object.entries(v)) {
      if (k === "text" && typeof x === "string") out[k] = lang === "en" ? (EN[x] ?? x) : x;
      else if (k === "link" && typeof x === "string" && x.startsWith("/")) out[k] = prefix + x;
      else if (k === "activeMatch" && typeof x === "string") out[k] = prefix + x;
      else out[k] = walk(x);
    }
    return out;
  };
  return walk(value);
}

function sidebar(lang: Lang) {
  const prefix = lang === "zh" ? "/zh" : "";
  return Object.fromEntries(
    Object.entries(zhSidebar).map(([key, groups]) => [prefix + key, localize(groups, lang)]),
  );
}

// Absolute links in the Chinese sources (`/learn/gpu-frame`) are written
// against the Chinese tree; send them to /zh/ on the built site.
function zhAbsoluteLinks(md: any) {
  md.core.ruler.push("zh-absolute-links", (state: any) => {
    if (!String(state.env?.relativePath ?? "").startsWith("zh/")) return;
    for (const block of state.tokens) {
      for (const token of block.children ?? []) {
        if (token.type !== "link_open") continue;
        const href = token.attrGet("href");
        if (href && href.startsWith("/") && !href.startsWith("/zh/") && !href.startsWith("//")) {
          token.attrSet("href", "/zh" + href);
        }
      }
    }
  });
}

const github = "https://github.com/sena-nana/NanaUI";

export default defineConfig({
  base: "/NanaUI/",
  title: "NanaUI",
  cleanUrls: true,
  srcExclude: ["**/README.md"],
  rewrites(id) {
    if (id.startsWith("en/")) return id.slice(3);
    return "zh/" + id;
  },
  ignoreDeadLinks: [/^https?:\/\//, /archive\/docs-notes/, /\.\.\/\.\.\//],
  head: [["script", {}, restoreStyle]],
  markdown: {
    config(md) {
      apiStylePlugin(md);
      zhAbsoluteLinks(md);
    },
  },
  locales: {
    root: {
      label: "English",
      lang: "en-US",
      description: "A retained-mode Rust UI framework where live GPU content is an ordinary node in the UI tree.",
      themeConfig: {
        nav: localize(zhNav, "en"),
        sidebar: sidebar("en"),
        editLink: { pattern: `${github}/edit/main/docs/:path`, text: "Edit this page on GitHub" },
      },
    },
    zh: {
      label: "简体中文",
      lang: "zh-CN",
      link: "/zh/",
      description: "易上手的桌面界面框架。一棵界面树，窗口和 GPU 由你的应用持有。",
      themeConfig: {
        nav: localize(zhNav, "zh"),
        sidebar: sidebar("zh"),
        outline: { label: "本页目录", level: [2, 3] },
        docFooter: { prev: "上一页", next: "下一页" },
        sidebarMenuLabel: "菜单",
        darkModeSwitchLabel: "外观",
        returnToTopLabel: "回到顶部",
        langMenuLabel: "切换语言",
        lastUpdatedText: "最后更新",
        editLink: { pattern: `${github}/edit/main/docs/:path`, text: "在 GitHub 上编辑此页" },
      },
    },
  },
  themeConfig: {
    search: {
      provider: "local",
      options: {
        locales: {
          zh: {
            translations: {
              button: { buttonText: "搜索", buttonAriaLabel: "搜索" },
              modal: {
                noResultsText: "没有结果",
                resetButtonTitle: "清除",
                footer: { selectText: "选择", navigateText: "切换", closeText: "关闭" },
              },
            },
          },
        },
      },
    },
    outline: { level: [2, 3] },
    socialLinks: [{ icon: "github", link: github }],
  },
});
