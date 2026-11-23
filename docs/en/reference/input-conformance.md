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

## Windows real-machine acceptance (Issue #271 / #276)

IME and pen input in a real window are checked with the `input-acceptance-probe` example and `scripts/validate-input-acceptance.ps1`. The probe window holds one focused `TextInput` (accessible name `IME editor`) and an empty stroke area; every canonical input event the window's router delivers is written to stdout as one JSON line (`"event":"input"`), and changes to the editor's value and focus as `"event":"editor"`. The script operates only the probe window. The IME part types with `SendInput` through the system's real Microsoft Pinyin (0804, in Chinese mode) and Microsoft IME Japanese (0411), so preedit, candidates and commit come from the real IME, then checks the editor's `ControlType.Edit` and `ValuePattern` value through UI Automation. The pen part injects `PT_PEN` frames with `InjectSyntheticPointerInput`: the same `WM_POINTER` path a digitizer takes, but not a physical device. Leave the keyboard and mouse alone while it runs: injected keys go to the foreground window, and the script stops typing as soon as the probe loses the foreground.

```powershell
cargo build --release -p nana-ui --features hosted,bundled-fonts --example input-acceptance-probe
powershell -NoProfile -ExecutionPolicy Bypass -Mta -File scripts/validate-input-acceptance.ps1 -ExePath target/release/examples/input-acceptance-probe.exe
```

Output goes to `target/acceptance/input/`: `trace.jsonl` (the event trace), `summary.json` (each check and the environment) and screenshots at preedit, candidates and commit. When the system drops injected keys (under some endpoint policies `SendInput` reports success but the last-input time does not move), the IME part is recorded as not executed, never as passed.

Record of 2026-11-23 (Windows 11 Enterprise 25H2 build 26200.9974, RTX 5060 Ti / DX12, 96 DPI):

| Item | Result |
| --- | --- |
| Pen hover: moves in range are delivered with `buttons = 0` and `pressure = 0` | pass |
| One contact and one lift; pressure follows the device frame by frame from 1/1024 to 1.0 and back to 64/1024, no jump at contact, range 0–1 | pass |
| Tilt `(10, -5)` delivered unchanged | pass |
| A contact with the barrel button held maps to `button = 2` (secondary); the lift clears `buttons` | pass |
| Focus loss mid-stroke: `Focus { focused: false }` is delivered, the stroke ends, no button stays down | pass |
| Mouse after the pen: `pointer_type = Mouse`, `buttons = 0`, `pressure = 0`, no cross-talk | pass |
| Cancellation (`POINTER_FLAG_CANCELED`) | **fail**: delivered as an ordinary `Up`. winit 0.31's Win32 backend does not read the flag, so a cancelled stroke commits as if it had completed |
| Leaving the sensing range | **fail**: no `PointerLeave`. winit sends `PointerLeft` after every lift (while the pen still hovers), so the router gives each stroke a new pointer id; when the pen really leaves range, hover stays on the last element |
| A contact whose first frame has zero pressure | **fail**: the device reports 0, 40/1024, 80/1024 and 0.5, 0.039, 0.078 are delivered. winit reports pressure 0 as `None` and the host substitutes `0.5` while a button is down, so it jumps to 0.5 at contact |
| Microsoft Pinyin `nihao` + Space: preedit `ni'hao` delivered key by key, the candidate window sits under the caret without moving the layout, `你好` committed, focus unchanged | pass |
| Microsoft IME Japanese `konnnichiha` + Space + Enter: preedit and commit of `こんにちは`, focus unchanged | pass |
| UIA: the editor is `ControlType.Edit` and its `ValuePattern` value equals the committed value (both IMEs) | pass |
| Focus loss mid-composition: `Focus { focused: false }` and `Composition::End` are delivered; refocusing restores the editor focus | pass |
| The commit after focus loss mid-composition | pass: after the host sends `End`, Microsoft Pinyin still sends `Commit("ceshi")`; once blur has cancelled the preedit, the router drops that source's composition edits until focus returns, so the editor stays empty (`ime.focus_loss.no_late_commit`). This matches the IME paragraph of the window reference (`docs/en/reference/window.md`): "Runtime cancels the preedit when the native window loses focus" |
| Physical XP-Pen tablet | not executed; needs a person |

The three pen failures are in winit's Win32 pointer handling (it handles neither `WM_POINTERLEAVE` nor `POINTER_FLAG_CANCELED`, and does not tell zero pressure from no pressure); fixing them means changing winit or handling `WM_POINTER` in the host. Physical pen acceptance uses the same probe by hand: start the probe and keep its stdout as the trace, then with a real pen hover, draw a stroke with pressure, press the barrel button, switch windows mid-stroke and move out of range.
