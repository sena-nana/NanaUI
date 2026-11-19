# HarmonyOS / OpenHarmony (OHOS) Compile Tier

OHOS is a support tier **one level below** the [Android experimental host](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/android.md): the repository promises only that the core crates compile for OHOS and that platform branches behave correctly on OHOS. There is no OHOS host in the repository; the window, XComponent, IME, and clipboard integration are all done by the application. Do not include it in user-facing cross-platform promises.

UI on OHOS goes through the same `UiWorld` → `UiScene` → `SceneWgpuPainter` path, with no separate tree, no separate painter, and no WebView shell.

## Target and cfg

| Item | Value |
| --- | --- |
| Supported target | `aarch64-unknown-linux-ohos` |
| `target_os` | `linux` |
| `target_env` | `ohos` |

The `target_os` of OHOS is `linux`, so any desktop branch written as `target_os = "linux"` (X11 / Wayland, portal, AT-SPI, ALSA, XDG paths, fontconfig XML) is entered by mistake on OHOS. Rules:

- Always detect OHOS with `target_env = "ohos"`.
- When adding a `target_os = "linux"` desktop branch or target dependency, also add `not(target_env = "ohos")`.

Branches already isolated under this rule:

| Location | Behavior on OHOS |
| --- | --- |
| `nana-ui-platform`'s `arboard` dependency, `OsClipboard` | Not compiled |
| `default_shared_clipboard()` | Returns `UnsupportedClipboard`; see Clipboard below |
| `app_data_dir()` | Returns `None`; does not fall back to `~/.local/share` |
| `PathPlatform::current()` | Returns `None`; `PathEnvironment::current()` reports `UnsupportedPlatform("ohos")` |
| `nana-ui`'s `accesskit_unix` | Not compiled |
| `nana-ui`'s `hosted` | `compile_error!`: it is the winit / `nana-window` desktop host and has no OHOS backend |
| `nana-text` system font discovery | Does not use fontdb's Linux scan; see Fonts below |

## What is promised

- The crates listed in `scripts/check-ohos-arm64.sh` pass `cargo check` for `aarch64-unknown-linux-ohos`; CI's `ohos-arm64-cross` runs this every time.
- The platform branches in the table above take OHOS behavior on OHOS and do not enter desktop branches.
- The font discovery and fallback chain logic has host unit tests (with an injectable root directory), so it can be verified without a device.

## What is not promised

- **No official host template.** There is no `platform/ohos` in the repository, no ArkTS project, HAP packaging, or XComponent example.
- **No device or emulator CI.** CI only does a cross `cargo check`. There is no linking, no `.so`, and no code has run on a device.
- **The window and XComponent host are integrated by the application.** Building a `GpuContext` and surface from XComponent's `OHNativeWindow`, frame scheduling, lowering touch and keys into `nana-ui-input` canonical events, IME integration, and the accessibility projection have not been verified on OHOS. Whether wgpu can create a surface on OHOS is likewise unverified.
- **The `fetch` / GPU paths are not in CI.** The build script of `fetch` (ureq → rustls → ring) needs a C compiler for the target platform even for `check`, and CI has no OHOS SDK installed. Only after `OHOS_NDK_HOME` is set does the script check `nana-ui-platform`'s default features and `nana-ui --features gpu`.
- **Pure-Rust cross-check without an SDK**: `nana-ui-runtime` no longer depends on `stacker` or `psm` assembly, so every crate checked without `OHOS_NDK_HOME` is pure Rust (`scripts/check-ohos-arm64.sh` still checks for host `clang` on `PATH` for cross toolchain setup, as host `gcc` only targets x86).
- **Font configuration is verified only against the open-source sample.** Parsing of `fontconfig.json` is checked against `src/ports/skia_ohos/config/fontconfig.json` in the OpenHarmony Skia port repository; no on-device file has been read. Family aliases in `generic` (such as `HarmonyOS-Sans-Bold`) are recognized only for CSS generic family names; other aliases are not mapped.
- ARM64 only. `armv7-unknown-linux-ohos` and `x86_64-unknown-linux-ohos` are not checked.

## Host integration contract

The application's OHOS host owns the window, surface, `GpuContext`, each frame's `FrameContext`, and frame scheduling, and builds `SceneWgpuPainter` on its own context like any other host. Framework-side conventions:

- **Clipboard**: OHOS does not compile the desktop clipboard and has no default backend. The host implements `HostServices::read_clipboard` / `write_clipboard` with the system Pasteboard (or implements `ClipboardHost` and wraps it with `shared_clipboard(...)` for shared reads and writes). Without injection, `default_shared_clipboard()` is `UnsupportedClipboard`, and reads and writes both return `HostServiceError::Unsupported` without panicking.
- **Application data directory**: only the ArkTS `Context` knows the sandbox's files directory. After obtaining `filesDir`, the host calls `FileStore::open` itself; the framework never writes to disk on its own.
- **Desktop host**: do not enable `nana-ui`'s `hosted` on OHOS; use `--lib` and `gpu`, as with the Android host.

## Fonts

`FontSystem::with_system_fonts()` on OHOS:

1. Reads `/system/etc/fontconfig.json` (the OpenHarmony Skia port format). Only three keys are used: `fontdir` (which directories to scan), `generic` (aliases for CSS generic family names such as `serif` and `monospace`), and, in `fallback`, the default set `""` mapping BCP 47 tags → family lists. `font_file_map` is not read: the file names it lists do not always exist on the device, and each face's family name is read from the font's own name table anyway.
2. Scans the directories in the configuration that exist; when the configuration is missing, fails to parse, or none of its directories exist, scans `/system/fonts`.
3. The fallback policy puts the configured families first, then the built-in HarmonyOS chain:

| Case | Built-in families |
| --- | --- |
| `sans-serif` / `system-ui` | HarmonyOS Sans → HarmonyOS Sans SC → Noto Sans |
| Simplified Chinese (`Hani`) | HarmonyOS Sans SC |
| Traditional Chinese (`zh-Hant` / `zh-TW` / `zh-HK` / `zh-MO`) | HarmonyOS Sans TC |
| Japanese / Korean | Noto Sans CJK JP / Noto Sans CJK KR |
| Arabic | HarmonyOS Sans Naskh Arabic UI |
| Emoji | HMOS Color Emoji → HMOS Color Emoji Flags |
| Symbols | Noto Sans Symbols → Noto Sans Symbols 2 → HM Symbol |

Scripts present in the configuration but not in the built-in chain (such as `und-Gujr`) use only the configured families. On OHOS, `FallbackPolicy::platform_default()` returns only the built-in chain and reads no files.

## Differences from the Android tier

| | Android experimental host | OHOS compile tier |
| --- | --- | --- |
| In-repo host | `platform/android` (`GameActivity`) | None |
| CI | Cross check, with NDK | Cross check, no SDK installed, excluding `fetch` / GPU |
| Build artifacts | `.so` and an installable APK | None |
| Device acceptance | Has a checklist; some items pass on devices | None |
| Clipboard | JNI `ClipboardManager`; `default_shared_clipboard()` works directly | Injected by the host; `Unsupported` by default |
| Application data directory | `app_data_dir` returns `getFilesDir()` | `app_data_dir` returns `None`; passed in by the host |
| IME / accessibility | GameActivity IME and TalkBack actions wired | Not wired; handled by the host |
| System fonts | Scans `/system/fonts` | Reads `fontconfig.json`, scans directories as a fallback, HarmonyOS fallback chain |

## Checking

```bash
rustup target add aarch64-unknown-linux-ohos
./scripts/check-ohos-arm64.sh
```

No linker is needed; without an OHOS SDK, the host `clang` is required (see above). To also check the `fetch` / GPU paths, first install the native part of the OpenHarmony SDK, then:

```bash
OHOS_NDK_HOME=/path/to/ohos-sdk/linux ./scripts/check-ohos-arm64.sh
```

`OHOS_NDK_HOME` points to the SDK directory containing `native/llvm` and `native/sysroot` (the `native` directory itself is also accepted); if `CC_aarch64_unknown_linux_ohos` is already set, it is used directly.
