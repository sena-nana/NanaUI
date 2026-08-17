# Vue 兼容路线

## 目标

Nana 的 L1 是“WebView 中常见 Vue 3 + JavaScript 源码”的兼容子集：消费应用用
Nana 专用 Vite 入口构建 SFC、TypeScript 和 CSS，IIFE 在 QuickJS 或 V8 中执行，
Custom Renderer 把结果映射到 Nana Style Model，最终由 Runtime/UiScene 保留，
并由 `SceneWgpuPainter` 绘制。

三层输入仍可混合：

| 层 | 合同 | 状态 |
| --- | --- | --- |
| L1 | Vue SFC/TS/JS + DOM/CSS/Web API 子集 | 支持；边界见下文 |
| L2 | `nanavue-components` 语义组件 | 保留，可与 L1 同树 |
| L3 | Rust `nana-ui` | Runtime 入口；Scene host 绘制 |

## 明确非目标

- 真实 WebView 或以 WebView 绘制产品 UI；
- Tauri `invoke`、插件、窗口、事件、存储或权限协议；
- 未经 Nana 入口处理的 `@vue/runtime-dom` 生产 bundle 原样运行；
- 完整 DOM、CSSOM、浏览器布局、流式网络、cookie、cache、CORS、ServiceWorker、
  WebSocket；
- 外部 bundle loader、Tauri 项目探测器或兼容 Demo CLI。

仓库可选的 Hosted Browser 是应用显式承载外部网页的独立能力，不属于 Vue L1，
也不能用来替代 NanaUI 绘制。

## 当前可验收子集

### 源码入口

- Vue 3 SFC、`<script setup lang="ts">`、Composition API、template 与 CSS；
- `@nanaui/nanavue-runtime` 的 `createNanaApp()` Custom Renderer；
- QuickJS 与 V8 加载同一份可复现 IIFE；
- L1 `createElement` 与 L2 `createWidget` 写入同一 `MessageBridge`。

最小权威夹具：
[`crates/nana-js-engine/fixtures/vue-sfc-compat`](../crates/nana-js-engine/fixtures/vue-sfc-compat)。

### Web API

- `window`/`document`/EventTarget、存储、timer/rAF、history/location、clipboard 子集；
- 缓冲式 `fetch`、`Headers`、`Request`、`Response`、`text/json/arrayBuffer`、
  `bodyUsed/clone`、`AbortSignal`；
- HTTP 4xx/5xx 解析为正常 `Response`；网络、策略与资源限制错误拒绝 Promise；
- 应用通过精确 origin 白名单授权网络，默认全部拒绝。

Fetch 以 [WHATWG Fetch API](https://fetch.spec.whatwg.org/#fetch-api) 的对象形状为
参照，但只承诺上述缓冲式子集。非默认且未实现的 Request 选项直接抛出
`TypeError`，不做静默伪兼容。

### CSS 与布局

L1 CSS 继续只映射到 Style Model 的已记录子集。Flex、盒模型、有限 grid、级联、
选择器和布局边界以 [`css-layout-parity.md`](css-layout-parity.md) 为准；这不是完整
浏览器 CSS 引擎的路线图。

## 发布门禁

```bash
(cd crates/nana-js-engine/fixtures/vue-sfc-compat && npm ci && npm run build)
cargo test -p nana-ui-platform --lib --locked
cargo test -p nana-ui-web-api --lib --locked
cargo test -p nana-ui-vue --features scene-view --locked
cargo test -p nana-js-quickjs --lib --locked
cargo test -p nana-js-v8 --features engine --lib --locked -- --test-threads=1
cargo check -p vue-counter --all-targets --features windowed --locked
cargo check -p vue-counter --all-targets --no-default-features --features engine-v8,windowed --locked
./scripts/check-android-arm64.sh
```

没有视觉行为变化时不新增或更新视觉快照。SFC 构建门禁必须确认重建后的 `dist`
与提交产物一致。
