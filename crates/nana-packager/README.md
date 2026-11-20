# nana-packager

English · [简体中文](README.zh-CN.md)

The build-time packaging tool for Nana applications, providing the `nana-packager` command and a library of the same name.

It reads `nana-package.toml` and an already built executable, packs resources into `.nrpack`, assembles the directory layout for the target platform, writes the package manifest and publisher signature, and can also generate Steam depot scripts. Given the previous release's output, unchanged entries keep their original offsets, so SteamPipe delta patches are smaller. `validate` does the final check of the delivery directory; checks that cannot run are reported as `not-executed` and never count as passing.

It does not invoke cargo and does not go into the application binary; reading resource packs at run time is [`nana-package`](../nana-package)'s job. Platform signing (codesign, notarization, Authenticode, GPG, apksigner, hap-sign-tool) wraps each platform's own tool, and installer backends build dmg, msi, NSIS, deb, AppImage and Flatpak; whatever cannot run on the current host is reported `not-executed`, never as done.

## Example

```bash
nana-packager keygen content --out content.key
nana-packager package --config nana-package.toml --exe target/dist/my-app --out out
nana-packager validate out/app/<app-root> --trust-key ed25519:HEX --run
```

`nana-packager inspect` and `nana-packager delta` inspect resource packs and estimate the difference between two releases; `sign` signs an app or artifacts built elsewhere, `update-manifest` writes a signed update manifest, and `android-assets` lays packs out for an APK. For the full arguments, see `USAGE` in `src/main.rs`.

## Tests

```bash
cargo test -p nana-packager
```

The end-to-end packaging and tamper-resistance flow is the `packaging` job in [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml).

See also [Packaging and distribution](../../docs/en/reference/packaging.md) and [Packaging guide](../../docs/en/guide/scaling/packaging.md).
