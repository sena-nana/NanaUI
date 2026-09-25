# NanaUI Agent Guide

This repository is the NanaUI framework. Keep framework contracts here and keep application business state, persistence, navigation, and region content in consumers.

## Architecture

The product path is one retained tree:

\`\`\`text
Rust / Vue input
    -> nana-ui-core        shared style, theme, geometry, workspace, motion contracts
    -> nana-ui-runtime     UiWorld, components, layout, input, focus, IME, accessibility,
                           Shell, Workspace, Dock, overlays, and GPU-node declarations
    -> nana-ui-scene       extracted drawable scene, independent of WGPU
    -> nana-ui              host adapter and SceneWgpuPainter
\`\`\`

Supporting boundaries:

- \`nana-text\` is the product text measurement, shaping, and retained text-layout authority.
- \`nana-ui-platform\` owns platform-neutral window/input contracts; \`nana-window\` owns native handles, materials, title-bar chrome, scaling, and fullscreen behavior.
- \`nana-ui-vue\` and the JS host are input adapters into the same \`UiWorld\`; they do not create another tree or painter.
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

- Preserve one authoritative \`UiWorld\` → \`UiScene\` path. \`PendingHostOps\` commit at the frame boundary; \`LayoutBoxStore\` is a projection and scrolling does not write Runtime layout boxes.
- Use the shared \`ComponentRegistry\` for built-in and extension components. Keep \`event_flags\` in Runtime event listeners and GPU slots in Runtime \`CustomRenderNode\`.
- Keep logical state separate from presentation overlays. Compositor animation must not write transient samples back into base style state.
- Keep raw window handles out of ordinary controls. Native material failure returns the actual applied outcome or an explicit fallback.
- Keep one WGPU major version across manifests, lockfile, and the resolved dependency graph. Never add a second Device/Queue or a product CPU readback path.
- Public GPU extension contracts use \`nana-gpu\` types. \`wgpu::*\` may appear in a public signature of nana-gpu, nana-frame-exchange, nana-ui, nana-ui-vue or nana-ui-devtools only behind \`wgpu-interop\`; \`nana_gpu::__framework\` is for those crates' own sources, which therefore never turn \`wgpu-interop\` on for consumers. \`scripts/check-engine-boundary.py\` enforces both.
- Do not add a WebView product shell, a second text engine, a second UI tree, or product-specific Live2D/Cubism types.
- Framework diagnostics go through `nana-diagnostics` (`event!` / `metric!` / `fault!` with static descriptors in `nana_diagnostics::framework`, IDs append-only). No logging crate, `eprintln!`, formatting, or I/O on frame paths; high-frequency data is a metric, never a per-frame event. See `docs/diagnostics.md`.
- Visible actions must be wired to real state. Do not add placeholder routes, agent/tool instructions, technical copy, or unconnected controls to product UI.

## Change and validation hygiene

- Inspect existing implementation, consumers, docs, features, manifests, lockfiles, and tests when a public or cross-layer contract changes.
- Preserve unrelated worktree edits. Do not reset, overwrite, or create a temporary worktree.
- Prefer the smallest root-cause change. Add behavior tests only for changed behavior.
- Run \`cargo fmt --all -- --check\` and the focused checks selected by \`$nanaui-validation\`; use \`git diff --check\` for documentation or Skill-only changes.
- Report exact commands, results, and untested platform or consumer boundaries. A compile check alone is not GPU, visual, consumer, or native-window evidence.
