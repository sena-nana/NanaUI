# Issue #5 — 最终验收（Composer 复核 + 可复制命令）

> **入口更名（2026-08-06）**：原 `lilia-github-nana` 已删除，改用通用宿主 `nana-tauri-demo --project <tauri根> [--bundle …] [--entry …]`。业务 IIFE 来自外部 Tauri 工程（相对 `--project`）；NanaUI 不再内置 `fixtures/lilia-github`。见 `examples/nana-tauri-demo/README.md`。

Date: 2026-08-06

对照来源：[最终复核 Issue5 MVP](9919b3ef-c0f3-4a53-8446-3277abbc6870)（只读 12 项）+ 本工程收口（文档对齐、V8 单测根因修复）。

权威依赖/完成度：[`docs/vue-backend-deps.md`](../vue-backend-deps.md)。  
三层兼容目标合同：[`docs/vue-nana-renderer-system.md`](../vue-nana-renderer-system.md) §0。

---

## 三层兼容合同（Issue #5 落地范围 vs 目标）

用户设计意图：Issue #5 **应表示** NanaUI 具备三层兼容，并支持混合（尤其 L1+L2）。  
下表区分 **MVP 已签字落地** 与 **目标合同**（不以本表推翻 §0 签字）。

| 层 | 目标合同 | Issue #5 当前落地 | 说明 |
|----|----------|-------------------|------|
| **L1** 完整 Tauri Vue | Tauri 兼容 + 尽可能靠近原业务/布局 | **部分** | `nana-tauri-demo` + soft `tauriInvoke` + Custom Renderer + `css_map` 子集已有；**不是**完整 CSSOM，也不进 `nana-ui` 公共核心。#6「部分」即 L1 保真定界 |
| **L2** Nana 组件 × Vue | Vue 语义组件，可跳过 CSS 解析 | **已有（子集）** | `nanavue-components` + `createWidget` / `nana-*` → MessageBridge（验收 #5/#7/#8） |
| **L3** Rust NanaUI | 直接 Rust 布局 | **已有** | Gallery / `nana-ui` 不回退（验收 #1）；公共框架无 WebView/CSS/JS |
| **混合** L1+L2 | 同树共存 | **已有机制** | 同一 `MessageBridge`；历史称「C 混合」。壳级 L3 Region 深度插槽仍可加深 |

L1「完整 CSS」= 兼容目标（映射到 Nana **Style Model** = Tokens + Semantics + Layout），**禁止**把 CSSOM 塞进 `nana-ui`，也**不是**「CSS 全部变成 ThemeTokens」。目标合同细节见 [`vue-nana-renderer-system.md`](../vue-nana-renderer-system.md) §0。

---

## 0. 签字结论（Composer）

**Issue #5 MVP：完成 11 / 部分 1（#6 保真）/ 未做 0 — 可签字；#6 不阻塞。**

| # | MVP 验收项 | 状态 | 最新证据 |
|---|-----------|------|----------|
| 1 | 现有 Iced Gallery / CI 不回退 | **完成** | `component-gallery` + `.github/workflows/ci.yml` |
| 2 | `nana-ui-core` 独立编译/测试 | **完成** | 27 项单测；仅 serde |
| 3 | 标准 Vue 3 SFC + `<script setup lang="ts">` | **完成** | 外部 LiliaGithub IIFE（经 `nana-tauri-demo --project`） |
| 4 | Counter/Todo 双引擎（QuickJS + V8） | **完成** | `vue-counter`（Todo 无独立 example，Counter 覆盖双引擎） |
| 5 | Vue Custom Renderer → Rust DOM | **完成** | `nanavue-runtime` + `nana-ui-vue` renderer |
| 6 | Blitz / Vello / WGPU 布局与绘制 | **部分（定界更新）** | paint-stub + paint-vello Phase B 仍可用；**windowed 默认壳几何改 Nana Iced**（Blitz CSS 非默认）见 [`2026-08-06-skip-blitz-css-nana-shell.md`](./2026-08-06-skip-blitz-css-nana-shell.md) |
| 7 | 组件 / 响应式 / 事件 / class·style | **完成** | Phase3 + Phase4 `--interact=` |
| 8 | 主题 / 基础组件 / Workspace 示例 | **完成** | `nanavue-components` + Lilia shell |
| 9 | 自定义 WGPU 节点 | **完成** | `<nana-gpu>`；Home `composited=1` |
| 10 | 桌面 + Android ARM64 | **部分完成（宣称子集闭合）** | 交叉编译 + `AndroidShellStub` + slot（Icon/Text/Input/Switch/Button）+ Motion/KeyEvent + APK/模拟器绿；完整 DesktopShell / 软 IME **defer**（见 [`android-arm64.md`](../android-arm64.md)） |
| 11 | Release 不明文业务 JS | **完成** | QuickJS `--bytecode` / `--bytecode-file`；V8 host-free snapshot |
| 12 | UI 改外观不能绕过 Rust 权限 | **完成** | `PermissionPolicy` + JS bridge deny 集成测 |

**#6 不阻塞理由**：Issue 要求「完成布局和绘制」，非 WebView 像素级；Phase B 已满足可读 UI + 双引擎一致 + 宿主 wgpu 30。

三条并行完整实现线均已落地：paint-vello Phase B、Android ARM64 交叉编译宿主、Release bytecode + nanavue-components + Repo readme/files。

---

## 1. 最短最终验证命令（复核报告第三节）

工作目录：仓库根。

```bash
cd /Users/zt-c604184/Documents/workspace/sena-nana/NanaUI

# ── 核心回归（~2–5 min）──
cargo test -p nana-ui-core --locked
cargo test -p nana-ui-vue --lib capabilities --locked
cargo test -p nana-js-quickjs --lib --locked
# V8：须 --features engine；全量 lib 已串行化（见 §4）。仍可用规避命令复核：
cargo test -p nana-js-v8 --lib --features engine --locked
# 或：… compile_and_load_v8_snapshot_without_plaintext -- --exact
# 或：… -- --test-threads=1
cargo check -p component-gallery --locked
cargo check -p vue-counter --features evidence-png --locked
cargo check -p nana-tauri-demo --features evidence-png --locked

# ── Release 不明文 JS（--in 为外部工程产物）──
cargo run -p nana-js-quickjs --bin nana-qjs-compile --locked -- \
  --in ~/work/LiliaGithub/dist/lilia-github.iife.js \
  --out target/app.qbc --compose-shim --name app.qbc.js

# ── paint-vello Phase B + 双引擎一致（~5–10 min，需 release）──
cargo tree -p nana-ui-blitz --features paint-vello -i wgpu   # 期望仅 wgpu v30.0.0
cargo run -p vue-counter --release --no-default-features \
  --features engine-quickjs,paint-vello,evidence-png --locked -- \
  counter --clicks=1 --png=/tmp/vue-counter-vello-qjs.png
cargo run -p vue-counter --release --no-default-features \
  --features engine-v8,paint-vello,evidence-png --locked -- \
  counter --clicks=1 --png=/tmp/vue-counter-vello-v8.png
shasum -a 256 /tmp/vue-counter-vello-qjs.png /tmp/vue-counter-vello-v8.png  # 应相同

# ── Lilia home + Repo（先在 LiliaGithub 内构建 IIFE；NanaUI 无内置 fixtures）──
cargo run -p nana-tauri-demo --release --features evidence-png --locked -- \
  --project ~/work/LiliaGithub \
  --bundle dist/lilia-github.iife.js \
  --entry __nanaLiliaRunHome --page home --complete-setup --theme=light \
  --png=/tmp/lilia-home-qjs.png
cargo run -p nana-tauri-demo --release --no-default-features \
  --features engine-v8,paint-stub,evidence-png --locked -- \
  --project ~/work/LiliaGithub \
  --bundle dist/lilia-github.iife.js \
  --entry __nanaLiliaRunHome --page home --complete-setup --theme=light \
  --png=/tmp/lilia-home-v8.png
cargo run -p nana-tauri-demo --release --features evidence-png --locked -- \
  --project ~/work/LiliaGithub \
  --bundle dist/lilia-github.iife.js \
  --entry __nanaLiliaRunRepo --page repo --repo-id=repo-1 --theme=light \
  --png=/tmp/lilia-repo-qjs.png

# ── Android ARM64（需 NDK；bash source）──
# ./scripts/setup-android-ndk.sh          # 首次
bash -c 'source scripts/android-env.sh && ./scripts/check-android-arm64.sh --build'
file target-android/aarch64-linux-android/debug/libnana_android_host.so
```

**可选快速 PNG 对照（不重新跑）：**

```bash
shasum -a 256 \
  docs/performance/vue-counter-vello-*.png \
  docs/performance/lilia-home-vello-*.png \
  docs/performance/lilia-real-repo-light-*.png
```

验收路径须使用**互斥** `engine-quickjs` XOR `engine-v8`；勿 `cargo test --workspace --all-features`（example bin 会 `compile_error!`）。

---

## 2. 剩余 P2 清单（不阻塞 MVP 签字）

| 项 | 说明 |
|----|------|
| 完整 Blitz/stylo paint | CSS borders/radii/shadows/images；anyrender fork on wgpu30 |
| Parley / skrifa shaping | 仍用 fontdb/ab_glyph |
| oklch / color-mix 完整 | 构建期 neutralize + 运行时近似 |
| Lucide 像素级保真 | vello 曲线已优于 stub，≠ 浏览器 SVG |
| 完整 Profile 编辑器 | 骨架已验收 |
| 完整 RepoDetail workbench | readme/files 子集已验收；Diff/Actions 等仍 Tauri |
| 203 workspace Tauri commands | mock 覆盖 bootstrap/settings/create |
| AccessKit / a11y | blitz a11y 未启 |
| Android 真机/模拟器 APK | 交叉编译 + debug APK + **模拟器 KeyEvent 绿**；真机可选；完整 DesktopShell / 软 IME defer |
| Android V8 | 默认 QuickJS |
| CI Android matrix | 脚本已有，workflow 未接入 |
| `vite-plugin-nanavue` / `nanavue-cli` | JS stub；Release 入口为 Rust bin |
| 独立 `vue-todo` example | Issue 列举 Todo；仓库以 Counter 覆盖 |
| lazy route chunks | P1d 内联多页 |

---

## 3. 文档对齐备注

- paint 状态以 **Phase B** 为准（文本/CJK/SVG + QJS≡V8）；Phase A 仅为 fills 存档。
- `phase4-lilia-github-real.md` / `vue-backend-deps.md` 已与总表对齐。
- 分片证据：[`paint-vello-phase-b.md`](2026-08-06-paint-vello-phase-b.md)、[`android-arm64.md`](../android-arm64.md)、[`phase4-lilia-github-real.md`](2026-08-06-phase4-lilia-github-real.md)、[`release-artifacts.md`](../release-artifacts.md)。
- **宣称面扩展（2026-08-10）**：相对 Issue #5 home/settings 硬闸的增量合同 X1–X7 见 [`2026-08-10-lilia-fidelity-gap.md`](2026-08-10-lilia-fidelity-gap.md)；**不推翻**本节 MVP 签字，仅扩大后续可宣称边界。

---

## 4. V8 lib 单测 SIGSEGV（复核发现 → 已修）

**现象（复核机）**：`cargo test -p nana-js-v8 --lib --features engine` 全量 3 测间歇 **SIGSEGV**；单测 `compile_and_load_v8_snapshot_without_plaintext`（`--exact`）稳定通过；`--test-threads=1` 稳定通过。

**复现（本机收口）**：默认并行约 **8/20～7/30** 崩溃；`--test-threads=1` **0/20**；排除 snapshot 的两测并行 **0/30** → 根因为 **SnapshotCreator 与同进程其它 live isolate 并行**。

**修复（`crates/nana-js-v8/src/engine.rs`）**：

1. 进程级 gate：`compile_snapshot` 在无 live isolate 时独占；isolate create/drop 计数。
2. snapshot blob 在释放 gate 前拷贝出 `Vec<u8>`。
3. lib 测试 `with_serial_v8_tests` 串行化三测。

**回归**：修复后全量 lib **0/40** 失败（默认 test threads）。

**规避（旧二进制 / 未拉修复时）**：

```bash
cargo test -p nana-js-v8 --lib --features engine --locked \
  compile_and_load_v8_snapshot_without_plaintext -- --exact
# 或
cargo test -p nana-js-v8 --lib --features engine --locked -- --test-threads=1
```
