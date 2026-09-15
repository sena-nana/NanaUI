# NanaUI Android ARM64 host (experimental, phase 2)

This crate is an **experimental** GameActivity host. **Android is not a
current NanaUI product target** and is **not a second product paint kernel**.
The control slot is a NanaUI Runtime / UiScene path painted by
`SceneWgpuPainter`. It is not DesktopShell.

Phase 2 wires GameTextInput `InputConnection`, JNI `ClipboardManager`, and
TalkBack Click/Focus/SetValue/SetSelection for Button / Switch / TextInput.
Scroll / virtual lists stay later. Cross-compile and `--lib` tests are not
device CJK / TalkBack evidence.

Rust owns V8 (desktop smoke, device via `RUSTY_V8_ARCHIVE`) + Vue custom
renderer (`VueHost`) + wgpu Vulkan Surface + `AndroidShellStub`. There is no
System WebView.

## Layout

| Path | Role |
|------|------|
| `src/lib.rs` | `android_main` entry (`cdylib`) |
| `src/shell.rs` | `AndroidShellStub` — `WorkspaceLayout` / `WorkspaceGeometry` |
| `src/runtime.rs` | GameActivity lifecycle + GameTextInput + Scene present |
| `src/gpu.rs` | Host-owned wgpu 30 Surface (Vulkan) |
| `src/slot_runtime.rs` | RuntimeDocument + pointer/key/IME into Runtime |
| `src/slot_ime.rs` | GameTextInput buffer → `ImeEvent` (host-testable) |
| `src/engine.rs` | V8 + VueHost smoke boot (desktop); Android cross-build skips V8 without archive |
| `app/` | Gradle GameActivity wrapper (`NanaActivity`) |

## Build

```bash
# From repo root:
./scripts/setup-android-ndk.sh          # once
source scripts/android-env.sh           # bash
./scripts/check-android-arm64.sh        # cargo check
./scripts/check-android-arm64.sh --build
```

Artifact: `target-android/aarch64-linux-android/debug/libnana_android_host.so`

Host-side compile/smoke (no NDK UI):

```bash
cargo check -p nana-android-host --locked
cargo test -p nana-android-host --lib --locked
```

## Features

- `engine-v8` (default on host) — desktop smoke. Android ARM64 cross-check links
  V8 when `RUSTY_V8_ARCHIVE` is set (GitHub Actions `Package V8`); otherwise
  `--no-default-features` (`docs/android.md`). The `android-arm64-cross` CI job
  is the V8 **stub** path and does not build GN.
- **`AndroidShellStub`** sizes Primary viewport from the same `nana-ui-core` geometry as desktop
  `DesktopShell`. `VueHost` resolves layout in that viewport. Frame presentation is wgpu chrome
  fill plus a NanaUI Runtime control-slot strip; this is not DesktopShell.

## Packaging

GameActivity needs Java (`androidx.games:games-activity:4.4.0`, no prefab).
The script copies the stripped `.so` into `app/src/main/jniLibs` and assembles
with Gradle when `gradle` is on `PATH`:

```bash
source scripts/android-env.sh
./scripts/check-android-arm64.sh --build --dist
./scripts/package-android-host-apk.sh
# → $CARGO_TARGET_DIR/apk/nana-android-host-debug.apk
```

`cargo-apk` 0.10 cannot parse this repo’s root `Cargo.toml`. Do not enable
GameActivity prefab C++ glue; `android-activity` already links its own.

Requires SDK `build-tools`, `platforms;android-34`, and Gradle. Metadata under
`[package.metadata.android]` remains documentation.

## NanaUI control slot (experimental host test)

- Geometry: `control_slot` / `chrome_present_bands`
- Widget strip: Nana Runtime `Button` / `Text` / `TextInput` / `Switch`.
  `desktop_shell_available()` stays `false`.
- Pointer: GameActivity `MotionEvent` → `RuntimeInputAdapter`.
- Keyboard / IME: GameTextInput `TextEvent` diffs into
  `ImeEvent::{Preedit,Commit,DeleteSurrounding}` via `dispatch_ime`.
  Hardware KeyEvents still cover editing keys. Soft keyboard show/hide follows
  `SlotRuntime::text_input_focused`. `PlatformCapabilities::ime` is true.
- Accessibility: `slot_ax.rs` / `accesskit_android::InjectingAdapter` publishes
  the control-slot tree; TalkBack Click/Focus/SetValue/SetSelection activate
  Button, Switch, and TextInput. Scroll / virtual lists are later.
- Clipboard: JNI `ClipboardManager` (`AndroidClipboard`);
  `default_shared_clipboard()` and slot `with_clipboard` use it;
  `PlatformCapabilities::clipboard` is true.

## Device checklist

See [`docs/android.md`](../../docs/android.md). Cross-compile is not CJK or
TalkBack evidence.

```bash
source scripts/android-env.sh
./scripts/check-android-arm64.sh --build --dist
./scripts/package-android-host-apk.sh
adb install -r target-android/apk/nana-android-host-debug.apk
adb shell am start -n app.nanaui.host/.NanaActivity
adb logcat -s nana-android-host
```

Headless `-gpu swiftshader_indirect` is not claimed as wgpu evidence.
