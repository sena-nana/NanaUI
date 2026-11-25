# Android experimental host

Android (`platform/android`) is the repository's **experimental ARM64 host path**. It is not a current product delivery target, and not a second product rendering core — do not write it into user-facing cross-platform promises.

The HarmonyOS / OpenHarmony build tier one step below it is [OHOS](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/ohos.md): it only promises that it compiles, and has no host in the repository.

On the Android host the UI also enters `nana-ui-runtime` (`UiWorld`) and `nana-ui-scene` (`UiScene`) and is painted by the same `SceneWgpuPainter`; the window and the Vulkan graphics device are held by the host's Android `Activity` (it does not call the desktop `run_runtime`).

## Window content

The whole Surface is one Runtime document, held by the [mobile host](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/mobile-host.md) (`nana-mobile-host`); the iOS host uses the same layer. `GameActivity` lays the Surface under the status bar, the navigation bar and the soft keyboard. The document root fills the theme background color and keeps padding for the four insets (`WindowInsets` `systemBars | displayCutout` and `ime`); in landscape the notch and navigation bar on each side are avoided as well. Touches outside the primary window go to the system.

The window shows an `AndroidHostApp`. This crate's own APK (the default `probe` feature) mounts the measurement page shared by both hosts (`nana-mobile-probe`): Android has no camera in the prototype, so after turning on "CPU preview" it generates and uploads 640×480 frames at 30 fps to measure the cost of the upload path.

## Hosting an app

As with iOS's `IosApp` / `export_ios_app!`, an app crate uses the host as an rlib and builds the one cdylib its Gradle project packages:

1. Depend on `nana-android-host` with default features off (`default-features = false`), leaving out the probe page and its `android_main`.
2. Implement `AndroidHostApp`: `mount` mounts the view once, when the first window arrives, and receives a `Shell`; `before_frame` runs on every pass of the loop before the frame demand is resolved; `demand` adds the app's own deadline; `lifecycle` receives `Resumed` / `Paused` (a `Resume` from before the first window is delivered after mounting).
3. Export `android_main` with `export_android_app!(MyApp::new(), "my-tag")`, which logs to logcat under that tag. The manifest's `android.app.lib_name` names the app's cdylib, and the Activity extends `GameActivity`; the app's own Java/Kotlin code (camera, scanner and so on) and its JNI calls live on it.

`Shell` is callable from any thread: `wake()` wakes the loop (state the document reads changed on another thread), `haptic(Haptic)` plays the same five styles as iOS through the window's `View.performHapticFeedback`, and `android_app()` hands over the `AndroidApp` for the app's own JNI calls.

The manifest's `configChanges` contains `uiMode|fontScale|density`; these changes do not recreate the Activity, and the light/dark mode and font size follow the system update.

## Frame scheduling and lifecycle

The host loop decides how to wait from `FrameDemand`:

- When frames are needed (`FrameDemand::Now`), an `AChoreographer_postFrameCallback` hosted on a dedicated `ALooper` thread paces frames to display hardware vsync and wakes the main loop via `AndroidAppWaker` (avoiding `ALooper_pollOnce` `ALOOPER_POLL_CALLBACK` error logs in `android-activity`'s main-thread loop), paired with the Fifo swapchain.
- With a deadline, `poll_events` sleeps until the deadline.
- When idle, Choreographer callbacks are disarmed and `poll_events(None)` blocks until there is input, a lifecycle event, or a view task woken through `AndroidAppWaker`.

The 16 ms polling and the 500 ms heartbeat frame are gone, and in-slot multi-touch (`MotionAction::Move`/`Cancel` across all active `pointers()`) preserves concurrent `PointerId` routing into `MobileRuntime`.

`TerminateWindow` drops only the surface; the document, the state and the Device are all kept. The next `InitWindow` attaches a new surface with the same Device.

## Phase 2 platform contract (based on `GameActivity`)

Compared with the earlier `NativeActivity`, the current Android host is built on the `game-activity` feature of `android-activity` (`NanaActivity` inherits from `GameActivity`, which brings `InputConnection` and `GameTextInput`):

- **Input method (IME)**: composition updates map to `CompositionInput::Update`, commits to `Commit`, and delete-surrounding context to `DeleteSurrounding`; like touch and key events, the document's input source stamps them with a sequence number and they all go through `AppContext::route_input`, with no second Android text state machine. `EditorInfo` mirrors the `password` and `multiline` flags from the currently focused editor (`PlatformCapabilities::android_mvp().ime = true` only means that channel is connected; it is not the same as full desktop-level acceptance).
- **System clipboard**: `ClipboardHost` is implemented through JNI calls to the system `ClipboardManager` and wired into the Android branch of `default_shared_clipboard()`; the document's `HostServices` uses that backend through `read_shared_clipboard` / `write_shared_clipboard` (`clipboard = true`).
- **App data directory**: `app_data_dir` returns the sandbox's `Context.getFilesDir()` directly on Android (ignoring the `app_id` argument); the host opens it with `FileStore::open` and injects it, and the framework never writes to disk on its own.
- **Accessibility (TalkBack)**: with the `Click`, `Focus`, `SetValue` and `SetSelection` action projections wired in, `Button`, `Switch` and `TextInput` can be activated (`Click` on `TextInput` requests focus so the soft keyboard comes up; scrolling and virtual-list projections are left to a later phase).

## Cross-compiling and APK packaging

Local cross-compilation and the `nana-android-host --lib` unit tests that run on the desktop verify only the Rust-side action queue, the input/IME mapping and the accessibility projection contracts. They **cannot replace acceptance on a device for CJK IME candidate windows, the system clipboard or TalkBack touch exploration**.

```bash
# 1. Cross-compile check for ARM64 only: checks nana-ui-core, nana-ui-platform, nana-ui --lib, nana-mobile-host and nana-android-host in turn
./scripts/check-android-arm64.sh

# 2. Build the dist profile and package an installable APK (requires a configured Android NDK and the GameActivity 4.4.0 Gradle wrapper; do not enable prefab)
./scripts/check-android-arm64.sh --build --dist
./scripts/package-android-host-apk.sh
```

Build notes:
- Packaging an APK must use the `--dist` profile: a `dev` build embeds about `390 MB` of DWARF debug information and a `46 MB` symbol table in the `.so`, while `package-android-host-apk.sh` sets a `60 MiB` size limit on the stripped `.so`. The `.so` inside the APK is stored uncompressed together with `extractNativeLibs="false"`, so no second copy is unpacked after installation. More engineering details are in `platform/android/README.md`.

## Device acceptance checklist

After installing the APK on a device, check each of the following behaviours. On 2026-09-16 the first four passed on a Galaxy Z Fold4 with Android 16 (see #83), when the look page from before the move to the mobile host was still mounted; after the switch to the measurement page they need to be accepted again:

| Acceptance item | Expected behaviour | Evidence required |
| --- | --- | --- |
| **Soft keyboard show and hide** | Tapping the search field brings up the soft keyboard, and losing focus hides it; the bottom edge of the page rises with the IME inset to sit above the keyboard | Device or emulator |
| **System bar avoidance** | The top of the page is below the status bar, the bottom is above the navigation bar, and landscape avoids both sides; the theme background color shows beneath the system bars | Device, including notch / punch-hole screens |
| **Background and restore** | After going to the background and back, the text in the search field, the switches and the counter are all still there | Device or emulator |
| **No frames when idle** | Frames stop after interaction stops (the frame count in `adb shell dumpsys gfxinfo` stops growing) | Device |
| **Light/dark mode and font size** | Follows the system dark mode and system font size | Device |
| **CJK IME candidates** | Pinyin/stroke candidates are written correctly into the Runtime `TextInput`, and the host reports caret, selection, and composing span coordinates via `InputMethodManager.updateCursorAnchorInfo` | **Must use a device IME**; `adb shell input keyevent` is not a substitute |
| **System clipboard interop** | Copy and paste inside `TextInput` interoperate with the system `ClipboardManager` | Device |
| **TalkBack screen reader interaction** | `Button` activation counts, `Switch` state toggling, `TextInput` focus and entry into editing | **A real person must perform TalkBack gestures on a device** (`adb`-injected taps bypass accessibility touch exploration) |
| **Touch scrolling** | The grid flings with `ScrollPhysics::Android`, stops at the edge, and does not overshoot | Device |
| **Accessibility for scrolling and virtual lists** | Android-side scroll accessibility actions are not wired in yet | Left to a later phase |
