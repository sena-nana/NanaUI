# Phase 4 evidence — LiliaGithub P1e RepoDetail subset + Release bytecode + components

> **入口更名（2026-08-06）**：原 `lilia-github-nana` 已删除，改用通用宿主 `nana-tauri-demo --project <tauri根> [--bundle …] [--entry …]`。业务 IIFE 来自外部 Tauri 工程（相对 `--project`）；NanaUI 不再内置 `fixtures/lilia-github`。见 `examples/nana-tauri-demo/README.md`。

Date: 2026-08-06

## Pipeline

```text
LiliaGithub NanaRepoPage (readme/files panels) + NanaButton
  + nanavue-components Appearance/Workspace
  + SecondaryPanel + @lilia/ui
    → Vite IIFE (inlineDynamicImports)
    → nana-ui-web-api shim
    → VueHost DOM hostOps + CSS inject
    → forced mock workspace transport (PermissionPolicy / Rust host)
    → paint-stub → evidence PNG
```

## This slice (Issue #5 MVP 收口)

1. **#11 Release 不明文 JS**：`nana-tauri-demo --bytecode`（shell 子集）+ `--bytecode-file` 离线 embed；文档化业务 artifact 编译→宿主加载
2. **#8 nanavue-components**：`NanaThemeToggle` / `NanaAppearancePanel` / `NanaWorkspaceShell` / `NanaSidebarNav`；shell + Repo 路径使用；`examples/appearance-workspace.js`
3. **RepoDetail 子集**：`NanaRepoPage` 默认 README 面板 + 可切换 文件 面板；双引擎 PNG 字节一致
4. PermissionPolicy / paint-stub / wgpu 30 边界不变

## Results (this slice)

| Engine | Page | theme | boxes | key texts | PNG SHA-256 |
|--------|------|-------|------:|-----------|-------------|
| QuickJS | repo | light | 36 | nana-demo/NanaUI · README · # NanaUI | `100ec1e6e445…` |
| V8 | repo | light | 36 | **byte-identical** | `100ec1e6e445…` |

**Release bytecode E2E (shell)**：

- `home --complete-setup --bytecode --bytecode-source=shell` → `artifact=QuickJsBytecode`，texts 含 `Workspace ready`
- `settings --bytecode --bytecode-source=shell` → Appearance / Theme / Light
- `nana-qjs-compile` → `--bytecode-file=target/lilia-github-shell.qbc` → `artifact=QuickJsBytecode`

Prior home/settings/profile × QJS≡V8 matrix unchanged.

## Commands

```bash
# 在 LiliaGithub 仓库内按该项目文档构建 IIFE（NanaUI 不再内置 fixtures）
cd ~/work/LiliaGithub && # … build → dist/lilia-github.iife.js

# RepoDetail subset × QuickJS / V8
cargo run -p nana-tauri-demo --release --features evidence-png --locked -- \
  --project ~/work/LiliaGithub \
  --bundle dist/lilia-github.iife.js \
  --entry __nanaLiliaRunRepo --page repo --repo-id=repo-1 --theme=light \
  --png=docs/performance/lilia-real-repo-light-quickjs.png
cargo run -p nana-tauri-demo --release --no-default-features \
  --features engine-v8,paint-stub,evidence-png --locked -- \
  --project ~/work/LiliaGithub \
  --bundle dist/lilia-github.iife.js \
  --entry __nanaLiliaRunRepo --page repo --repo-id=repo-1 --theme=light \
  --png=docs/performance/lilia-real-repo-light-v8.png

# Release bytecode（QuickJS；--in 为外部工程产物）
cargo run -p nana-js-quickjs --bin nana-qjs-compile --locked -- \
  --in ~/work/LiliaGithub/dist/lilia-github.iife.js \
  --out target/app.qbc --compose-shim --name app.qbc.js
```

## Issue MVP 完成度表

权威总表见 [`docs/vue-backend-deps.md`](../vue-backend-deps.md)「Issue #5 MVP 完成度总表」与
[`2026-08-06-issue5-final-acceptance.md`](2026-08-06-issue5-final-acceptance.md)。本页保留分片明细：

| Item | Status | Evidence |
|------|--------|----------|
| P0 / P1a / layout / theme force | **Done** | prior |
| P1b real Home + SecondaryPanel IIFE | **Done** | fixture + host |
| CJK / SVG / form focus | **Done** | Phase4 PNGs |
| home/settings × light/dark × QJS/V8 | **Done** | matrix prior slice |
| QJS ≡ V8 PNG | **Done** | SHA identity |
| light ≠ dark pixels | **Done** | SHA + corner RGBA |
| Interactive `--interact=` | **Done** | search + settings |
| Phase 3 counter | **Done** | `vue-counter` |
| `<nana-gpu>` host composite (not teal) | **Done** | ready Home `composited=1` |
| QuickJS bytecode Release（编译+加载冒烟） | **Done** | `nana-qjs-compile` + lib test |
| QuickJS bytecode **宿主 E2E**（counter） | **Done** | `release_bytecode_…` + `--bytecode` |
| QuickJS bytecode **Lilia shell E2E** | **Done** | `nana-tauri-demo --bytecode` + `--bytecode-file` |
| V8 snapshot Release | **Done** | host-free probe path |
| Rust `PermissionPolicy` unit | **Done** | `capabilities` lib tests |
| Permission **integration**（JS bridge deny） | **Done** | `vue_host_denies_privileged_ops_without_grant` |
| Profile / Repo route skeletons | **Done** | dual-engine PNG |
| RepoDetail **readme/files 子集** | **Done** | NanaRepoPage + QJS≡V8 PNG |
| nanavue-components Appearance/Workspace + Button/Chip | **Done** | ThemeToggle/AppearancePanel/WorkspaceShell/SidebarNav/Button/Chip |
| Lucide multi-path / stroke | **Partial→Improved** | stub 点描；vello 曲线描边 |
| oklch sampling | **Partial→Improved** | approx parser；color-mix 仍跳过 |
| Full Blitz Vello wgpu 30 paint | **Phase B Done** | 文本/CJK/SVG + HostTexture；QJS≡V8；见 `paint-vello-phase-b.md`（Phase A fills 仅存档） |
| Android ARM64 / NDK | **Cross-built** | NDK r27 + `nana-android-host` `.so`；见 `docs/android-arm64.md` |
| Full Profile editor / full RepoDetail workbench | **P2** | readme/files 子集已验收；完整 SFC 仍 Tauri |

## Paint / Vello 签字（MVP #6）

- **Phase B complete**（签字基准）：`VelloPaintBackend` on host wgpu **30** via patched `euclio/vello` `wgpu30`（linebender PR #1754）；fills + fontdb/ab_glyph 文本/CJK + Lucide SVG 描边；QJS≡V8 PNG。
- Phase A（fills-only）见 [`2026-08-06-paint-vello-phase-a.md`](2026-08-06-paint-vello-phase-a.md)，**勿再将表内状态写为 Phase A Done**。
- Evidence: `vue-counter-vello-{quickjs,v8}.png` 与 `lilia-home-vello-*.png`（引擎间字节一致）。
- **Do not** enable crates.io `blitz-renderer-vello` / `anyrender_vello` (wgpu 29) or downgrade NanaUI wgpu.
- `paint-stub` remains the default app feature；不是 vello 的静默 fallback。
- P2：fork anyrender / 完整 stylo paint（border-radius / Parley / images）。Issue #5 #6「部分」= 保真 gap，**不阻塞** MVP 签字。

## Android

NDK + linker 已脚本化；交叉编译 `libnana_android_host.so`（aarch64）为 MVP #10 证据。
真机/模拟器 APK 运行仍为 P2。详见 [`docs/android-arm64.md`](../android-arm64.md)。

## 相关文档

- [`2026-08-06-issue5-final-acceptance.md`](2026-08-06-issue5-final-acceptance.md) — 最终验收命令
- [`docs/release-artifacts.md`](../release-artifacts.md) — bytecode / snapshot 合同
- [`packages/nanavue-components/README.md`](../../packages/nanavue-components/README.md)
- [`examples/nana-tauri-demo/README.md`](../../examples/nana-tauri-demo/README.md)
