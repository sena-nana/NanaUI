# Phase 4 evidence — LiliaGithub P0 shell (QuickJS vs V8)

> **入口更名（2026-08-06）**：原 `lilia-github-nana` 已删除，改用通用宿主 `nana-tauri-demo --project <tauri根> [--bundle …] [--entry …]`。业务 IIFE 来自外部 Tauri 工程（相对 `--project`）；NanaUI 不再内置 `fixtures/lilia-github`。见 `examples/nana-tauri-demo/README.md`。

Date: 2026-08-06

## Pipeline

```text
外部 Tauri 工程 IIFE（例：LiliaGithub dist/*.iife.js）
  → nana-ui-web-api shim compose
  → VueHost DOM hostOps
  → mock workspace transport (hasWindow/isDev, no Tauri)
  → paint-stub → evidence PNG
```

> **注（2026-08-06 后）**：NanaUI 已删除 `fixtures/lilia-github(-shell)`；下列历史命令改写为外部 `--project`。当时证据 PNG 仍保留。

## Commands

```bash
# 先在 LiliaGithub 内构建 IIFE，再：
cargo run -p nana-tauri-demo --release --features evidence-png -- \
  --project ~/work/LiliaGithub \
  --bundle dist/lilia-github.iife.js \
  --entry __nanaLiliaRunHome --page home --complete-setup \
  --png=docs/performance/lilia-home-light-quickjs.png
cargo run -p nana-tauri-demo --release --features evidence-png -- \
  --project ~/work/LiliaGithub \
  --bundle dist/lilia-github.iife.js \
  --entry __nanaLiliaRunSettings --page settings \
  --png=docs/performance/lilia-settings-light-quickjs.png

cargo run -p nana-tauri-demo --release --no-default-features \
  --features engine-v8,paint-stub,evidence-png -- \
  --project ~/work/LiliaGithub \
  --bundle dist/lilia-github.iife.js \
  --entry __nanaLiliaRunHome --page home --complete-setup \
  --png=docs/performance/lilia-home-light-v8.png
cargo run -p nana-tauri-demo --release --no-default-features \
  --features engine-v8,paint-stub,evidence-png -- \
  --project ~/work/LiliaGithub \
  --bundle dist/lilia-github.iife.js \
  --entry __nanaLiliaRunSettings --page settings \
  --png=docs/performance/lilia-settings-light-v8.png

# Phase 3 regression
cargo run -p vue-counter --release -- counter --clicks=2
cargo run -p vue-counter --release --no-default-features --features engine-v8,paint-stub -- todo
```

## Results (release, this machine)

| Engine | Page | boxes | texts (key) | cold_start_ms | mount_ms | PNG |
|--------|------|------:|-------------|--------------:|---------:|-----|
| QuickJS | home ready | 20 | LiliaGithub / Workspace ready / Status: ready | ~85–120 | ~10–14 | `lilia-home-light-quickjs.png` |
| V8 150.4.0 | home ready | 20 | same | 180.23 | 9.59 | `lilia-home-light-v8.png` |
| QuickJS | settings appearance | 33 | Appearance / Theme / Light / Dark / Corners | ~100–110 | ~15 | `lilia-settings-light-quickjs.png` |
| V8 150.4.0 | settings appearance | 33 | same | 78.39 | 10.87 | `lilia-settings-light-v8.png` |

Visual: QuickJS vs V8 PNG **byte-identical** (SHA-256 prefix match) for home and settings light theme.

`gpu_slots=1` on Home (`<nana-gpu data-slot="home-preview">`). `stylesheets=2` (Rust token inject + JS inject).

## JSON

See [`2026-08-06-phase4-lilia-github.json`](./2026-08-06-phase4-lilia-github.json).

## Acceptable differences vs LiliaGithub / LiliaUI

| Area | Nana P0（外部工程 IIFE） | Full LiliaGithub (Tauri/WebView) |
|------|-----------------|----------------------------------|
| UI kit | Structural Shell/Home/Settings replica + hex token subset | Full `@lilia/ui` SFC + oklch/color-mix tokens |
| Icons | Text labels (`Home`, `Set`) | Lucide SVG |
| CJK glyphs | Cell placeholders in paint-stub | System fonts |
| Transport | Forced mock (no Tauri) | mock in yarn dev / Tauri in prod |
| Router | location/history shim | vue-router + lazy chunks |

## Phase 3 regression

| Engine | App | Result |
|--------|-----|--------|
| QuickJS | counter clicks=2 | `texts=["2","inc"]` ok |
| V8 | todo | Todo list structure ok |
