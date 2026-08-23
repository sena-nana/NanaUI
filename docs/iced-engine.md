# Iced migration asset (removed)

`engine/iced` was an in-tree, workspace-excluded migration snapshot while NanaUI
built Runtime, Scene, and `SceneWgpuPainter`. **The snapshot is gone.** It is
not a public API, application model, current renderer, or pending checkout.
Do not rejoin Iced as a `nana-*` compile dependency.

```text
engine/iced  (removed historical migration snapshot; not in the tree)
```

`scripts/check-engine-boundary.py` forbids Iced/GPUI crates from re-entering the
workspace and keeps `nana-ui-runtime` / `nana-ui-scene` backend-neutral. Scene
shaders retain MIT attribution to historical Iced; the snapshot's upstream
license lived at `engine/iced/LICENSE` while it was in-tree.

## Imported lineage

The snapshot (removed) was assembled on 2026-08-13 from:

- Nana fork: `sena-nana/iced` at
  `31bde4e4bac6de08f5ba581a8ea9c55ad0031e67`;
- shared upstream ancestor:
  `3c81aac2e1b48125efdf0c996fbbb9c72c06ae50`;
- upstream source: `iced-rs/iced`;
- retained Nana changes: WGPU 30, Cryoglyph WGPU 30 pin, fixed-size button
  centering, bounded MSAA resolve, editor scroll viewport preservation, and the
  WGPU image-batch initialization fix;
- folded-in NanaUI compatibility changes formerly carried by partial vendor
  crates: Android clipboard/key conversion guards and renderer-level affine /
  isolated-opacity groups used by the mixed Vue and native composition path;
- selected upstream updates from `ce69b89e6cf5bf3fa4e335d4753a8e1b2a30672a`
  through `65a6738df8eceae6771b83e95fe40fe3c805bf43`: the unified text editing
  core, editor alignment and operations, multiline input, paste handling,
  combo-box corrections, word/line deletion, click-drag selection, message
  tracking, doc-test repair, and toggler rendering repair.

The upstream commits `836a93a1d17ac46dee3781b0879fa461ee7b4b82`
and `2b275718d19a5cf306537e1d2417a1f5e9d94ef4` were deliberately not imported:
they describe their secure-input work as a draft and do not yet form a complete
feature contract.

Original commit IDs are recorded so the import can be audited without treating
the old fork as a build dependency or merge ancestry.

## Current inventory

Desktop product UI is `RuntimeDocument` / `UiScene` / `RuntimeProgram` /
`run_runtime` / `SceneWgpuPainter`. `nana-ui` and `nana-ui-vue` have no Iced
dependency. `scene-view` on `nana-ui-vue` is the Scene/Runtime adapter (the
`iced-view` alias is gone); it does not enable Iced or build an `iced::Element`
tree.

Issue [#12](https://github.com/sena-nana/NanaUI/issues/12) Iced/GPUI observation
uses archived `--from-report` fixtures. Live compile against `engine/iced` or
`engine/gpui-scenario-bench` is gone. Do not restore GPUI as a third product
paint path. WGPU stays the host GPU API.

The product path has left Iced. This was an archive decision, not a paint todo.

## Remaining platform debt

Desktop still pins the [iced-rs winit fork](https://github.com/iced-rs/winit)
(`rev = 05b8ff17a06562f0a10bb46e6eaacbe2a95cb5ed`) via workspace
`[patch.crates-io]` and `nana-ui`'s optional `winit` dep. That is leftover
window / event-loop platform debt, not an Iced renderer, widget tree, or
Android Iced dependency. Replacing it with upstream winit is still open.
