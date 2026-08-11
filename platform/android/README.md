# NanaUI Android ARM64 host (no WebView)

Rust owns QuickJS + **Vue custom renderer** (`VueHost`) + wgpu Vulkan Surface clear +
**`AndroidShellStub`** (Nana shell geometry). There is **no** System WebView and **no** Blitz.

**Architecture:** NanaUI = shell layout + optional generic Iced controls. Vue apps keep CSS,
custom components, and business logic — not limited to Nana widget types. What is still open on
Android is **Nana Iced shell paint** on the Surface, not Vue rendering itself.

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
./scripts/check-android-arm64.sh        # cargo check (no Blitz / paint-stub)
./scripts/check-android-arm64.sh --build
```

Artifact: `target-android/aarch64-linux-android/debug/libnana_android_host.so`

Host-side shell contract tests (no NDK UI):

```bash
cargo test -p nana-android-host --locked
```

## Features

- `engine-quickjs` (default) — Android MVP engine; do not enable V8 here.
- **`AndroidShellStub`** sizes Primary viewport from the same `nana-ui-core` geometry as desktop
  `DesktopShell`. `VueHost` resolves layout in that viewport. Frame presentation is wgpu clear
  until Nana Iced shell paint lands on the shared Surface.

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

## Iced control slot (pre-DesktopShell)

- Geometry: `iced_control_slot` / `chrome_present_bands`
- Widget strip: `IcedSlotPainter` draws **Icon + Text + Input + Switch + Button**
  via host-owned `iced_wgpu`. `iced_shell_available()` stays `false`.
- Pointer: NativeActivity `MotionEvent` → iced update; Button / Switch / Input
  messages update `press_count` / `switch_on` / `input_value`.
- Keyboard: NativeActivity `KeyEvent` → iced text (US-QWERTY subset + Backspace /
  arrows). Soft IME without KeyEvent stays open: NativeActivity has **no**
  InputConnection (`text_input_*` NOP); do not claim `ime=true` via show-only.
- Cross-compile: workspace patches `vendor/arboard` + `vendor/iced_winit` (see root `Cargo.toml`).

## Device / KeyEvent evidence

| Prerequisite | Status (2026-08-10) |
|--------------|---------------------|
| NDK + `.so` + debug APK script | OK — APK buildable |
| Android Emulator AVD | **OK** — `nana_api34_arm64` (`system-images;android-34;google_apis;arm64-v8a`) |
| `adb install` + launch | **OK** on emulator (`-gpu host`) |
| KeyEvent → iced Input | **OK** — logcat `iced slot Input len=… in_slot=true` |
| Soft IME (`ime=true`) | Still **deferred** (no InputConnection) |

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

Headless `-gpu swiftshader_indirect` boots but hit a goldfish Vulkan hang on this host — use `-gpu host` for wgpu evidence. Details: `docs/android-arm64.md`「模拟器 KeyEvent 证据」.

## Next (P2)

1. Optional physical device cross-check.
2. IME milestone: GameActivity / InputConnection — separate from NativeActivity MVP.
3. See `docs/android-arm64.md`「整体收敛状态」.
