# NanaUI Android ARM64 host (experimental)

This crate is an **experimental** NativeActivity host. **Android is not a
current NanaUI product target.** The control slot is a basic NanaUI Runtime /
UiScene path painted by `SceneWgpuPainter`. It is not DesktopShell, not a
shipping feature, and does not claim complete IME / accessibility / CJK.

Rust owns V8 (desktop smoke) + Vue custom renderer (`VueHost`) + wgpu Vulkan Surface +
`AndroidShellStub` (Nana shell geometry). There is no System WebView.

## Layout

| Path | Role |
|------|------|
| `src/lib.rs` | `android_main` entry (`cdylib`) |
| `src/shell.rs` | `AndroidShellStub` — `WorkspaceLayout` / `WorkspaceGeometry` |
| `src/runtime.rs` | NativeActivity lifecycle + shell viewport + Scene present |
| `src/gpu.rs` | Host-owned wgpu 30 Surface (Vulkan) |
| `src/slot_runtime.rs` | RuntimeDocument + pointer/key into Runtime |
| `src/engine.rs` | V8 + VueHost smoke boot (desktop); Android cross-build skips V8 |
| `app/` | Optional Gradle wrapper notes (load `libnana_android_host.so`) |

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
  `--no-default-features` (`docs/android.md`).
- **`AndroidShellStub`** sizes Primary viewport from the same `nana-ui-core` geometry as desktop
  `DesktopShell`. `VueHost` resolves layout in that viewport. Frame presentation is wgpu chrome
  fill plus a NanaUI Runtime control-slot strip; this is not DesktopShell.

## Packaging

Preferred (works with this workspace):

```bash
source scripts/android-env.sh
./scripts/check-android-arm64.sh --build
./scripts/package-android-host-apk.sh
# → $CARGO_TARGET_DIR/apk/nana-android-host-debug.apk
```

`cargo-apk` can be installed (`cargo install cargo-apk`) but **0.10 cannot parse** this
repo’s root `Cargo.toml` (multiline inline tables). Use the script above instead.
Metadata under `[package.metadata.android]` remains for documentation / future tools.
Requires SDK `build-tools` (e.g. `build-tools;34.0.0`).

## NanaUI control slot (experimental host test)

- Geometry: `control_slot` / `chrome_present_bands`
- Widget strip: Nana Runtime `Button` / `Text` / `TextInput` / `Switch`.
  `desktop_shell_available()` stays `false`.
- Pointer: NativeActivity `MotionEvent` → `RuntimeInputAdapter`.
- Keyboard: NativeActivity `KeyEvent` → Runtime text (US-QWERTY subset + Backspace /
  arrows). The soft keyboard is shown/hidden from the Runtime focus mirror
  (`SlotRuntime::text_input_focused` / `HostState::sync_soft_input`); committed text
  arrives as hardware-style KeyEvents — NativeActivity has no InputConnection, so no
  composition/preedit.
- Accessibility: phase one publishes the control-slot tree (name/role/value) via
  `slot_ax.rs` / `accesskit_android::InjectingAdapter`; reader actions are not
  mapped back into Runtime yet.
- Clipboard: Android does not compile `arboard` / `OsClipboard`. Hosts use
  `UnsupportedClipboard`; `PlatformCapabilities::clipboard` stays false.

## Device / KeyEvent notes

| Prerequisite | Status |
|--------------|--------|
| NDK + `.so` + debug APK script | Buildable (cross-check, not device acceptance) |
| Soft IME (`ime=true`) | Soft keyboard shown/hidden from the focus mirror; committed text as KeyEvents. No composition/preedit (NativeActivity has no InputConnection) |
| Accessibility | Phase one publishes name/role/value (`slot_ax.rs`); reader actions pending |

Reproduce (emulator; not claimed as current acceptance):

```bash
source scripts/android-env.sh
./scripts/check-android-arm64.sh --build
./scripts/package-android-host-apk.sh
emulator -avd nana_api34_arm64 -gpu host -no-snapshot -no-audio -no-boot-anim
adb install -r target-android/apk/nana-android-host-debug.apk
adb shell am start -n app.nanaui.host/android.app.NativeActivity
adb logcat -s nana-android-host
# tap slot Input, then: adb shell input keyevent KEYCODE_H …
```

Headless `-gpu swiftshader_indirect` boots but hit a goldfish Vulkan hang on this host — use `-gpu host` for wgpu evidence. Details: `docs/android.md`.
