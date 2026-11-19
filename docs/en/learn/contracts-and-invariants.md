# Contracts and Invariants

NanaUI has a set of **non-negotiable architecture contracts**. They are not style suggestions: break any one of them and the framework grows a second authority, a second tree, or a second GPU device. This chapter lists the nine core contracts one by one, each with "what", "why" and "consequence of violation", plus verification readings you can rerun directly in the current repository.

The full contract reference for framework maintainers is [Architecture contracts](/reference/architecture). This chapter focuses on the design causes behind each contract, for readers learning the architecture.

## How the contracts are enforced

The framework holds these invariants with three lines of defense:

1. **Cargo dependency graph layering**: one-way dependencies mean lower crates cannot even see upper-layer types at compile time (see [Modules and layers](/learn/modules-and-layers)).
2. **Automated boundary scripts**: CI always runs `scripts/check-engine-boundary.py`, `scripts/check-api-convergence.py`, `scripts/check-package-boundary.py` and `scripts/audit-framework-strings.py`.
3. **Review criteria**: `AGENTS.md` at the repository root lists these nine contracts as the non-negotiable review baseline.

## Contract one: the product path flows one way, with one authoritative retained tree

```text
UiWorld → ExtractedNode → UiScene → SceneWgpuPainter
```

- **What**: the single authority for tree topology, computed style, unscrolled layout, hit testing, focus, IME and accessibility is `UiWorld` (`nana-ui-runtime`); the single authority for the draw graph is `UiScene` (`nana-ui-scene`); the only on-screen painter is `SceneWgpuPainter` (`nana-ui`). The layout output `LayoutResult` is only a projection and is not a second tree authority; scroll offsets are never written back to Runtime layout boxes.
- **Why**: once a "second tree" exists, layout, hit testing, animation, focus and accessibility must be synchronized both ways across two trees. That adds fixed overhead and can never fully line up under asynchronous input and transitions.
- **Consequence of violation**: the guarantee that changes commit atomically through `MutationQueue` at the frame boundary breaks; queried geometry boxes diverge from the real hit and paint areas.
- **How to verify**: the normal dependency graph from `cargo metadata` is strictly forward and acyclic (see [Modules and layers](/learn/modules-and-layers)); how the two authoring paths flow into the same tree is in [Authoring paths](/learn/authoring-paths).

## Contract two: CSS has one engine (`nana-ui-css`)

- **What**: stylesheet tokenizing, parsing, selector matching and cascade ordering are implemented only in `nana-ui-css`. The four core crates `nana-ui-core`, `nana-ui-runtime`, `nana-ui-scene` and `nana-ui` contain no stylesheet parsing code; they only consume the cascade output, the Style Model (`LayoutStyle`).
- **Why**: CSS cascade semantics (origin order, specificity triples, `@layer`, `!important`, `@container` range bucketing) are very fine-grained. Two implementations would inevitably compute different layouts for the same styles across `view!`, `.vue` and `tools/css-parity`.
- **Consequence of violation**: `view!` and `.vue` styles behave differently, and the `tools/css-parity` baseline is lost.
- **Readings from the current repository**:
  - Searching `nana-ui-runtime` for `specificity | Selector | cascade | stylesheet | CssParser | tokenizer`: every hit is a comment, a test event name, or `view/style.rs` consuming the build-time compiled `Sheet` / `StylePatch` (its module header states explicitly: "No CSS is parsed and no selector is matched at run time, and this crate has no CSS in it").
  - Searching `nana-ui-core`: only `SemanticColorRole::from_css_token_name` (`style_model.rs:341`, mapping `--nana-*` variable names to semantic roles) and single-value keyword parsing; no stylesheet parser.
  - Reading the dependency graph: the only direct consumer of `nana-ui-css` inside `crates/` is `nana-ui-view-codegen` (build time).

## Contract three: input routing has one consumer (`AppContext::route_input`)

- **What**: `nana-ui-input` owns the contracts for the canonical input event (`CanonicalInputEvent`), endpoint (`InputEndpoint`), sequencer (`InputSequencer`) and host services (`HostServices`). Each platform host lowers native events into canonical events, and **the router's only implementation lives in `nana-ui-runtime`**: the `route_input` of each `AppContext` (`crates/nana-ui-runtime/src/framework/input.rs:397`).
- **Why**: input event sequence numbers, endpoint generations, dropping of out-of-order and duplicate events, and key-versus-text suppression (`CommittedText`) must be decided in one place; otherwise the combined behavior of pointer capture, focus traversal and IME composition cannot stay deterministic.
- **Consequence of violation**: two routing paths reach different "press / release / activate" decisions for the same event sequence.
- **Readings from the current repository**:
  - `pub fn route_input` is defined only at `crates/nana-ui-runtime/src/framework/input.rs:397` in the whole repository (the headless test variant `HeadlessInput::route_shaped` is at `input/headless.rs:119` in the same module and still calls `context.route_input` internally).
  - `nana-ui` (`scene_host/host_services.rs`) and `nana-ui-devtools` (`agent/runtime.rs:120`) only lower or forward events to `AppContext::route_input`; there is no second router.

## Contract four: one GPU device and queue per process, and one wgpu

- **What**: WGPU is the only official graphics backend, and the dependency graph holds one wgpu: NanaUI's fork in `third_party/wgpu` (upstream package names and version 30.0.1 kept). A git dependency on this repository brings it along; an application that also uses a crate built on crates.io wgpu, such as egui-wgpu, patches wgpu and naga onto this repository at the same rev with `[patch.crates-io]` in its own workspace (see `third_party/wgpu/README.md`). Creating a second `Device` / `Queue` is forbidden, and so is CPU readback on the product presentation path.
- **Why**: two devices mean two independent submission timelines and fences, so live GPU content and UI controls cannot line up exactly in the same frame and the same `present`; CPU readback breaks the asynchronous GPU pipeline outright.
- **Consequence of violation**: tearing, one-frame lag, or cross-device handle crashes during window resizing or high-refresh animation.
- **How to verify**: `python3 scripts/check-engine-boundary.py` enforces public signatures: `wgpu::*` may appear only with the `wgpu-interop` feature enabled and only in the four allowlisted crates (`nana-gpu`, `nana-frame-exchange`, `nana-ui`, `nana-ui-devtools`); framework-internal sources reach it through the private module `nana_gpu::__framework`; the same script requires every wgpu and naga crate in the graph to come from `third_party/wgpu`.

## Contract five: raw window handles never reach ordinary controls

- **What**: platform raw window handles such as `HWND` and `NSWindow` are sealed strictly inside the `nana-window` and `nana-ui-platform` boundary; ordinary controls cannot get a handle at compile time or at run time. When a system material request fails, the code must truthfully return **the actually applied outcome (`MaterialOutcome`)** or an explicit fallback reason, never silently fake success or quietly try another material family.
- **Why**: once an ordinary control holds a native handle and calls system APIs directly, it bypasses the framework's layout, clipping, hit testing and multi-window lifecycle.
- **Consequence of violation**: native content drawn privately by a control cannot be clipped by rounded corners or covered by overlays, and cannot be verified in headless tests.

## Contract six: logical state is separate from presentation overlays

- **What**: `UiWorld` stores only the logical target state; transient presentation values during a transition live in the `PresentationStore` overlay. Compositor animations (`AnimationClass::Compositor`) must never write per-frame samples back to a node's base style.
- **Why**: if the intermediate values `0.01`, `0.02` of `opacity: 0 → 1` were written back to `UiWorld` every frame, application code would read the wrong transitional state, and the retained tree would be marked dirty and re-extracted every frame.
- **Consequence of violation**: `UiWorld` goes dirty every frame during a pure compositor animation, and the `compositor-steady` zero-work gate in `perf/README.md` fails.

## Contract seven: diagnostics go through `nana-diagnostics`, with zero I/O on frame paths

- **What**: framework diagnostics use the `event!`, `metric!`, `fault!` and `span!` macros with static descriptors (`nana_diagnostics::framework`, IDs append-only). External logging crates, `eprintln!`, string formatting and file I/O are forbidden on frame paths; high-frequency data must be recorded as an atomic `metric!`, never sent as a per-frame `event!`.
- **Why**: at 144 Hz / 240 Hz, one `eprintln!` or formatting allocation per frame blows straight through the frame time budget.
- **Consequence of violation**: periodic spikes in frame time; changing diagnostic IDs arbitrarily breaks existing offline `.nlog` parsers.

## Contract eight: one shared `ComponentRegistry`, no second instantiation ABI

- **What**: all built-in and extension controls share the same `ComponentRegistry` and the same `register_component` entry point. `ComponentSupport` is not an instantiation ABI.
- **Why**: two component ABIs mean implementing each control's layout, focus and accessibility projection twice, and their capabilities would never line up.
- **Consequence of violation**: extension controls do not get the incremental layout, focus navigation and accessibility projection support that built-in controls have.

## Contract nine: WebView is not a product UI shell

- **What**: NanaUI has no whole-window WebView shell. Embedding a web page in a window is limited to the explicit host-native exception `runtime::BrowserView` (currently only a macOS `WKWebView` backend); headless `WebSurface` pages are **host content producers** (sending captured frames to an application sink) and are not part of the UI tree. The box-model comparison tool `tools/css-parity-webview` stays outside the workspace and is never linked into product crates.
- **Why**: using a whole-window WebView as the UI shell takes the entire interface out of the native retained tree and the host GPU pipeline, losing the core value of compositing live GPU content in document order within the same window.
- **Consequence of violation**: native WebView content cannot be captured in offscreen screenshots, and when it meets non-rectangular clipping, opacity/filter groups, or controls layered above it, it can only be hidden entirely.

## The one-sentence review criterion

Contracts one through nine all point to the same architecture review question:

> **"Who is the single authority for this?"** For the retained tree, CSS cascade, text layout, input routing, GPU device and native windows, each domain has exactly one crate in the whole repository with the final say; other crates may only submit intent to it or consume its projections, and must never privately maintain a second answer.

## Read next

- [Modules and layers](/learn/modules-and-layers): a full reading of the dependency graph of 25 crates
- [Authoring paths](/learn/authoring-paths): how the two authoring paths flow into the same tree
- [Evidence checklist](/learn/evidence-checklist): automated recheck commands and source line numbers for the nine contracts
