# NanaUI Agent Guide

This repository is the NanaUI framework. Keep framework contracts here and keep application business state, persistence, navigation, and region content in consumers. Facades forward optional ICU locale formatting explicitly; internal runtime/scene dependencies disable defaults so consumers can opt out without a transitive feature re-enabling it.

## Architecture

The product path is one retained tree:

\`\`\`text
Rust input (builders, view!, .vue dialect)
    -> nana-ui-core        shared style, theme, geometry, workspace, motion contracts
    -> nana-ui-runtime     UiWorld, components, layout, input, focus, IME, accessibility,
                           Shell, Workspace, Dock, overlays, and GPU-node declarations
    -> nana-ui-scene       extracted drawable scene, independent of WGPU
    -> nana-ui              host adapter and SceneWgpuPainter
\`\`\`

Supporting boundaries:

- \`nana-text\` is the product text measurement, shaping, and retained text-layout authority. It also owns where each laid-out glyph goes (\`TextLayout::glyph_placements\`) and, behind its \`raster\` feature, CPU glyph rasterization (\`nana_text::raster\`) that the renderer's atlas and hosts compositing text themselves share.
- \`nana-ui-charts\` is the chart model: the typed ECharts-shaped option, scales, layout, hit data and the GPU mark arrays the chart shaders read. It depends only on \`nana-ui-core\`; the \`Chart\` component lives in \`nana-ui-runtime\` and the chart pipelines in \`nana-ui\`.
- \`nana-ui-diagram\` is the Mermaid engine: parsing, layout and a theme-neutral vector display list, with no GPU and no text engine (callers pass measurement). \`nana-ui-runtime\` (\`rich-text\`) lays fences out through it and records the list against the theme; \`nana-ui-scene\` tessellates the recording like any custom paint. Diagram types land one family at a time behind \`families::<name>::NATIVE\`; see \`docs/reference/diagram-engine.md\`.
- \`nana-ui-css\` is the one CSS engine: stylesheet parse, cascade and the mapping onto the Style Model. The \`.vue\` compiler, \`view!\` and \`css!\` run it at build time and hand the runtime Style Model data. CSS parsing stays out of \`nana-ui-core\`, \`nana-ui-runtime\`, \`nana-ui-scene\` and \`nana-ui\`.
- \`nana-ui-input\` owns the canonical input event, endpoint, sequencer, and \`HostServices\` contract; hosts lower native input into it and the per-\`AppContext\` router in \`nana-ui-runtime\` is its only consumer.
- \`nana-ui-platform\` owns platform-neutral window contracts and re-exports the input contract; \`nana-window\` owns native handles, materials, title-bar chrome, scaling, and fullscreen behavior.
- \`nana-gpu\` is the GPU backend contract: \`GpuContext\` (device, generation, capabilities, loss, textures, submission guard), \`FrameContext\` (one frame's encoder, submit/discard), and \`GpuTexture\`. WGPU is the only backend; \`wgpu-interop\` is the explicit escape hatch.
- \`nana-frame-exchange\` carries producer frames. It does not own the UI device or submission.

The host owns Window, Surface, the \`GpuContext\`, each \`FrameContext\`, and frame scheduling. \`SceneWgpuPainter\` is built on that context and paints into the host's frames. GPU content is represented by \`CustomRenderNode\` or ordinary \`HostTexture\` nodes and remains in layout, clipping, hit testing, and document order.

## Skill groups

Use the smallest matching skill and route across groups when a change crosses a boundary:

- \`$nanaui-runtime-scene\`: core models, retained tree, components, layout, text, motion, input, extraction, scene structure, and public runtime contracts.
- \`$nanaui-workspace-ui\`: Workspace, Dock, Shell, regions, settings, themes, widgets, overlays, title-bar slots, serialization, and public UI exports.
- \`$nanaui-gpu-integration\`: host GPU context, painter, render passes, HostTexture/GpuView, frame lifecycle, redraw, and WGPU dependency convergence.
- \`$nanaui-window-materials\`: native window handles, materials, chrome, scaling, resize, input bridges, fullscreen, and platform fallbacks.
- \`$nanaui-validation\`: proportionate Runtime, Scene, visual, GPU, window, compatibility, and performance evidence.

Read the focused contract document named by the selected skill before editing. Route application-specific behavior back to the consumer instead of adding it to NanaUI.

## Non-negotiable contracts

- Shared path spelling belongs to `nana-ui-platform::path` (facade `nana_ui::path`): normalize usable paths conservatively, canonicalize only on I/O workers, and format read-only display text separately. Read-only summaries use Runtime `PathText` (also `LabeledValue::path_value` / `SettingsRow::path_hint`) for width-aware middle elision and full-value tooltips. Never persist, copy or open display strings containing wrapping characters; native dialogs and editable path fields retain real paths. See `docs/reference/window.md`.

- Preserve one authoritative \`UiWorld\` → \`UiScene\` path. \`MutationQueue\` batches commit atomically at the frame boundary; layout output is a projection and scrolling does not write Runtime layout boxes.
- Use the shared \`ComponentRegistry\` for built-in and extension components. Keep \`event_flags\` in Runtime event listeners and GPU slots in Runtime \`CustomRenderNode\`.
- Keep logical state separate from presentation overlays. Compositor animation must not write transient samples back into base style state.
- Fade a subtree or compound painter as one isolated composition, including the frames before compositor-layer promotion; keep a modal scrim outside its own card's opacity group, and a triggered menu's trigger outside its hanging surface's fade and scale.
- Keep raw window handles out of ordinary controls. Native material failure returns the actual applied outcome or an explicit fallback.
- Auxiliary top-level windows use `WindowDescriptor::parent` for native ordering above their owner and host-managed level inheritance. Keep this separate from Underlay's below-owner contract; hidden startup owners and Wayland remapping follow `docs/reference/window.md`.
- Keep one WGPU major version across manifests, lockfile, and the resolved dependency graph. Never add a second Device/Queue or a product CPU readback path.
- Public GPU extension contracts use \`nana-gpu\` types. \`wgpu::*\` may appear in a public signature of nana-gpu, nana-frame-exchange, nana-ui or nana-ui-devtools only behind \`wgpu-interop\`; \`nana_gpu::__framework\` is for those crates' own sources, which therefore never turn \`wgpu-interop\` on for consumers. \`scripts/check-engine-boundary.py\` enforces both.
- Frame exchange may carry typed application payloads with completed frames and leases; payload changes do not change token identity or epoch, and framework crates do not interpret application coordinates.
- Do not add a WebView product shell, a second text engine, a second UI tree, or product-specific Live2D/Cubism types. Headless `WebSurface` pages are host content producers (frames to an application sink), not UI.
- Framework diagnostics go through `nana-diagnostics` (`event!` / `metric!` / `fault!` with static descriptors in `nana_diagnostics::framework`, IDs append-only). No logging crate, `eprintln!`, formatting, or I/O on frame paths; high-frequency data is a metric, never a per-frame event. See `docs/reference/diagnostics.md`.
- Visible actions must be wired to real state. Do not add placeholder routes, agent/tool instructions, technical copy, or unconnected controls to product UI.

## Change and validation hygiene

- Inspect existing implementation, consumers, docs, features, manifests, lockfiles, and tests when a public or cross-layer contract changes.
- Preserve unrelated worktree edits. Do not reset, overwrite, or create a temporary worktree.
- Prefer the smallest root-cause change. Add behavior tests only for changed behavior.
- Run \`cargo fmt --all -- --check\` and the focused checks selected by \`$nanaui-validation\`; use \`git diff --check\` for documentation or Skill-only changes.
- Report exact commands, results, and untested platform or consumer boundaries. A compile check alone is not GPU, visual, consumer, or native-window evidence.
