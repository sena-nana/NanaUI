# Vue 源码构建与运行时产物

## 支持的输入链

消费应用拥有 Vite 配置和入口：SFC/TypeScript/CSS 经
`@nanaui/nanavue-runtime` 的 `createNanaApp()` 构建为 Nana IIFE。NanaUI 不读取
Tauri 项目、不探测 `dist`、不提供外部 bundle loader，也不承诺普通
`@vue/runtime-dom` 产物可直接运行。

仓库的最小锁定夹具：

```bash
cd crates/nana-js-engine/fixtures/vue-sfc-compat
npm ci
npm run build
git diff --exit-code -- dist
```

夹具输出 `dist/vue-sfc-compat.iife.js` 与独立 CSS；V8 测试通过 Rust
`include_str!` 直接加载它们。这是兼容验收资产，不是应用 CLI。

## RuntimeArtifact

| Kind | 引擎 | 用途 |
| --- | --- | --- |
| `SourceUtf8` | V8 | 开发、SFC 兼容测试、顶层调用 host 的完整 Vue IIFE |
| `V8Snapshot` | V8 | host-free 启动快照（`nana-v8-snapshot`） |

QuickJS 与 `QuickJsBytecode` 已移除。发布时“编译字节码以隐藏 JS 源码”的 embed
能力随之消失：V8 snapshot **不是**通用源隐藏——`SnapshotCreator` 会执行源，且只
适用于顶层不调用 `__nanaHost` 的 host-free 脚本。挂载即调用 Custom Renderer 的
完整 Vue IIFE 继续使用 `SourceUtf8`。这不是兼容缺口，而是 V8 snapshot 创建阶段
的宿主回调边界。

引擎二进制不可互换。`VueHost::initialize_with_web_api` 只为 UTF-8 source 自动组合
Web API shim；二进制产物必须在生成前已经组合所需运行时。

Android ARM64 的 V8 静态库由 workflow `Package V8` 打包，消费方式见
[`android-arm64.md`](android-arm64.md)。

V8 snapshot：

```bash
cargo run -p nana-js-v8 --features engine --bin nana-v8-snapshot --locked -- \
  --in path/to/host-free.js \
  --out target/probe.v8snap --name probe.v8snap.js
```
