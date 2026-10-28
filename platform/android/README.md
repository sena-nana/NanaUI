# NanaUI Android ARM64 host (experimental, phase 2)

This crate is an **experimental** GameActivity host. **Android is not a
current NanaUI product target** and is **not a second product paint kernel**.
The window is one NanaUI Runtime document — the mobile probe page shared
with the iOS host — held by `nana-mobile-host` (`platform/mobile-core`) and
painted by `SceneWgpuPainter`. It is not DesktopShell.

Phase 2 wires GameTextInput `InputConnection`, JNI `ClipboardManager`, and
TalkBack Click/Focus/SetValue/SetSelection for Button / Switch / TextInput.
The loop presents only while the document demands a frame, and
`TerminateWindow` keeps the document and device (docs/reference/mobile-host.md).
Scroll / virtual lists stay later. Cross-compile and `--lib` tests are not
device CJK / TalkBack evidence.

Rust owns the wgpu Vulkan Surface, the Runtime document and
`AndroidShellStub`. There is no System WebView and no JS engine.

## Layout

| Path | Role |
|------|------|
| `src/lib.rs` | `android_main` entry (`cdylib`) |
| `src/host.rs` | GameActivity loop over `nana-mobile-host`: lifecycle, input, insets / Configuration (JNI), frame demand |
| `src/shell.rs` | `AndroidShellStub` — window size, scale, system insets → Primary |
| `src/slot_input.rs` | Primary input gate and KeyEvent mapping |
| `src/slot_ax.rs` | AccessKit (TalkBack) publication |
| `src/slot_ime.rs` | GameTextInput buffer → `CompositionInput` (host-testable) |
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

## Primary

GameActivity lays the surface out edge to edge. **`AndroidShellStub`** takes
the status bar (with display cutout) and the navigation bar or soft keyboard
off the window (`WindowInsets` through JNI) to get Primary. The document root
paints the theme background over the whole surface and pads its content into
Primary; touches outside Primary stay `Unhandled` for the system.

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

## Window document (experimental host test)

- Appearance page: `SettingsCard` with two `SegmentedControl`s. Theme
  (浅色 / 深色 / 跟随系统) installs the built-in light or dark theme;
  跟随系统 follows `Configuration` `uiMode` night. Text size
  (跟随系统 / 小 / 标准 / 大) sets the document text scale; 跟随系统 follows
  `Resources.getConfiguration().fontScale`. The manifest handles
  `uiMode|fontScale|density` as config changes.
- Control strip: Nana Runtime `Button` / `Text` / `TextInput` / `Switch`.
  `desktop_shell_available()` stays `false`.
- Pointer: GameActivity `MotionEvent` → `RuntimeInputAdapter`.
- Keyboard / IME: GameTextInput `TextEvent` diffs into
  `ImeEvent::{Preedit,Commit,DeleteSurrounding}` via `dispatch_ime`.
  Hardware KeyEvents still cover editing keys. Soft keyboard show/hide follows
  `SlotRuntime::text_input_focused`. `PlatformCapabilities::ime` is true.
- Accessibility: `slot_ax.rs` / `accesskit_android::InjectingAdapter` publishes
  the document tree; TalkBack Click/Focus/SetValue/SetSelection activate
  Button, Switch, TextInput and the segmented options. Scroll / virtual lists
  are later.
- Clipboard: JNI `ClipboardManager` (`AndroidClipboard`);
  `default_shared_clipboard()` and the document's `HostServices` use it;
  `PlatformCapabilities::clipboard` is true.

## Device checklist

See [`docs/reference/android.md`](../../docs/reference/android.md). Cross-compile is not CJK or
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
