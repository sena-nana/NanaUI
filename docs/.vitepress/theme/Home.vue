<script setup lang="ts">
import { computed } from "vue";
import { useData, withBase } from "vitepress";

import pipelineEn from "../../assets/diagrams/architecture-pipeline.en.svg";
import pipelineZh from "../../assets/diagrams/architecture-pipeline.zh-CN.svg";

import workspaceLight from "../../../examples/component-gallery/snapshots/metal-apple-m4/gallery-workspace-light.png";
import workspaceDark from "../../../examples/component-gallery/snapshots/metal-apple-m4/gallery-workspace-dark.png";
import controlsLight from "../../../examples/component-gallery/snapshots/metal-apple-m4/gallery-controls-light.png";
import controlsDark from "../../../examples/component-gallery/snapshots/metal-apple-m4/gallery-controls-dark.png";
import richTextLight from "../../../examples/component-gallery/snapshots/metal-apple-m4/gallery-rich-text-light.png";
import richTextDark from "../../../examples/component-gallery/snapshots/metal-apple-m4/gallery-rich-text-dark.png";
import dataLight from "../../../examples/component-gallery/snapshots/metal-apple-m4/gallery-data-light.png";
import dataDark from "../../../examples/component-gallery/snapshots/metal-apple-m4/gallery-data-dark.png";
import paletteLight from "../../../examples/component-gallery/snapshots/metal-apple-m4/gallery-command-palette-light.png";
import paletteDark from "../../../examples/component-gallery/snapshots/metal-apple-m4/gallery-command-palette-dark.png";

const { lang } = useData();
const zh = computed(() => lang.value.startsWith("zh"));
const prefix = computed(() => (zh.value ? "/zh" : ""));
const link = (path: string) => withBase(prefix.value + path);

const copy = {
  en: {
    accent: "Retained-mode",
    rest: "native desktop UI",
    description:
      "A Rust UI framework where live GPU content is an ordinary node in one UI tree. Your application owns the Window, Surface and GPU device.",
    start: "Get started",
    quick: "Quick start",
    components: "Components",
    architecture: "Architecture",
    heroCaption: "NanaUI Gallery — Workspace, Dock, Inspector & live theme switching (Metal, Apple M4)",
    metrics: [
      { value: "52 KiB", label: "500-level stack", note: "Heap-iterative measure & mount, zero stacker" },
      { value: "943 B / box", label: "21k wide live heap", note: "Compacted NodeRecord & paged node slots" },
      { value: "5.56 MiB", label: "dist binary size", note: "-64.8% via precompiled MSL & LTO profile" },
      { value: "0.3 µs", label: "Pointer dispatch", note: "Constant time across 250–2,000 rows" },
    ],
    pillarsHeading: "Built as one coherent system",
    pillars: [
      {
        title: "One retained tree",
        body: "Controls, text and live GPU nodes share <code>UiWorld</code>, a pure <code>Flex + Grid (+ subgrid) + Inline + Layers + Overlay</code> layout engine, one hit-test index and incremental AccessKit.",
      },
      {
        title: "Host-owned GPU & precompiled shaders",
        body: "The application holds the single WGPU device and each frame's <code>FrameContext</code>. Built-in shaders load precompiled MSL or compacted Naga IR, with zero CPU readback on product paths.",
      },
      {
        title: "Three authoring styles, one mount",
        body: "Plain Rust builders, <code>view!</code> / <code>css!</code> / <code>stylesheet!</code> macros and <code>.vue</code> SFCs with Rust scripts compile at build time into the same Style Model and <code>UiWorld</code>.",
      },
      {
        title: "Work proportional to change",
        body: "Clean frames do no work, compositor animations stay on <code>PresentationStore</code> overlays, and <code>WorkCounters</code> enforce incremental invariants in CI with zero per-node bloat.",
      },
    ],
    diagramTitle: "Architecture at a glance",
    diagramSub: "Build-time CSS compilation feeds a strictly acyclic four-stage pipeline into the host-owned Surface.",
    diagramAlt: "NanaUI Architecture Pipeline",
    diagramLinkArch: "Read Architecture overview",
    diagramLinkLearn: "Explore 25-crate Architecture Deep Dive",
    showcaseTitle: "Native controls & rich content",
    showcaseSub: "89 built-in controls, vertical/BiDi text, native KaTeX & 34 Mermaid diagrams, GPU charts and modal overlays — with no embedded browser.",
    showcase: [
      { title: "Controls & validation", caption: "Size tiers, forms, selection, sliders and 2D pads", light: controlsLight, dark: controlsDark },
      { title: "Rich text & native diagrams", caption: "Vertical writing, CommonMark, KaTeX math and native Mermaid", light: richTextLight, dark: richTextDark },
      { title: "Data visualization", caption: "ECharts-style GPU charts and calendar heatmaps", light: dataLight, dark: dataDark },
      { title: "Command palette & overlays", caption: "Modal scrim isolation, keyboard actions and focus return", light: paletteLight, dark: paletteDark },
    ],
    exploreTitle: "Explore the documentation",
    explore: [
      { title: "Guide", body: "Signals, view syntax, layouts, .vue SFCs, hot reload and packaging.", path: "/guide/introduction" },
      { title: "89 Components", body: "Controls, forms, rich text, charts, graphs, overlays and desktop shells.", path: "/components/" },
      { title: "Architecture", body: "The 8-step frame lifecycle, HostTexture/GpuView, nana-text and windows.", path: "/architecture/" },
      { title: "Architecture Deep Dive", body: "All 25 crates, 58 acyclic dependency edges and source-line anchors.", path: "/learn/" },
      { title: "Reference", body: "Application API, Style Model CSS subset, resource budgets and Taffy benchmarks.", path: "/reference/how-it-works" },
    ],
  },
  zh: {
    accent: "保留式",
    rest: "原生桌面界面",
    description: "把实时 GPU 画面作为普通节点放进同一棵界面树的 Rust UI 框架。窗口、Surface 与 GPU 设备归你的应用所有。",
    start: "开始",
    quick: "快速开始",
    components: "组件目录",
    architecture: "架构",
    heroCaption: "NanaUI Gallery — Workspace、Dock、检查器与明暗主题切换（Metal，Apple M4）",
    metrics: [
      { value: "52 KiB", label: "500 层深树线程栈", note: "堆上显式迭代测量与挂载，零 stacker" },
      { value: "943 B / 盒", label: "21k 宽页常驻堆", note: "紧凑化 NodeRecord 与分页节点槽位" },
      { value: "5.56 MiB", label: "dist 分发体积", note: "预编译 MSL + LTO 较基线缩减 64.8%" },
      { value: "0.3 µs", label: "指针移动派发", note: "250 至 2 000 行耗时恒定" },
    ],
    pillarsHeading: "作为同一套系统协同工作",
    pillars: [
      {
        title: "一棵权威保留树",
        body: "控件、文字与实时 GPU 画面共用 <code>UiWorld</code>、纯 <code>Flex + Grid (+ subgrid) + Inline + Layers + Overlay</code> 布局引擎、同一套命中索引与增量 AccessKit。",
      },
      {
        title: "宿主拥有 GPU 与预编译着色器",
        body: "进程唯一的 WGPU 设备与每帧 <code>FrameContext</code> 归应用所有；内置着色器默认加载预编译 MSL 或紧致 Naga IR，产品路径零 CPU 回读。",
      },
      {
        title: "三种写法，同一次挂载",
        body: "Rust 视图函数、<code>view!</code> / <code>css!</code> / <code>stylesheet!</code> 宏与 Rust 脚本的 <code>.vue</code> 单文件组件均在构建期编译为相同的 Style Model 与 <code>UiWorld</code>。",
      },
      {
        title: "工作量与改动成正比",
        body: "无变更的帧不做功，合成器动画留在 <code>PresentationStore</code> overlay 上，零节点开销的 <code>WorkCounters</code> 在 CI 中守住增量预算。",
      },
    ],
    diagramTitle: "架构一览",
    diagramSub: "构建期编译 CSS 样式，沿严格无环的四层产品管线绘制进宿主持有的 Surface。",
    diagramAlt: "NanaUI 架构总览图",
    diagramLinkArch: "阅读架构总览",
    diagramLinkLearn: "深入 25 crate 架构学习",
    showcaseTitle: "原生控件与富内容展示",
    showcaseSub: "89 个内置控件、竖排与双向文本、原生 KaTeX 与 34 种 Mermaid 图表、GPU 图表与模态浮层——全程无内嵌浏览器。",
    showcase: [
      { title: "控件与表单校验", caption: "三档尺寸、表单校验、选择控件与滑块", light: controlsLight, dark: controlsDark },
      { title: "富文本与原生图表", caption: "竖排正文、CommonMark、KaTeX 公式与 Mermaid", light: richTextLight, dark: richTextDark },
      { title: "数据可视化", caption: "ECharts 风格 GPU 图表与日历热力图", light: dataLight, dark: dataDark },
      { title: "命令面板与浮层", caption: "模态遮罩隔离、键盘快捷键与焦点归还", light: paletteLight, dark: paletteDark },
    ],
    exploreTitle: "浏览文档板块",
    explore: [
      { title: "指南", body: "信号、视图语法、布局、.vue 组件、热重载与资源打包。", path: "/guide/introduction" },
      { title: "89 个内置控件", body: "输入、表单、富文本、图表、节点图、浮层与桌面壳层。", path: "/components/" },
      { title: "架构", body: "一帧 8 步生命周期、HostTexture/GpuView、nana-text 与原生窗口。", path: "/architecture/" },
      { title: "架构学习", body: "25 个 crate 全覆盖、58 条依赖边实读与源码行号锚点。", path: "/learn/" },
      { title: "参考合同", body: "应用 API、CSS 子集布局、资源预算与 Taffy 对照基准。", path: "/reference/how-it-works" },
    ],
  },
};

const t = computed(() => (zh.value ? copy.zh : copy.en));
const diagramSrc = computed(() => (zh.value ? pipelineZh : pipelineEn));
</script>

<template>
  <section id="hero">
    <h1 class="tagline">
      <span class="accent">{{ t.accent }}</span>
      <br />{{ t.rest }}
    </h1>
    <p class="description">{{ t.description }}</p>
    <p class="actions">
      <a class="get-started" :href="link('/guide/introduction')">
        {{ t.start }}
        <svg class="icon" xmlns="http://www.w3.org/2000/svg" width="10" height="10" viewBox="0 0 24 24">
          <path d="M13.025 1l-2.847 2.828 6.176 6.176h-16.354v3.992h16.354l-6.176 6.176 2.847 2.828 10.975-11z" />
        </svg>
      </a>
      <a class="setup" :href="link('/guide/quick-start')">{{ t.quick }}</a>
      <a class="setup" :href="link('/components/')">{{ t.components }}</a>
      <a class="setup" :href="link('/architecture/')">{{ t.architecture }}</a>
    </p>

    <div class="hero-shot">
      <img class="shot-light" :src="workspaceLight" :alt="t.heroCaption" />
      <img class="shot-dark" :src="workspaceDark" :alt="t.heroCaption" />
      <p class="shot-caption">{{ t.heroCaption }}</p>
    </div>
  </section>

  <section id="metrics" class="home-section">
    <div class="metrics-grid">
      <a v-for="m in t.metrics" :key="m.label" class="metric-card" :href="link('/reference/resource-budgets')">
        <div class="metric-value">{{ m.value }}</div>
        <div class="metric-label">{{ m.label }}</div>
        <div class="metric-note">{{ m.note }}</div>
      </a>
    </div>
  </section>

  <section id="pillars" class="home-section">
    <h2 class="section-title">{{ t.pillarsHeading }}</h2>
    <div class="pillars-grid">
      <div v-for="p in t.pillars" :key="p.title" class="pillar-card">
        <h3>{{ p.title }}</h3>
        <p v-html="p.body"></p>
      </div>
    </div>
  </section>

  <section id="architecture-diagram" class="home-section">
    <h2 class="section-title">{{ t.diagramTitle }}</h2>
    <p class="section-sub">{{ t.diagramSub }}</p>
    <div class="diagram-frame">
      <img :src="diagramSrc" :alt="t.diagramAlt" />
    </div>
    <p class="diagram-links">
      <a :href="link('/architecture/')">{{ t.diagramLinkArch }} →</a>
      <a :href="link('/learn/')">{{ t.diagramLinkLearn }} →</a>
    </p>
  </section>

  <section id="showcase" class="home-section">
    <h2 class="section-title">{{ t.showcaseTitle }}</h2>
    <p class="section-sub">{{ t.showcaseSub }}</p>
    <div class="showcase-grid">
      <div v-for="item in t.showcase" :key="item.title" class="showcase-card">
        <div class="showcase-img-wrap">
          <img class="shot-light" :src="item.light" :alt="item.title" />
          <img class="shot-dark" :src="item.dark" :alt="item.title" />
        </div>
        <div class="showcase-meta">
          <h3>{{ item.title }}</h3>
          <p>{{ item.caption }}</p>
        </div>
      </div>
    </div>
  </section>

  <section id="explore" class="home-section explore-section">
    <h2 class="section-title">{{ t.exploreTitle }}</h2>
    <div class="explore-grid">
      <a v-for="card in t.explore" :key="card.path" class="explore-card" :href="link(card.path)">
        <h3>{{ card.title }}</h3>
        <p>{{ card.body }}</p>
      </a>
    </div>
  </section>
</template>

<style scoped>
#hero {
  padding: 72px 32px 24px;
  text-align: center;
  max-width: 1120px;
  margin: 0 auto;
}

.tagline {
  font-size: 72px;
  line-height: 1.12;
  font-weight: 900;
  letter-spacing: -1.5px;
  max-width: 960px;
  margin: 0 auto;
  border: 0;
  padding: 0;
}

.accent {
  background: linear-gradient(315deg, #7bb9f0 20%, #2c7ed6);
  background-clip: text;
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;
}

.description {
  max-width: 740px;
  line-height: 1.6;
  color: var(--vp-c-text-2);
  font-size: 20px;
  margin: 22px auto 32px;
}

.actions {
  margin: 0 0 36px;
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  gap: 12px;
}

.actions a {
  font-size: 15px;
  display: inline-flex;
  align-items: center;
  padding: 9px 18px;
  font-weight: 600;
  border-radius: 8px;
  text-decoration: none;
  transition: background-color 0.2s, color 0.2s, border-color 0.2s;
}

.actions .get-started {
  background: var(--vp-c-brand-1);
  color: var(--vp-c-brand-text, #fff);
}

.dark .actions .get-started {
  color: #0d1622;
}

.actions .setup {
  background: var(--vp-c-bg-soft);
  color: var(--vp-c-text-1);
  border: 1px solid var(--vp-c-divider);
}

.actions .get-started:hover {
  background: var(--vp-c-brand-2);
}

.actions .setup:hover {
  border-color: var(--vp-c-brand-1);
  background: var(--vp-c-bg-mute, var(--vp-c-bg-elv));
}

.icon {
  display: inline;
  margin-left: 6px;
  fill: currentColor;
  transition: transform 0.2s;
}

.get-started:hover .icon {
  transform: translateX(2px);
}

.hero-shot {
  margin: 0 auto;
  max-width: 980px;
}

.hero-shot img {
  width: 100%;
  border-radius: 12px;
  border: 1px solid var(--vp-c-divider);
  box-shadow: 0 18px 44px rgba(0, 0, 0, 0.16);
}

.dark .hero-shot img {
  box-shadow: 0 18px 48px rgba(0, 0, 0, 0.5);
}

.shot-caption {
  margin: 10px 0 0;
  font-size: 13px;
  color: var(--vp-c-text-3);
}

.shot-dark {
  display: none;
}

html.dark .shot-light {
  display: none;
}

html.dark .shot-dark {
  display: block;
}

.home-section {
  max-width: 1120px;
  margin: 0 auto;
  padding: 28px 32px;
}

.section-title {
  margin: 0 0 8px;
  padding: 0;
  border: 0;
  font-size: 26px;
  font-weight: 700;
  letter-spacing: -0.4px;
  text-align: center;
}

.section-sub {
  margin: 0 auto 24px;
  max-width: 720px;
  text-align: center;
  color: var(--vp-c-text-2);
  font-size: 15px;
  line-height: 1.65;
}

.metrics-grid {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 14px;
}

.metric-card {
  display: block;
  padding: 18px 18px 16px;
  border-radius: 10px;
  border: 1px solid var(--vp-c-divider);
  background: var(--vp-c-bg-soft);
  text-decoration: none;
  transition: border-color 0.2s, transform 0.2s;
}

.metric-card:hover {
  border-color: var(--vp-c-brand-1);
  transform: translateY(-2px);
}

.metric-value {
  font-size: 26px;
  font-weight: 800;
  letter-spacing: -0.5px;
  color: var(--vp-c-brand-1);
  line-height: 1.2;
}

.metric-label {
  margin-top: 6px;
  font-size: 14px;
  font-weight: 600;
  color: var(--vp-c-text-1);
}

.metric-note {
  margin-top: 4px;
  font-size: 12.5px;
  line-height: 1.5;
  color: var(--vp-c-text-2);
}

.pillars-grid {
  margin-top: 20px;
  display: grid;
  grid-template-columns: repeat(2, 1fr);
  gap: 16px;
}

.pillar-card {
  padding: 20px 22px;
  border-radius: 12px;
  border: 1px solid var(--vp-c-divider);
  background: var(--vp-c-bg-soft);
}

.pillar-card h3 {
  margin: 0 0 8px;
  font-size: 17px;
  font-weight: 650;
  color: var(--vp-c-text-1);
}

.pillar-card p {
  margin: 0;
  font-size: 14.5px;
  line-height: 1.68;
  color: var(--vp-c-text-2);
}

.pillar-card :deep(code) {
  font-size: 0.9em;
}

.diagram-frame {
  border-radius: 14px;
  overflow: hidden;
  border: 1px solid var(--vp-c-divider);
  box-shadow: 0 12px 32px rgba(0, 0, 0, 0.14);
}

.diagram-frame img {
  display: block;
  width: 100%;
}

.diagram-links {
  margin: 14px 0 0;
  display: flex;
  justify-content: center;
  gap: 24px;
  flex-wrap: wrap;
  font-size: 14px;
  font-weight: 600;
}

.diagram-links a {
  color: var(--vp-c-brand-1);
  text-decoration: none;
}

.diagram-links a:hover {
  text-decoration: underline;
}

.showcase-grid {
  display: grid;
  grid-template-columns: repeat(2, 1fr);
  gap: 18px;
}

.showcase-card {
  border-radius: 12px;
  border: 1px solid var(--vp-c-divider);
  background: var(--vp-c-bg-soft);
  overflow: hidden;
}

.showcase-img-wrap img {
  display: block;
  width: 100%;
  border-bottom: 1px solid var(--vp-c-divider);
}

.showcase-img-wrap .shot-dark {
  display: none;
}

html.dark .showcase-img-wrap .shot-light {
  display: none;
}

html.dark .showcase-img-wrap .shot-dark {
  display: block;
}

.showcase-meta {
  padding: 14px 18px 16px;
}

.showcase-meta h3 {
  margin: 0 0 4px;
  font-size: 15.5px;
  font-weight: 650;
  color: var(--vp-c-text-1);
}

.showcase-meta p {
  margin: 0;
  font-size: 13.5px;
  color: var(--vp-c-text-2);
}

.explore-section {
  padding-bottom: 64px;
}

.explore-grid {
  margin-top: 20px;
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(190px, 1fr));
  gap: 14px;
}

.explore-card {
  display: block;
  padding: 18px;
  border-radius: 10px;
  border: 1px solid var(--vp-c-divider);
  background: var(--vp-c-bg-soft);
  text-decoration: none;
  transition: border-color 0.2s, background-color 0.2s;
}

.explore-card:hover {
  border-color: var(--vp-c-brand-1);
  background: var(--vp-c-bg-mute, var(--vp-c-bg-elv));
}

.explore-card h3 {
  margin: 0 0 6px;
  font-size: 16px;
  font-weight: 650;
  color: var(--vp-c-brand-1);
}

.explore-card p {
  margin: 0;
  font-size: 13.5px;
  line-height: 1.6;
  color: var(--vp-c-text-2);
}

@media (max-width: 960px) {
  .tagline {
    font-size: 52px;
  }
  .metrics-grid {
    grid-template-columns: repeat(2, 1fr);
  }
  .pillars-grid,
  .showcase-grid {
    grid-template-columns: 1fr;
  }
}

@media (max-width: 576px) {
  #hero {
    padding: 44px 20px 16px;
  }
  .home-section {
    padding: 20px;
  }
  .tagline {
    font-size: 38px;
    letter-spacing: -0.8px;
  }
  .description {
    font-size: 16px;
    margin-bottom: 24px;
  }
  .metrics-grid {
    grid-template-columns: 1fr;
  }
}
</style>
