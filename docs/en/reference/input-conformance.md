# Input Conformance

This page lists the automated regression commands for the input contract (Issue #243) that can be rerun inside the repository, and the boundary acceptance that must be done on real platform windows or physical devices. Offscreen and headless tests do not replace real-device evidence for native windows, a real system input method (IME), WebView, XR, or remote network environments.

## In-repo automated contract tests

The window host and the Android control slot both lower platform-native events to `CanonicalInputEvent` at the boundary and send them through `AppContext::route_input` (windowless test environments use `HeadlessInput`). Diagnostic counters are registered in `nana-diagnostics` under `runtime.input.*` (IDs 20–37; retired IDs are never reused).

| Acceptance contract | Verification command |
| --- | --- |
| Bounded queue capacity, coalescing of adjacent `PointerMove` / `Wheel` events, endpoint `generation` advance and recount | `cargo test -p nana-ui-input --lib --locked` |
| Source binding, disconnect cleanup, focus loss, pointer capture, clipboard, file drag and drop (`InputPayload::FileDrag`), an empty endpoint drains to 0; key policy observes the physical key, logical key, repeat, and unified modifiers together | `cargo test -p nana-ui-runtime --all-features --lib framework::input --locked` |
| `Keymap` matches on the logical key (the four modifier bits match `InputModifiers` exactly; key release does not trigger by mistake) | `cargo test -p nana-ui-runtime --all-features --lib key_layers --locked` |
| An uncaptured pointer move does exactly 1 hit query, 0 queries while captured, and zero heap allocations in steady-state moves; over 1,000 consecutive `PointerMove` events at the same position or across rows, the layout frontier and scene extract are both empty | `cargo test -p nana-ui-runtime --all-features --test perf_contracts input_alloc --locked` |
| Single-document hit testing follows the presentation layer's translation, scale, 3D projection, and clip chain | `cargo test -p nana-ui-runtime --all-features --lib world::hit_test --locked` |
| When the host polls at 240 Hz and a static window stays at `FrameDemand::OnDemand`, `present` runs 0 times over 240 polls | `cargo test -p nana-ui --features hosted --lib scene_host::schedule::tests::a_static_window_polled_at_240hz_presents_nothing --locked` |

Note: `1000 Hz` in the tests is the rate of the synthetic `PointerMove` event stream, and `240 Hz` is the host event loop's polling rate (the tests verify work counts, not a physical display refresh rate). In a window, coalescible pointer-move and wheel events are held until the end of the current event round and routed together; a redraw frame is requested only when that routing actually marked document work dirty. For wall-clock benchmarks use `nana-input-benchmark` (`crates/nana-ui-devtools/src/bin/nana-input-benchmark.rs`).

## Cross-host and external environment boundaries

| Host adapter | In-repo support and external boundary |
| --- | --- |
| **Native desktop window (Window)** | `scene_host` lowers winit events to `CanonicalInputEvent` and routes them end to end |
| **Nested NanaUI (Nested)** | `EmbeddedSurfaceNode` currently carries only size and generation metadata; the nested input transform arrives with #247 and does not add a second coordinate transform |
| **Qt / game engine embedding** | Windowless paths all use `HeadlessInput`; the repository contains no Qt- or engine-specific glue layer |
| **XR spatial input** | No contract for raycasting, trigger, or thumbstick lifecycles |
| **Web / WebView** | No browser DOM composition or dynamic DPR host bridge |
| **Remote streaming input (Remote)** | `HeadlessInput` covers sequence numbers, generations, and disconnect semantics; network-level authentication, replay protection, rate limiting, and reordering belong to the external transport layer |
