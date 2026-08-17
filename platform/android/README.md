# NanaUI Android ARM64 host (experimental, frozen)

This crate is an **experimental, frozen** NativeActivity host. **Android is not a
current NanaUI product target.** Resume only after desktop Runtime/Vue stabilize.
Do not migrate this host to Runtime, and do not treat Iced slot paint as a
shipping feature.

Rust owns QuickJS + Vue custom renderer (`VueHost`) + wgpu Vulkan Surface +
`AndroidShellStub` (Nana shell geometry). There is no System WebView.

## Layout

| Path | Role |
|------|------|
| `src/lib.rs` | `android_main` entry (`cdylib`) |
| `src/shell.rs` | `AndroidShellStub` — `WorkspaceLayout` / `WorkspaceGeometry` (DesktopShell contract) |
| `src/runtime.rs` | NativeActivity lifecycle + shell viewport + clear heartbeat |
| `src/gpu.rs` | Host-owned wgpu 30 Surface (Vulkan) |
| `src/engine.rs` | QuickJS + VueHost smoke boot |
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

- `engine-quickjs` (default) — historical Android host engine; do not enable V8 here.
- **`AndroidShellStub`** sizes Primary viewport from the same `nana-ui-core` geometry as desktop
  `DesktopShell`. `VueHost` resolves layout in that viewport. Frame presentation is wgpu clear
  plus a frozen Iced control-slot strip; this is not DesktopShell and is not a shipping paint
  path.

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

## Iced control slot (frozen host test)

- Geometry: `iced_control_slot` / `chrome_present_bands`
- Widget strip: raw Iced `button` / `text` / `toggler` plus Nana `Icon` identity.
  This host does not use `nana_ui::compatibility` adapters.
  `iced_shell_available()` stays `false`.
- Pointer: NativeActivity `MotionEvent` → iced update; Button / Switch / Input
  messages update `press_count` / `switch_on` / `input_value`.
- Keyboard: NativeActivity `KeyEvent` → iced text (US-QWERTY subset + Backspace /
  arrows). NativeActivity has no InputConnection (`text_input_*` NOP).
- Cross-compile: workspace patches `vendor/arboard` + `vendor/iced_winit` (see root `Cargo.toml`).

## Device / KeyEvent notes

| Prerequisite | Status (2026-08-10) |
|--------------|---------------------|
| NDK + `.so` + debug APK script | OK — APK buildable |
| Android Emulator AVD | OK — `nana_api34_arm64` (`system-images;android-34;google_apis;arm64-v8a`) |
| `adb install` + launch | OK on emulator (`-gpu host`) |
| KeyEvent → iced Input | OK — logcat `iced slot Input len=… in_slot=true` |
| Soft IME (`ime=true`) | NativeActivity has no InputConnection |

Reproduce (emulator):

```bash
source scripts/android-env.sh
./scripts/check-android-arm64.sh --build
./scripts/package-android-host-apk.sh
emulator -avd nana_api34_arm64 -gpu host -no-snapshot -no-audio -no-boot-anim
adb install -r target-android/apk/nana-android-host-debug.apk
adb shell am start -n app.nanaui.host/android.app.NativeActivity
adb logcat -s nana-android-host
# tap slot Input, then: adb shell input keyevent KEYCODE_H …
# expect: iced slot Input len=
```

Headless `-gpu swiftshader_indirect` boots but hit a goldfish Vulkan hang on this host — use `-gpu host` for wgpu evidence. Details: `docs/android-arm64.md`.
