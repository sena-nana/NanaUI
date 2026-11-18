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
