---
title: "Evidence checklist: how to verify the key architecture claims"
---

# Evidence checklist: how to verify the key architecture claims

This page turns the **core architecture claims** that recur throughout [Learn](/learn/) into a three-column table: claim → read-only verification command → actual reading in the current workspace. Every command can be re-run directly from the repository root and modifies no file.

## A. Layering and dependency direction

### A1. Dependencies flow one way along `core → runtime → scene → nana-ui` and are strictly acyclic

**Why it matters**: once a reverse dependency edge appears, the lower-layer contract starts to see implementation details of the upper tree or the renderer, and one-way layering collapses (see [Contracts and invariants](/learn/contracts-and-invariants)).

```powershell
# Parse the workspace's normal dependency edges (a successful cargo metadata parse is itself proof of acyclicity: Cargo rejects cyclic dependency graphs)
cargo metadata --format-version 1 --no-deps | ConvertFrom-Json
# How to read it: walk dependencies[] of every member package; an entry whose path falls inside crates/ and whose kind is empty (normal) is one normal dependency edge
```

**Current reading**: `crates/` holds **58 workspace-member normal dependency edges** (57 among the 24 member crates + 1 optional edge `nana-ui → nana-icons-tabler`, plus standalone `nana-icons-tabler → nana-ui-core`), and **all of them strictly forward**. The core mainline: `nana-ui-core` depends only on `nana-diagnostics`; `nana-ui-runtime` depends on `{core, text, input, view-schema, diagnostics}` plus optional `{charts, diagram, view-macros}`; `nana-ui-scene` depends on `{runtime, core, diagnostics, text (optional benchmark)}` with `nana-ui-charts` removed; `nana-ui` depends on `{scene, runtime, core, …}`. Not one normal edge points from a lower layer back to an upper one. The four `dev-dependencies` exceptions used only for test hosts are listed separately in [Modules and layers](/learn/modules-and-layers#dev-edges-and-exceptions) and never enter the product binary.

### A2. None of the four main UI crates depends on the CSS engine

```powershell
rg "nana-ui-css" crates/nana-ui-core/Cargo.toml crates/nana-ui-runtime/Cargo.toml crates/nana-ui-scene/Cargo.toml crates/nana-ui/Cargo.toml
```

**Current reading**: `0` hits. The only consumer of `nana-ui-css` inside `crates/` is `nana-ui-view-codegen` (build time); see B1.

## B. Inventory of single authoritative implementations

### B1. CSS parsing and the cascade live only in `nana-ui-css`

```powershell
cargo tree -p nana-ui-css --depth 1                          # css -> core + serde_json, no retained-tree dependency
rg -i "selector|cascade" crates/nana-ui-runtime/src -g '!tests'  # every hit is a comment/doc/ICU selectordinal
rg "nana_ui_css" crates/*/src                                # only view-codegen (build time)
```

**Current reading**: the module docs at `crates/nana-ui-runtime/src/view/style.rs:8-9` state `"No CSS is parsed and no selector is matched at run time, and this crate has no CSS in it"`; `nana-ui-core` has no stylesheet parser, selector matching or cascade implementation (it only provides single-value enum keyword parsing, the `calc()` AST and the `--nana-*` semantic color token name mapping in `box_layout.rs` / `geometry.rs` / `style_model.rs:341`); `nana-ui-scene` has a single benchmark comment; `nana-ui` has `0` hits.

### B2. The only consumer of input routing is `AppContext::route_input` in Runtime

```powershell
cargo tree --workspace -i nana-ui-input --edges normal       # normal-edge consumers: runtime + platform (re-export)
rg "pub fn route_input" crates                               # the only definition: crates/nana-ui-runtime/src/framework/input.rs:397
rg "route_input|route_shaped" crates -g '!*tests*'           # every host funnels into context.route_input
```

**Current reading**: the only implementation of `route_input` is at `crates/nana-ui-runtime/src/framework/input.rs:397` (`impl AppContext`); `nana-ui-devtools` calls `route_shaped` (`crates/nana-ui-devtools/src/agent/runtime.rs:123`) → internally at `crates/nana-ui-runtime/src/framework/input/headless.rs:126` it still calls `context.route_input`, and there is no bypass route anywhere.

### B3. One text engine, one GPU backend

```powershell
rg "cosmic_text|cryoglyph" crates/nana-text/src              # only the doc statement itself at lib.rs:25
rg "nana_gpu::__framework" crates                            # __framework is used only inside the framework crates allowed to
python3 scripts/check-engine-boundary.py                     # automated gate script
```

**Current reading**: `crates/nana-text/src/lib.rs:25` has a single module comment stating the prohibition itself (`"No cosmic_text or cryoglyph identifier appears under src/"`); on the GPU side WGPU is the only backend, and `wgpu::*` in public signatures is allowed only under the `wgpu-interop` feature inside the four whitelisted crates, enforced and passing via `scripts/check-engine-boundary.py`.

## C. Anchor points of the key frame-path invariants

| Architecture claim | Source anchor | What to check |
| --- | --- | --- |
| Changes commit atomically at the frame boundary | `crates/nana-ui-scene/src/runtime_document.rs:109` (`flush`) | Bounded 8-pass convergence; layout, extraction, hit testing and accessibility share one `layout_generation` (`:28`) |
| Scrolling does not rewrite Runtime layout boxes | `crates/nana-ui-runtime/src/world/mutation.rs:2758` (`scroll_to_clamped`) | Only `scroll_offset` and the hit-test scroll offset change; the `LayoutBox` stays the same |
| One `Device` / `Queue`, cross-generation resources rejected | `crates/nana-gpu/src/context.rs:19` (`DeviceGeneration`), `:715` (`check_device`) | Device loss records the first cause stickily; a texture or target from an old generation returns `DeviceMismatch` at the entry point |
| No CPU readback on the product present path | `crates/nana-ui-devtools/src/offscreen.rs` (agent session) | CPU readback exists only in the devtools offscreen snapshot; the `present` path of `run_runtime` never goes through it |
| Zero string formatting and zero file I/O on the frame path | `crates/nana-diagnostics/src/framework.rs:2` | `event!` / `metric!` are both based on static descriptors, and IDs are append-only |

## D. Anchor point of the 35-step total order of one frame

The full 35-step order of one frame, from scheduling and draining input messages to `present`, is in [GPU and frame lifecycle](/learn/gpu-frame); the four cross-layer end-to-end flows are in [End-to-end flows](/learn/flows). The source entry anchor is:

```powershell
rg "fn redraw" crates/nana-ui/src/scene_host/present.rs      # pub(super) fn redraw (starting at line :114)
```

## Read next

- [Contracts and invariants](/learn/contracts-and-invariants): why the nine core contracts are designed this way and what violating them costs
- [Modules and layers](/learn/modules-and-layers): the full list of 58 workspace-member normal dependency edges and the dev edges explained
- [25-crate coverage matrix](/learn/coverage-matrix): the four-part quick-reference table for each crate
