# Packaging and distribution contract

This page specifies the packaging configuration (`nana-package.toml`), the resource pack binary format (`.nrpack`), tiered keys and publisher signing, and the automated tamper verification of the final deliverable (Issue #226). For day-to-day quick-start steps, start with [Packaging](https://github.com/sena-nana/NanaUI/blob/main/docs/guide/scaling/packaging.md) in the guide.

Distribution is mainly Steam and standalone installers: a normal launch always has **one main process and one executable**, with no resident launcher, no extra IPC, and no A/B directory switching. `nana-packager` does not call `cargo` in reverse; it takes an already built executable and completes packaging and validation:

```text
cargo build --profile dist
        │  the built executable
        ▼
nana-packager package                 reads nana-package.toml and the resource directory
        │  pack plan → .nrpack → platform directory layout → manifest → signing → Steam metadata
        ▼
<out>/app/                            final deliverable directory / .app bundle
        │
nana-packager validate --run --tamper-suite
```

## 1. Module split and dependency boundaries

| Crate / module | Responsibility | Enters the application binary? |
| --- | --- | --- |
| `nana-package` | Reads the `package.json` manifest, parses the read-only `.nrpack`, the `KeyProvider` interface, publisher Ed25519 signature verification, binary identity tag parsing | **Yes** (enabled on demand through `nana-ui/packaged-resources`; pure Rust dependencies only, zero C libraries, zero randomness source) |
| `nana-packager` | Parses `nana-package.toml`, generates the pack plan, compresses and encrypts `.nrpack`, assembles the platform directory layout, produces Steam metadata, and runs the CLI for final-artifact and anti-tamper validation | **No** (a pure build-time tool) |
| `nana-ui-core::packaged` | `nana://res/` logical path normalization and the process-wide `PackagedResourceSource` mount hook | **Yes** (zero external dependencies) |

`scripts/check-package-boundary.py` in CI enforces these boundaries: `nana-package` must not depend on the C `zstd`, `ring`, `rustls`, `getrandom` or `rand`, and `blake3` must use the `pure` backend; embeddable Runtime-style crates must not depend on packaging crates; and no workspace member may point a non-dev dependency at `nana-packager`.

## 2. Logical locations and cross-platform install layout

Business code obtains logical directories uniformly through `ApplicationPaths` (see [Diagnostics and app paths](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/diagnostics.md)); never hard-code relative paths such as `./runtime/...`:

| Logical location | Windows / Linux | macOS (`.app` bundle) |
| --- | --- | --- |
| Application root | `App/` (only the main program `App.exe` and `runtime/`) | `Name.app` |
| Main executable | `App.exe` / `App` | `Contents/MacOS/App` |
| `RuntimeBin` | `runtime/bin` | `Contents/Frameworks` |
| `RuntimeResources` | `runtime/resources` | `Contents/Resources` |
| `RuntimePlugins` | `runtime/plugins` | `Contents/PlugIns` |
| `RuntimeTools` | `runtime/tools` | `Contents/Helpers` |
| `RuntimeManifest` | `runtime/manifest` | `Contents/Resources/manifest` |

`[runtime]`'s `bin`, `tools` and `resources` are copied to `RuntimeBin`, `RuntimeTools` and `RuntimeResources` respectively. An entry may be a directory: copied whole, with symbolic links preserved as they are (macOS `.framework` relies on the `Versions/Current` link); links inside a directory that point outside it (absolute paths or `..` that escapes the directory) are rejected; the manifest records each file inside one by one, and a link to a file counts by its target's content.

Linux installers (`deb`, AppImage, Flatpak) lay the same tree out in FHS form: the executable and the architecture-dependent `runtime/{bin,plugins,tools}` under `<prefix>/lib/<pkg>/`, the architecture-independent `resources` and `manifest` under `<prefix>/share/<pkg>/`, plus a `<prefix>/bin/<exe>` link, an `<id>.desktop` entry and a 256×256 icon. At run time `ApplicationPaths` recognizes an executable under `<prefix>/lib/<pkg>/` (or `libexec`) whose prefix is `/usr`, `/usr/local`, an AppImage's `$APPDIR/usr` or Flatpak's `/app`, and reads `RuntimeResources` / `RuntimeManifest` from the first of `<prefix>/share` and the `$XDG_DATA_DIRS` entries that holds `<pkg>/manifest`, else from `<prefix>/share`. A self-contained directory such as `/opt/<app>/` still reads the sibling `runtime/` of the table above. The self-check's `files.present` looks for every file the manifest lists where the resolved locations put it.

By default the Windows application root may only contain the main `.exe`; files that must sit next to the `.exe` because a third-party SDK requires it (such as Steamworks' `steam_api64.dll`) must be registered explicitly in `platform.windows.root_exceptions` with the file path and the reason. When `distribution.backend = "portable"`, the packager writes the `runtime/manifest/portable` marker, and runtime writable data all goes into `<root>/data`; an installed application never writes any data, cache or log into the install directory.

## 3. Configuration file `nana-package.toml` and key passing

```toml
schema_version = 1

[application]            # must match the NANA-IDENTITY-V1 tag compiled into the binary exactly
id = "dev.nana.fixture"
name = "Nana Fixture"
version = "0.1.0"
vendor = "Nana"

[executable]
file_name = "NanaFixture"            # produces NanaFixture.exe on Windows

[distribution]
backend = "steam"                    # steam | installer | portable
[distribution.steam]                 # public ids only; never put account credentials here
app_id = 480
depots = { windows = 481, macos = 482, linux = 483 }

[platform.macos]
minimum_system_version = "14.0"
url_schemes = ["nanafixture"]
document_types = [{ name = "Fixture Doc", extensions = ["nfx"], role = "Editor" }]
info_plist_extra = "packaging/Info.extra.plist"

[platform.windows]
root_exceptions = [{ source = "vendor/steam_api64.dll", reason = "Steamworks loads it from the exe directory" }]

[runtime]                            # source may be a directory: copied whole, symlinks preserved (.framework)
bin     = [{ source = "vendor/foo.dll", platforms = ["windows"] },
           { source = "vendor/Foo.framework", platforms = ["macos"] }]
tools   = [{ source = "vendor/ffmpeg" }]
resources = [{ source = "LICENSES.txt" }]   # loose files kept out of packs → RuntimeResources
plugins = [{ name = "fx", source = "vendor/fx.plugin", abi = "nana-plugin-1", version = "1.0.0" }]

[resources]
root = "assets"                      # in-pack logical path = the path relative to root
block_size = 65536
compression = { codec = "zstd", level = 19 }
max_free_ratio = 0.25

[[resources.packs]]
name = "bootstrap"
class = "bootstrap-ui"
include = ["boot/**"]

[[resources.packs]]
name = "ui"
class = "protected"
include = ["ui/**"]
exclude = ["ui/**/*.psd"]
key = "content-main"
depends_on = ["bootstrap"]
max_bytes = 536870912

[keys.content-main]
generation = 1
availability = "process-start"       # embedded | process-start | after-bootstrap-ui

[signing]
publisher = { public_key = "ed25519:…", required = true }
platform = ["macos-codesign", "windows-authenticode"]
```

With `backend = "installer"`, `[distribution.installer]` chooses the installers to build; see section 7.

Configuration parsing is strict: unknown fields are always an error; any field name containing `secret`, `private`, `token`, `password` or `credential` is rejected outright with a pointer to the secure channel. Keys may only be read from environment variables (`NANA_CONTENT_KEY_<NAME>` / `NANA_PUBLISHER_SIGNING_KEY`) or from key files outside the git worktree (`--content-key-file NAME=PATH` / `--signing-key-file PATH`); before packaging finishes it also scans every output artifact in full (covering raw bytes, case-varied hex, Base64 and Base64Url), and on finding any key material residue it deletes the artifacts at once and exits with an error.

## 4. The `.nrpack` pack format, encryption and Steam delta-friendly layout

### Startup resource tiers (`ResourceClass`)

Files matched under `resources.root` must belong to exactly one pack (duplicate claims, case-only differences, and names containing `..` or backslashes are all errors), and the runtime routes by longest path prefix. Packs fall into three tiers by startup order (`depends_on` may only point at the same tier or an earlier one):

| `class` | Startup constraint and purpose |
| --- | --- |
| `early-splash` | **Not encrypted**, total size no more than `1 MiB` (per-file limit `4 MiB`), depends on no other pack, read directly by the native splash for the logo |
| `bootstrap-ui` | May be encrypted, but only with a key available in the `embedded` or `process-start` phase (referencing an `after-bootstrap-ui` key raises `StartupKeyCycle`, preventing a deadlock on a blank window while waiting for UI login to fetch a key) |
| `protected` | Ordinary business resources, may use a content key from any phase (including `after-bootstrap-ui`, issued after login authentication) |

### Binary layout and the eight-step fail-closed read

```text
[Header: 4096 B][Data Region: each entry gets a 16 B aligned extent, gaps zero-filled][TOC: 4096 B aligned]
```

- **Per-block independent compression and encryption**: each resource entry is cut into blocks by `block_size` (64 KiB by default), each block independently compressed with `zstd` (the packager encodes with the C library for a high compression ratio, the runtime decodes with pure-Rust `ruzstd`; if compression does not shrink a block, the original bytes are stored), and encrypted packs seal each block independently with `XChaCha20-Poly1305` (random 192-bit nonce, AAD binding the pack ID, entry key, block index, key generation, plaintext length and flags); the TOC is likewise encrypted and sealed as a whole.
- **Eight-step strict read order**: ① verify the Header and reserved bits → ② verify the Ed25519 publisher signature against the `TrustPolicy` → ③ check the `pack_id`, TOC hash, `class` and key generation pinned by `package.json` → ④ verify the BLAKE3 of the stored TOC → ⑤ fetch the key from the `KeyProvider`, decrypt and authenticate the TOC → ⑥ check `max_bytes` before reading data → ⑦ locate the target blocks on demand, verify each `record_hash`, decrypt the AEAD and decompress within limits → ⑧ verify the BLAKE3 of the whole entry. Any step failing aborts at once; it **never decrypts to a temporary folder and never decrypts again every frame**.

### SteamPipe delta update optimization

When packaging with `--baseline <previous out>` or `--cache <DIR>`, the ciphertext blocks of unchanged entries (including their original nonces) stay as they are at their original file offsets (extents); an entry that changed but still fits is overwritten in place and its tail zero-filled; new or grown entries fill free holes (first-fit) or are appended at the end (compactification only when the free ratio exceeds `max_free_ratio` or `--compact` is passed). Changing one small resource therefore only touches the Header, that entry's extent and the few 1 MiB blocks holding the TOC, maximizing reuse of SteamPipe delta patches (guarded by the gate in `crates/nana-packager/tests/steam_delta.rs`).

## 5. Application-side mounting and final-artifact tamper validation (`validate`)

The application embeds the `NANA-IDENTITY-V1` identity tag in the binary through the `application_identity!` macro and mounts resource packs through `NanaApplication::builder(identity).resource_packs(...)` (without a manifest during development it falls back to reading the `loose_root` directory; stylesheets, `@font-face` and `url()` all use `nana://res/...` paths).

Run the end-to-end validation and anti-tamper test suite before release:

```bash
nana-packager validate "out/app/Nana Fixture.app" --trust-key ed25519:… --run --tamper-suite \
    --env NANA_FIXTURE_CONTENT_KEY=…
```

`validate` checks in order:
1. the Ed25519 publisher signatures of `package.json` and every `.nrpack` (`--trust-key` is required);
2. the single-entry constraint at the root, every file's size and BLAKE3 hash (add `--allow-resigned` after platform signing), and the binary identity tag's consistency;
3. the reachability of the PE / Mach-O / ELF dynamic library import tables, and that the Steam build carries no self-updater marker (`NANA-UPDATER-V1`);
4. `--run`: launch the application in an unrelated working directory with `NANA_PACKAGE_VALIDATE=1` to run the pre-window self-check;
5. `--tamper-suite`: first run the untampered control group (`tamper.control`), then inject six kinds of tampering in turn — “corrupt the manifest, corrupt the pack Header, corrupt the TOC, corrupt the first real data block, delete a pack, strip key environment variables” — and verify that the application's self-check reports and blocks every one accurately, without crashing or hanging.

## 6. Platform signing adapters

NanaUI does not reimplement codesign, notarization, Authenticode, GPG, apksigner or the HarmonyOS hap-sign-tool: each adapter wraps the platform tool and reports what actually happened in this run. When the tool cannot run here — wrong host, tool missing, credentials missing, nothing to sign — it reports `not-executed` with the reason, never signed. Credentials are read only from CI environment variables, never from `nana-package.toml`, the manifest or a report; a password a tool can take from its environment or stdin never goes on its command line, and those that can only go there (signtool's PFX password, hap-sign-tool) are masked in every report and error.

Signing runs in two stages. The **application stage** signs the application tree in place after the manifest is written (codesign seals the manifest into the bundle, and both rewrite executables, so the manifest records pre-signing hashes; validate with `--allow-resigned`). The **artifact stage** signs what the installer backends built, or files built elsewhere through `nana-packager sign`.

| Adapter | Stage | Host | What it does | Credentials and settings |
| --- | --- | --- | --- | --- |
| `macos-codesign` | application | macOS | Signs the Mach-O files and nested bundles under `Frameworks`, `PlugIns`, `Helpers` and `MacOS` inside out, then the `.app`; a real identity gets the hardened runtime and a timestamp; verified with `codesign --verify --strict --deep` | `NANA_MACOS_SIGN_IDENTITY` (`Developer ID Application: …`, or `-` for an ad-hoc development signature), optional `NANA_MACOS_KEYCHAIN`; `[platform.macos] entitlements` |
| `macos-notarize` | artifact | macOS | With a `.dmg`: signs the image, submits it and staples it; otherwise zips the `.app`, submits it and staples the `.app`; verified with `stapler validate` and `spctl --assess`. Needs a Developer ID signature from codesign in the same run | `NANA_NOTARY_KEYCHAIN_PROFILE`, or `NANA_NOTARY_API_KEY` (the `.p8` path) + `NANA_NOTARY_API_KEY_ID` + `NANA_NOTARY_API_ISSUER` |
| `windows-authenticode` | application, artifact | Windows | The application stage always signs the main `.exe` and signs other PE files only when they carry no valid signature (a vendor's signed DLL keeps its own); the artifact stage signs `.msi` and installer `.exe`; verified with `signtool verify /pa /all` | `NANA_AUTHENTICODE_CERT_SHA1` (certificate store thumbprint, recommended), or `NANA_AUTHENTICODE_PFX` + `NANA_AUTHENTICODE_PFX_PASSWORD`; timestamp from `[platform.windows] timestamp_url` or `NANA_AUTHENTICODE_TIMESTAMP_URL` (no timestamp, no signature); optional `NANA_SIGNTOOL` |
| `msix` | artifact | Windows | Signs `.msix` / `.appx` (the packager builds no MSIX; for packages built elsewhere) | As Authenticode |
| `linux-package` | artifact | any host with gpg | Detached ASCII signatures `<file>.asc` for `.deb`, `.AppImage`, `.flatpak`, verified with `gpg --verify` | `NANA_GPG_KEY_ID`; optional `NANA_GPG_PASSPHRASE` (through stdin), `NANA_GPG_HOMEDIR`, `NANA_GPG` |
| `android-apksigner` | artifact (`sign` command only) | any | `apksigner sign` (signature scheme v2+), verified with `apksigner verify`; passwords passed as `env:` references | `NANA_ANDROID_KEYSTORE`, `NANA_ANDROID_KEY_ALIAS`, `NANA_ANDROID_KEYSTORE_PASSWORD`, optional `NANA_ANDROID_KEY_PASSWORD`; `NANA_APKSIGNER` or the newest build-tools under `ANDROID_HOME` |
| `ohos-hapsigner` | artifact (`sign` command only) | any host with Java | `hap-sign-tool.jar sign-app` (localSign), then replaces the input `.hap`; verified with `verify-app` | `NANA_OHOS_HAP_SIGN_TOOL`, `NANA_OHOS_KEYSTORE`, `NANA_OHOS_KEYSTORE_PASSWORD`, `NANA_OHOS_KEY_ALIAS`, `NANA_OHOS_KEY_PASSWORD`, `NANA_OHOS_APP_CERT`, `NANA_OHOS_PROFILE`, optional `NANA_OHOS_SIGN_ALG`, `NANA_JAVA` |

`signing.platform` takes only the first five, those that apply to a packaging target. `android-apksigner` and `ohos-hapsigner` sign APKs and HAPs the packager does not build; listing them in the config is an error pointing at the `sign` command:

```bash
nana-packager sign --adapter android-apksigner app-release.apk
nana-packager sign --adapter macos-codesign --app "out/app/Nana Fixture.app"
```

`sign` exits 0 when every adapter verified, 2 when some did not run, 1 on failure. `package-report.json` records each adapter's outcome in `platform_signing` (application stage) and `artifact_signing` (artifact stage); the manifest only records which adapters are configured. `validate` calls each adapter's verification again.

## 7. Installer backends

```toml
[distribution]
backend = "installer"

[distribution.installer]
formats = ["dmg", "msi", "nsis", "deb", "appimage", "flatpak"]   # only those of the packaging target are built
summary = "One-line summary"         # the application name by default
description = "Longer text"
homepage = "https://example.com"
maintainer = "Nana <dev@example.com>" # deb Maintainer; the vendor or name by default
linux_package = "nana-fixture"       # /usr/lib/<pkg>; derived from the name by default
categories = ["Utility"]
deb_depends = ["libc6 (>= 2.31)"]
icon_png = "packaging/icon-256.png"  # the Nana mark by default
msi_upgrade_code = "{…}"             # derived from the application id by default; never change it once shipped
flatpak_runtime = "org.freedesktop.Platform"
flatpak_runtime_version = "24.08"
flatpak_sdk = "org.freedesktop.Sdk"
```

| Format | Target | Built by | Built on |
| --- | --- | --- | --- |
| `dmg` | macOS | `hdiutil`: the `.app` copied with `ditto` (signature, extended attributes and framework links intact) plus an `/Applications` link, UDZO-compressed, checked with `hdiutil verify` | macOS |
| `msi` | Windows | A WiX v4+ `.wxs` generated from the application tree, built with `wix build` (`NANA_WIX`). Installs per machine into `Program Files\<Name>` with a Start menu shortcut; a stable upgrade code upgrades in place and refuses downgrades | Windows |
| `nsis` | Windows | A generated `.nsi` compiled with `makensis` (`NANA_MAKENSIS`). The uninstaller removes only the files it installed and the directories they leave empty, never `RMDir /r` | any host with makensis |
| `deb` | Linux | Written by the packager itself in pure Rust, in FHS form; parsed back before it is reported built, and checked with `dpkg-deb` where available | any host |
| `appimage` | Linux | A generated AppDir (`AppRun` → `usr/bin/<exe>`, `<id>.desktop`, icon, FHS `usr/`), packed with `appimagetool` (`NANA_APPIMAGETOOL`) | Linux |
| `flatpak` | Linux | A generated `flatpak-builder` manifest whose one module copies the staged tree into `/app`; `flatpak-builder`, then `flatpak build-bundle`. The build host needs the runtime and SDK installed | Linux |

Installers go to `<out>/installer/`. A format whose tool cannot run here reports `not-executed`, and its generated `.wxs`, `.nsi`, AppDir or Flatpak manifest stays in `installer/staging/<format>/` to finish the build on the right host; a tool that runs and fails fails the whole packaging run. `installers` in `package-report.json` records each format's file, size, BLAKE3 and checks. `backend = "installer"` without `[distribution.installer]` produces only the application tree, with a warning. `validate` checks only the application tree; installing, launching and uninstalling an installer on its target system is not automated (reported as `installer.round-trip` not executed).

## 8. Android APK resources

APK assets are not filesystem paths. `nana-packager android-assets APP_ROOT --out app/src/main/assets` copies a package output's `package.json` (and signature) and every `.nrpack` into `assets/nana/manifest/` and `assets/nana/resources/`. The APK must store `.nrpack` uncompressed:

```kotlin
android { androidResources { noCompress += "nrpack" } }
```

On Android, `nana-ui/packaged-resources` first reads `assets/nana/manifest/` through the `AAssetManager` of the application's `Context`, checking the signature as the `TrustPolicy` demands (`PackageManifest::from_signed_bytes`); when the APK has one, packs mount from the APK, otherwise it falls back to `RuntimeManifest`. Each pack gets the APK's descriptor and the asset's byte range from `AAsset_openFileDescriptor64` and goes to `PackReader::open_source` as a `FileRange`: the same on-demand positional reads and the same eight-step checks as on the desktop, with nothing extracted to a temporary file. A compressed pack has no descriptor and is reported as an error. The `PackagedResourceSource` contract is unchanged.

## 9. Update manifests and the rollback journal

NanaUI ships no updater and no update UI. `nana_package::update` is the contract an application's own updater (or a store) builds on:

- **Update manifest** `update.json` + `update.json.sig`: the target application id, version, channel, target triple, an optional `minimum_from_version`, the BLAKE3 of the `package.json` it installs, and the format, size and BLAKE3 of each installer file. It is signed with the same publisher key as the package manifest, under a separate signature context, so neither signature passes for the other. `nana-packager update-manifest APP_ROOT --channel stable --artifact out/installer/… --out DIR` writes it; without the publisher key it writes nothing — there is no such thing as an unsigned update.
- **Verification** `UpdateManifest::verify(bytes, sig, installed, policy)`: the signature is checked before parsing; then the application id, target triple and channel must match, the version must be **strictly newer** than the installed one (SemVer order, prereleases before the release), and the installed version must be at least `minimum_from_version`. A validly signed older release therefore cannot be replayed as an update. `check_artifact` streams a download against its size and hash.
- **Rollback journal** `UpdateJournal`: the updater records the previous and pending versions before replacing files (`staged`) and calls `activated` after. Every launch calls `begin_launch` before any window and persists the journal right away: running the pending version counts one unconfirmed launch and returns `AwaitConfirmation`; reaching the healthy point (the first `UiReady`) calls `confirm`. Once the pending version has had `max_unconfirmed_launches` (2 by default) launches without confirming, the next launch returns `RollBack { restore }`; the application's own restore hook brings the previous version back and calls `rolled_back`. Where the journal lives and how files are restored belong to the application.

`distribution.self_update` is still refused: the packager builds no updater, and neither Steam nor installer packages carry `NANA-UPDATER-V1`. Verifying the packaged Early Splash → UiReady handoff on a real window is Issue #273.
