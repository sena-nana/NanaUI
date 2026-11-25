# iOS Experimental Host

The iOS host (`platform/ios`) is an **experimental** host path. Like [Android](android.md), it is not a product delivery target. It shares the [mobile host](mobile-host.md) layer with Android.

## Structure

| Path | Role |
| --- | --- |
| `platform/ios/src/lib.rs` | `nana-ios-host`: an rlib that provides the host's C ABI (lifecycle, touch, frames, text proxy, accessibility), wrapping `MobileRuntime` and `MobileRenderer` |
| `platform/ios/include/nana_ios_host.h` | The shared C header. Every call is on the main thread; geometry is in physical pixels by default, and names containing logical are in logical pixels; text offsets are UTF-8 bytes |
| `platform/ios/Shell/NanaView.swift` | A view backed by `CAMetalLayer`: `CADisplayLink` pauses and resumes according to `FrameDemand` (up to 120 Hz on ProMotion); touches carry coalesced samples; light/dark and Dynamic Type are read from the trait collection; it detaches when entering the background; haptic feedback; VoiceOver container. `afterTick`, `onTick`, and `onIdle` are left to the app for laying out native layers and collecting frame statistics |
| `platform/ios/Shell/NanaTextProxy.swift` | Hybrid layer ①: a `UITextInput` text proxy, plus `UITextInteraction` |
| `platform/ios/probe/` | `nana-ios-probe`: the staticlib for the measurement page. It puts the measurement page into the host with `export_ios_app!` and carries its own `nana_probe_*` calls (`include/nana_ios_probe.h`) |
| `platform/ios/App/NanaProbe.xcodeproj` | The Swift shell for the measurement page. The "Build Rust host" build phase calls `build-rust.sh`, which builds the staticlib per platform and configuration (Debug uses `dev`, Release uses `dist`) |
| `App/NanaProbe/ProbeViewController.swift`, `ProbeCamera.swift` | Hybrid layer ③: a 640×480 front-camera `AVCaptureVideoPreviewLayer` sits under the transparent surface, laid out at the position of the measurement page's preview area; it can also switch to the CPU upload path |
| `App/NanaProbe/ProbeMetrics.swift` | P50/P99 of frame interval and tick CPU time (logged every 240 frames), cold start to first frame, and signposts for Instruments |

### Building an app on the host

The host is an rlib; the Xcode target links only the app's own single staticlib:

1. Set the app crate to `crate-type = ["staticlib", "rlib"]` and depend on `nana-ios-host` and `nana-mobile-host`.
2. Implement `IosApp`:
   - `mount` mounts the app's view; the `Shell` passed in is used to request haptics (`Haptic`) and wake-ups;
   - `before_frame` runs before each frame is prepared, for example to update host textures.
3. Export `nana_ios_host_new` with `export_ios_app!(MyApp::new())`. The Swift shell's calls to `nana_ios_host_*` do not change.
4. The app's own C calls use `with_app::<MyApp, _>(host, |parts| ...)` to get the app, `MobileRuntime`, and `MobileRenderer`, and are declared in the app's own header (which includes `nana_ios_host.h`).
5. Xcode project:
   - reference the two Swift files in `Shell/`;
   - in the "Build Rust host" phase set `NANA_RUST_PACKAGE` and `NANA_RUST_LIB`, and also `NANA_CARGO_MANIFEST` when the crate is not in this repository;
   - link `-l<library name>` in `OTHER_LDFLAGS`.

### Accessibility

The host flattens the document's accessibility tree into the nodes VoiceOver stops on: nodes with a matching role, and nodes with a label.

- The shell calls `nana_ios_host_accessibility_count` to take a snapshot, then reads each node (role, state bits, logical-pixel frame, slider value) and its text (label, value, hint).
- Actions go through `nana_ios_host_accessibility_action`: activate, increment/decrement, and focus.
- `NanaView` turns these nodes into `UIAccessibilityElement`s; while VoiceOver is on, it posts `layoutChanged` whenever the node count changes.

## Text proxy

When an editor gains focus, the host notifies the shell through a callback. The shell places the proxy view over the editor, makes the proxy the first responder, and attaches `UITextInteraction(for: .editable)`. From then on, taps, long presses, and drags on the editor go to UIKit's text gestures.

| UIKit call | Host call |
| --- | --- |
| `setMarkedText` / `unmarkText` / `insertText` / `deleteBackward` | `set_marked_text` / `commit_text` / `delete_surrounding` |
| Setting `selectedTextRange` | `set_selection` |
| `caretRect(for:)` / `firstRect(for:)` / `selectionRects(for:)` | `text_caret_rect` |
| `closestPosition(to:)` | `text_offset_at` |

UIKit counts in UTF-16 offsets; the proxy converts to and from UTF-8 byte offsets. Callbacks run inside the host's input routing and cannot call back into the host synchronously, so the shell always dispatches them to the main queue first.

Known behavior and limitations:

- `NanaTextProxy` fixes its `tintColor` to `.clear` so UIKit's native `UITextInteraction` caret stays transparent while NanaUI paints the single editor caret, preventing duplicate carets while `caretRect(for:)` and `selectionRects(for:)` continue reporting the real editor rects for the loupe, edit menu, and IME candidates.
- `selectionRects` is computed for a single line only.

## Building

`nana-ios-host` and `nana-ios-probe` are in the root workspace but have content only on `target_vendor = "apple"` (the condition under which wgpu provides `SurfaceTargetUnsafe::CoreAnimationLayer`); on Linux and Windows they compile as empty crates, and `cargo check --workspace` passes as usual. Check the iOS target itself with the scripts below.

```bash
# Cross-check (device target; --sim switches to the simulator target, which needs rustup target add aarch64-apple-ios-sim first)
./scripts/check-ios-arm64.sh

# Build once without signing to confirm the project compiles and links
xcodebuild -project platform/ios/App/NanaProbe.xcodeproj -target NanaProbe \
  -configuration Release -sdk iphoneos CODE_SIGNING_ALLOWED=NO build
```

To install on a device, select a Development Team for the NanaProbe target in Xcode, then run with the Release configuration. The Debug staticlib carries full DWARF and is hundreds of MB; do not use it to measure size or performance.

The shell uses the UIScene lifecycle (`SceneDelegate` creates the window; backgrounding is observed through `UIScene` notifications): an app built with the iOS 27 SDK that does not adopt the scene lifecycle fails at launch.

### Simulator

```bash
# Install the rustup target on the toolchain used for the build (NanaUI uses stable)
rustup +stable target add aarch64-apple-ios-sim
xcodebuild -project platform/ios/App/NanaProbe.xcodeproj -target NanaProbe \
  -configuration Debug -sdk iphonesimulator -arch arm64 CODE_SIGN_IDENTITY=- build
xcrun simctl install booted build/Debug-iphonesimulator/NanaProbe.app
# --console-pty connects the host's stderr (tick failures, panics) to the terminal
xcrun simctl launch --console-pty booted dev.nana.probe
```

The simulator can verify functionality: first frame, touch scrolling and flinging, switches and sliders, Pinyin composition and candidate commit, backspace, going to the background and returning, light/dark switching, and no frames while idle. It cannot verify:

- **Camera**: the simulator has no camera; the preview layer stays black, and the CPU preview receives no frames.
- **Performance**: frame interval and CPU numbers do not count as evidence.
- **GPU capabilities**: the simulator's Metal offers only the Apple2 family, so wgpu provides only 15 inter-stage variables (31 on devices from A11 onward). Scene shaders are written to the limit of 15, so they also run on the simulator; keep to this limit when adding shader outputs.

## Acceptance

All of the following must be compared back to back on a real device against a minimal SwiftUI control app (the same grid, plus `AVCaptureVideoPreviewLayer`). Simulator numbers do not count as evidence.

| Item | Target | How to measure |
| --- | --- | --- |
| Cold start to first frame | ≤ 500 ms | `first frame` log (counted from process start) |
| Frame interval while flinging the grid | P99 ≤ 8.3 ms at 120 Hz | `frames=… interval p99` log, plus Instruments Animation Hitches |
| Idle | CPU about 0 over 10 seconds, no ticks | Xcode CPU gauge; no new frame statistics in the log |
| First-effect hitches | No frame over 16 ms when first entering a page or when each effect first appears | `tick` durations in signposts |
| Preview power | Native preview layer, NanaUI underlay preview, and CPU upload, each run for 10 minutes to compare power and temperature | Xcode Energy gauge, Instruments Power Profiler |
| Size | Record the IPA increase | Release archive |
| Feel | Chinese IME composition, long-press selection, the magnifier, and the edit menu all work; scrolling and bounce compared side by side with the SwiftUI version | Manual operation |

Touch-drag scrolling of the grid uses Runtime touch scrolling, and the iOS host selects `ScrollPhysics::Ios`: deceleration by 0.998 per millisecond, rubber banding when dragged past an edge, and a bounce back on release. The contract is in [runtime-scene.md](runtime-scene.md#touch-scrolling-and-scrollphysics). What must be compared side by side with the SwiftUI version for feel: how far a fling travels before stopping, the strength of the bounce, and the 8-logical-pixel slop from press to scroll start.
