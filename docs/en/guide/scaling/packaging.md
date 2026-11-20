# Packaging

A release is one main process and one executable. Packaging adds no resident launcher and does not look for A/B directories. The packager does not invoke cargo. You build first, then package, then validate.

```text
cargo build --profile dist
        │
nana-packager package
        │
<out>/app/
        │
nana-packager validate --run --tamper-suite
```

Three pieces split the work: `nana-package` reads the manifest and `.nrpack` inside the application (through `nana-ui`'s `packaged-resources`, off by default); `nana-packager` is only a build tool, and applications must not depend on it; `nana-ui-core::packaged` handles `nana://res/` paths. `scripts/check-package-boundary.py` guards the boundary. External hosts can skip this packaging entirely.

## Install layout

Application code gets logical locations from `ApplicationPaths`; do not assemble `./runtime/...` yourself.

| Logical location | Windows / Linux | macOS |
| --- | --- | --- |
| Application root | `App/` | `Name.app` |
| Executable | `App.exe` / `App` | `Contents/MacOS/App` |
| RuntimeBin | `runtime/bin` | `Contents/Frameworks` |
| RuntimeResources | `runtime/resources` | `Contents/Resources` |
| RuntimePlugins | `runtime/plugins` | `Contents/PlugIns` |
| RuntimeTools | `runtime/tools` | `Contents/Helpers` |
| RuntimeManifest | `runtime/manifest` | `Contents/Resources/manifest` |

The Windows root holds one EXE. Files that must sit next to it (such as `steam_api64.dll`) go in `platform.windows.root_exceptions`, with a reason. `backend = "portable"` writes `runtime/manifest/portable`, and writable data goes to `<root>/data`. Installed builds never write data, caches, logs or crash files into the install directory.

## Resource packs

`nana-package.toml` has `schema_version = 1`. Unknown fields are rejected. A field name containing secret, private, token, password or credential is an immediate error.

Content keys are read from the environment variable `NANA_CONTENT_KEY_<NAME>` (64 hex digits) or `--content-key-file NAME=PATH`. The publisher signing seed is read from `NANA_PUBLISHER_SIGNING_KEY` or `--signing-key-file PATH`. A key file inside the git worktree that holds the config is rejected.

Every file under `resources.root` goes into exactly one pack. Unclaimed files are not packaged. A file claimed twice, paths that differ only in case, `..`, backslashes or control characters are all errors. At runtime, lookups are routed by longest prefix to exactly one pack.

| class | Constraints |
| --- | --- |
| `early-splash` | Not encrypted, at most 1 MiB, no dependencies on other packs |
| `bootstrap-ui` | May be encrypted, only with `embedded` or `process-start` keys |
| `protected` | Ordinary application resources |

If `bootstrap-ui` uses an `after-bootstrap-ui` key, the result is `StartupKeyCycle`. `depends_on` may only point to the same class or an earlier one. `SplashLogo::packaged("nana://res/…")` reads the logo from the `early-splash` pack. The reader does not call `KeyProvider`; a pack over 4 MiB (`EARLY_SPLASH_MAX_PACK_BYTES`) yields `EarlySplashTooLarge` without reading the TOC. The 1 MiB limit is checked again against the TOC before reading data.

`.nrpack` is v1 little-endian: a 4096-byte header, one 16-byte-aligned extent per entry in the data area, and a TOC aligned to 4096. Blocks use zstd (the C library on the packaging side, `ruzstd` at runtime). Encryption is XChaCha20-Poly1305 with a random 192-bit nonce. The publisher signs the header and `package.json` with Ed25519; the application holds only the public key. Reading follows eight steps and stops at the first failure; it never decrypts to a temporary directory and never decrypts per frame. Keys held by the client itself only stop casual unpacking.

`--baseline` reuses unchanged ciphertext from the previous release. `--cache` reuses records by plaintext hash. Incrementing `generation` rebuilds the whole pack, and the report says `key_rotation: true`. CI that discards the previous `out/` or cache gets a new nonce for every block.

## Wiring it into the application

The identity macro leaves `NANA-IDENTITY-V1` in the binary. Invalid fields fail compilation. With `packaged-resources` enabled, `ResourcePackOptions` is exported from `nana_ui`, and `nana_package::TrustPolicy` is available through `nana_ui::nana_package`. `NanaApplication` does not depend on this feature.

```rust
let identity = nana_ui_platform::application_identity!(
    id: "dev.nana.fixture",
    name: "Nana Fixture",
    version: env!("CARGO_PKG_VERSION"),
    vendor: "Nana",
);
let _session = NanaApplication::builder(identity)
    .resource_packs(
        ResourcePackOptions::new()
            .keys(my_key_provider)
            .trust(nana_package::TrustPolicy::RequirePublisher(PINNED_KEY))
            .loose_root(concat!(env!("CARGO_MANIFEST_DIR"), "/assets")),
    )
    .start();
```

Styles, `@font-face` and `url()` use `nana://res/ui/app.css`. During development, without a manifest, the same logical paths are read from `loose_root`. In installed and portable builds, a relative `url()` without an explicit base resolves against `runtime_resources`. Development and embedded hosts still resolve against the current working directory. Each pack is opened only when it is first looked up.

## Validation before release

```bash
nana-packager validate "out/app/Nana Fixture.app" --trust-key ed25519:… --run --tamper-suite \
    --env NANA_FIXTURE_CONTENT_KEY=…
```

It checks: the manifest signature (reported as not executed without `--trust-key`); signing adapter verification (not executed when the tool or credentials are absent); that the root holds only the executable, `runtime/` and registered exceptions; that file sizes and hashes match the manifest; the binary identity marker; that dependencies can be found by the loader; one read of every pack per the manifest; and that there are no updater files and no `NANA-UPDATER-V1`. `--run` launches the app in an unrelated directory with `NANA_PACKAGE_VALIDATE=1`; the app prints one line of JSON before opening a window and exits. `--tamper-suite` first runs an untouched control, then separately corrupts the manifest, the header, the TOC and the first real record, deletes a pack, and removes keys with `--tamper-drop-env`. When the application declares a packaged logo, it also reads it from the `early-splash` pack and checks the PNG header.

Platform signing rewrites the executable. For a signed package, add `--allow-resigned`; those hash changes are downgraded to warnings. `--run` can only run on a host of the same platform as the package.

The CLI also has `package`, `keygen content|publisher`, `inspect`, `delta OLD NEW` and `macos-app`. `macos-app` only produces an `.app` with no resource packs and no manifest.

Platform signing (codesign, notarization, Authenticode, GPG, apksigner, hap-sign-tool), installers (dmg, msi, NSIS, deb, AppImage, Flatpak), Linux FHS install paths, Android APK resource mounting and the update manifest contract are in sections 6–9 of the reference. The CLI also has `sign`, `update-manifest` and `android-assets`.

Not yet available: a built-in self-updater, real-window Early Splash / UiReady in package validation, StaticAppPlan fingerprints, automated install/uninstall round trips of installers, paged TOC.

Fields, read order and every check are in [Packaging and distribution](../../reference/packaging.md).

## Next steps

<div class="next-steps">
  <a class="next-step" href="/guide/scaling/performance">
    <p class="next-step-link">Performance</p>
    <p class="next-step-caption">No frames without changes, and what one pointer event costs.</p>
  </a>
  <a class="next-step" href="/guide/scaling/fetch">
    <p class="next-step-link">Fetch</p>
    <p class="next-step-caption">The host allowlist shared by url() and page requests.</p>
  </a>
  <a class="next-step" href="/guide/scaling/hot-reload">
    <p class="next-step-link">Hot reload</p>
    <p class="next-step-caption">The dev watcher stays in debug and never reaches dist.</p>
  </a>
</div>
