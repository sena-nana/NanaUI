---
title: Reading the Code
---

# Reading the Code

This chapter answers a practical question: **faced with a repository of 25 crates under `crates/` (24 workspace members plus standalone `nana-icons-tabler`), which file should you start with, in what order should you read, how do you run a minimal check, and at what point have you really mastered the main path**.

## Step one: read these five short documents in order

Before diving into source files thousands of lines long, spend 20 minutes building a global coordinate system in this order:

| Order | Document path | What it teaches you |
| --- | --- | --- |
| 1 | `AGENTS.md` (repository root) | The most condensed form of the repository's architecture contracts: the single product path, supporting boundaries, and the nine non-negotiable red lines |
| 2 | [`docs/reference/how-it-works.md`](/reference/how-it-works) | The core runtime model: one retained tree, the host owns the GPU, the 8-step lifecycle of a frame |
| 3 | [`docs/reference/architecture.md`](/reference/architecture) | Crate boundaries, the Motion IR routing table, and the ownership table for global objects |
| 4 | [Modules and Layers](/learn/modules-and-layers) | The dependency-depth levels of the 25 crates and an actual reading of the 58 workspace-member normal dependency edges |
| 5 | [Contracts and Invariants](/learn/contracts-and-invariants) | The design rationale behind the nine non-negotiable contracts, with read-only verification commands |

Keep `AGENTS.md` at hand while reading the source. Nearly every design in the repository of the "why is this done in two steps, why not just pass the handle" kind ultimately maps to one of its non-negotiable contracts.

## Repository map

```text
NanaUI/
├── Cargo.toml                  Workspace root manifest: 24 workspace member crates + tools + platform + 5 examples
├── AGENTS.md                   Architecture contracts and development rules (the most important root document)
├── crates/                     All 25 framework crates (24 members + nana-icons-tabler; full table at /learn/coverage-matrix)
├── examples/                   5 workspace example packages (component-gallery, native-rhi-probe,
│                               runtime-host-fixture, package-fixture, reactive-sfc)
├── crates/nana-ui/examples/    26 single-file minimal topic examples (counter, GPU view, transparent window, Splash, accessibility, and more)
├── docs/                       VitePress docs site source (including the :::api dual-authoring check script)
├── platform/android/           Experimental Android GameActivity host
├── tools/css-parity/           Test fixtures and comparison tools for CSS-subset layout parity
├── perf/                       Performance benchmark scenes and gate definitions
└── scripts/                    Architecture boundary and compliance check scripts (check-engine-boundary.py and others)
```

## Three recommended source reading routes

### Route 1: the rendering path of a frame (state → pixels)

1. Read "Flow 1" in [End-to-End Data Flow](/learn/flows) and the "A frame" section of [How the Framework Runs](/reference/how-it-works).
2. Read [Scene Extraction](/learn/scene-extraction). Corresponding source: `crates/nana-ui-scene/src/runtime_document.rs:109` (`RuntimeDocument::flush`) and `crates/nana-ui-scene/src/scene.rs:1672` (`UiScene::apply_delta`).
3. Read [GPU and Frame Lifecycle](/learn/gpu-frame). Corresponding source: `crates/nana-gpu/src/context.rs` (`GpuContext`), `crates/nana-gpu/src/frame.rs` (`FrameContext`), `crates/nana-ui/src/scene_paint/mod.rs` (`SceneWgpuPainter`), and `crates/nana-ui/src/scene_host/present.rs:114` (the 35-step total order of the host `redraw`).

### Route 2: the retained tree and incremental layout (mutation → geometry)

1. Read [Retained Tree: UiWorld](/learn/runtime-tree). Start with the `UiMutation` enum at `crates/nana-ui-runtime/src/mutation.rs:15` to see the full vocabulary of operations the retained tree allows, then go to `world/mutation.rs:2915` for `UiWorld::commit`.
2. Read [Layout Engine](/learn/runtime-layout). Start from `crates/nana-ui-runtime/src/layout_engine.rs:89` (`RuntimeLayoutEngine`) and `layout_frontier.rs` (dependency graph and incremental seed propagation) to see how incremental layout relays out only the affected subtrees.

### Route 3: input routing (native event → callback)

1. Read [Input Path](/learn/runtime-input).
2. Open `crates/nana-ui-input/src/canonical.rs:44` (`CanonicalInputEvent` and `InputEndpoint`), then jump straight to the repository's single routing entry point, `crates/nana-ui-runtime/src/framework/input.rs:397` (`AppContext::route_input`), and the four dispatch chains in `framework/input/dispatch.rs`.

## Run minimal examples and watch for yourself

Each single-file example under `crates/nana-ui/examples/*.rs` demonstrates exactly one feature, which makes them excellent entry points for reading code in motion:

```bash
# 1. Gallery of all built-in controls and the workspace
cargo run -p component-gallery

# 2. Minimal application shell counter (ApplicationState pattern)
cargo run -p nana-ui --example application-counter --features hosted,bundled-fonts

# 3. Declarative view counter (view! macro pattern)
cargo run -p nana-ui --example reactive-counter --features hosted,bundled-fonts,view-macro

# 4. Live GPU content mixed with ordinary controls in one window
cargo run -p nana-ui --example hosted-gpu-demo --features hosted,bundled-fonts,wgpu-interop,runtime-wgsl

# 5. Two-phase startup and the native Early Splash probe
cargo run -p nana-ui --example startup-splash --features hosted,bundled-fonts -- --probe
```

## Debugging, diagnostics, and boundary check tools

- **Framework diagnostics and performance metrics**: the frame path must never log or do file I/O. To investigate frame time, cache hit rates, or dropped frames, look at the static metrics of `nana-diagnostics` and the `.nlog` export (see [Diagnostics and Application Paths](/reference/diagnostics) and [Diagnostic Codes](/reference/errors)).
- **Static architecture gate scripts**: after changing code, run these read-only scripts at any time to check whether you crossed an architecture red line:
  ```bash
  python3 scripts/check-engine-boundary.py
  python3 scripts/check-api-convergence.py
  python3 scripts/check-package-boundary.py
  python3 scripts/audit-framework-strings.py
  ```
- **Pre-commit hygiene**: following `AGENTS.md`, run `cargo fmt --all -- --check` before committing code. After changing docs, run `cd docs && npm run build` and `git diff --check`.

## Three self-test questions

After reading this section, use these three questions as a quick self-check (the answers are all in the linked chapters):

1. Why should a button's click callback not call `mount_view_root` to re-render the whole page? Which crates does a click pass through, in order, from the operating system event to firing `on_activate`? (See [L3 Tree Building](/reference/l3-authoring) and [Input Path](/learn/runtime-input).)
2. Why can a piece of live GPU content (`GpuTextureView`) be clipped by its parent container's rounded corners, be covered by a dropdown menu behind it in document order, and still respond to mouse clicks? (See [Scene Extraction](/learn/scene-extraction) and [GPU and Frame Lifecycle](/learn/gpu-frame).)
3. Why does the dependency table of `nana-ui-runtime` contain no `nana-ui-css` at all, while both Rust `view!` and `.vue` components can use CSS styles? (See [CSS Engine](/learn/css-engine) and [Authoring Toolchain](/learn/toolchain).)

## Read next

- [End-to-End Data Flow](/learn/flows): four complete paths that tie all subsystems together
- [25-Crate Coverage Matrix](/learn/coverage-matrix): four-point quick reference for all 25 crates under `crates/`
- [Evidence Checklist](/learn/evidence-checklist): verify every architecture claim yourself with read-only commands
