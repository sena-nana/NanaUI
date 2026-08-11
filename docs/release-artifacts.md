# Release 产物：QuickJS bytecode 与 V8 snapshot（业务 JS 不明文）

Issue #5 MVP — 引擎原生二进制产物，**不可跨引擎互换**。

## 合同

| Kind | 引擎 | 说明 |
|------|------|------|
| `SourceUtf8` | QuickJS **或** V8 | 开发 / 双引擎对照（含完整 Lilia IIFE） |
| `QuickJsBytecode` | **仅** QuickJS | `Module::write` / `Module::load` |
| `V8Snapshot` | **仅** V8 | `SnapshotCreator::create_blob` / `CreateParams::snapshot_blob` |

硬约束：引擎互斥；二进制产物不可跨引擎加载。

## QuickJS bytecode

### 编译

```bash
# 业务 IIFE（可先 compose web-api shim）
cargo run -p nana-js-quickjs --bin nana-qjs-compile --locked -- \
  --in crates/nana-js-engine/fixtures/vue-runtime-probe/dist/vue-phase3.iife.js \
  --out target/vue-phase3.qbc \
  --compose-shim \
  --name vue-phase3.qbc.js
```

输出为非 UTF-8 明文的 bytecode blob（`RuntimeArtifactKind::QuickJsBytecode`）。

### 加载（可运行）

```rust
use nana_js_engine::RuntimeArtifact;
use nana_js_quickjs::QuickJsEngine;
use nana_js_engine::JsEngine;

let bytes = std::fs::read("target/vue-phase3.qbc")?;
let artifact = RuntimeArtifact::from_quickjs_bytecode("vue-phase3.qbc.js", bytes);
let mut engine = QuickJsEngine::new();
engine.initialize(artifact)?;
```

行为测试：

```bash
cargo test -p nana-js-quickjs --lib compile_and_load_quickjs_bytecode_without_plaintext --locked
```

`VueHost::initialize_with_web_api` 对 binary artifact **不再** prepend shim——编译前请 `--compose-shim` 或自行 compose。

### 宿主 E2E（Phase 3 counter）

```bash
# 1) 打包（compose web-api shim → QuickJsBytecode）
cargo run -p nana-js-quickjs --bin nana-qjs-compile --locked -- \
  --in crates/nana-js-engine/fixtures/vue-runtime-probe/dist/vue-phase3.iife.js \
  --out target/vue-phase3.qbc \
  --compose-shim \
  --name vue-phase3.qbc.js

# 2) 宿主以 bytecode 路径跑 counter（非 SourceUtf8）
cargo run -p vue-counter --release --locked -- counter --clicks=2 --bytecode
# 期望报告含 artifact=QuickJsBytecode 且 texts 含点击后的计数
```

库级回归：

```bash
cargo test -p nana-js-quickjs --lib release_bytecode_compose_shim_runs_phase3_counter --locked
```

### 宿主 E2E（通用 `nana-tauri-demo` + 业务 artifact）

生产编译路径：**业务 JS → `nana-qjs-compile --compose-shim` → `.qbc`**（不明文 eval）。
当前通用宿主 MVP 以 **`--project` + `--bundle`（UTF-8 IIFE）** 加载；bytecode 文件加载可按同样 CLI 扩展。

```bash
# A) 通用宿主：先在外部 Tauri 工程内构建 IIFE，再 --project 指向该根
#    （NanaUI 不再内置 fixtures/lilia-github）
cargo run -p nana-tauri-demo --release --locked -- \
  --project ~/work/LiliaGithub \
  --bundle dist/lilia-github.iife.js \
  --entry __nanaLiliaRunHome \
  --page home --complete-setup --window

# B) 离线编译 bytecode（packager；--in 为外部工程内产物）
cargo run -p nana-js-quickjs --bin nana-qjs-compile --locked -- \
  --in ~/work/LiliaGithub/dist/lilia-github.iife.js \
  --out target/app.qbc \
  --compose-shim \
  --name app.qbc.js
```

`nana-tauri-demo` 主要标志：

| Flag | 含义 |
|------|------|
| `--project <path>` | **必选** Tauri 项目根 |
| `--bundle <iife.js>` | 前端产物；缺省则读 `nana-demo.toml` / 探测 `dist` |
| `--entry <fn>` | boot 后调用的 JS 全局（或 `nana-demo.toml` `[pages]`） |
| `--page` / `--theme` / `--window` / `--headless` | 页面与宿主模式 |

V8 Release 继续用 host-free snapshot 路径（见下）。UTF-8 IIFE 双引擎对照与 Phase4 证据继续用 `SourceUtf8`。

## V8 snapshot

### 限制（引擎边界）

- Snapshot 内容必须是 **host-free**：顶层不得依赖 `__nanaHost` / external callbacks。
- 加载后由 `V8Engine::initialize(V8Snapshot)` 在已恢复的 isolate 上安装 host bridge。
- 完整 Lilia / Vue IIFE（挂载时即调 host）**不**适合整包 snapshot；双引擎对照与 Phase4 证据继续用 `SourceUtf8`。
- MVP 可运行路径：host-free probe → `compile_snapshot` → `initialize(V8Snapshot)` → `resolve_function` / `invoke`。

### 编译

```bash
cargo run -p nana-js-v8 --features engine --bin nana-v8-snapshot --locked -- \
  --in path/to/probe.js \
  --out target/probe.v8snap \
  --name probe.v8snap.js
```

库 API：

```rust
use nana_js_v8::V8Engine;
use nana_js_engine::JsEngine;

let artifact = V8Engine::compile_snapshot(
    "probe.v8snap.js",
    r#"globalThis.__nanaSnapshotProbe = { run: () => ({ ok: true, via: "v8-snapshot", n: 2 }) };"#,
)?;
let mut engine = V8Engine::new();
engine.initialize(artifact)?;
```

行为测试：

```bash
# 推荐：全量 lib（已串行化 SnapshotCreator；须 --features engine）
cargo test -p nana-js-v8 --lib --features engine --locked

# 规避（旧树或仍见并行崩溃时）：
cargo test -p nana-js-v8 --lib --features engine --locked \
  compile_and_load_v8_snapshot_without_plaintext -- --exact
cargo test -p nana-js-v8 --lib --features engine --locked -- --test-threads=1
```

说明：历史上全量 lib 在默认并行下可能对 SnapshotCreator×live isolate **SIGSEGV**；
根因与修复见 [`docs/performance/2026-08-06-issue5-final-acceptance.md`](performance/2026-08-06-issue5-final-acceptance.md) §4。

## nanavue-cli

`packages/nanavue-cli` 仍为 stub；当前可交付入口是 `nana-qjs-compile` 与 `nana-v8-snapshot`。
