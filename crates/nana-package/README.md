# nana-package

English · [简体中文](README.zh-CN.md)

The read side of the Nana package format: package manifest, `.nrpack` resource packs, content keys, and publisher signatures. It ships with the application.

`PackReader` gives random access to a `.nrpack` and refuses on any inconsistency: it first verifies the header, publisher signature, and TOC hash, then verifies, decrypts, and decompresses block by block on demand. Content keys come from a `KeyProvider` implemented by the application, and `TrustPolicy` decides whether a publisher signature is required. A pack can be read from a file or from a byte range of one (`FileRange`, how an uncompressed Android APK asset is read). The `update` module defines signed update manifests and the rollback journal an application's own updater builds on. Because it is linked into shipped applications, it depends only on pure-Rust libraries; `scripts/check-package-boundary.py` checks this.

Applications usually do not use it directly; they mount resource packs through `nana-ui`'s `packaged-resources` feature and `ResourcePackOptions`. Building resource packs and generating keys and signatures happens at build time in [`nana-packager`](../nana-packager).

## Tests

```bash
cargo test -p nana-package
python3 scripts/check-package-boundary.py
```

See also [Packaging and distribution](../../docs/en/reference/packaging.md) and [Packaging guide](../../docs/en/guide/scaling/packaging.md).
