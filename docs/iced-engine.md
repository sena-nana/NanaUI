# Iced compatibility engine

`engine/iced` is NanaUI's in-tree migration substrate. It supplies mature text,
layout, accessibility, platform, widget, and WGPU behavior while NanaUI evolves
its own runtime, scene, platform, and rendering contracts. It is not NanaUI's
public API or its long-term application programming model.

The dependency direction is intentionally one-way:

```text
nana-* crates -> compatibility adapters -> engine/iced
```

Code under `engine/iced` must not depend on NanaUI crates, components, Vue
semantics, Live2D concepts, or application behavior. `scripts/check-engine-boundary.py`
enforces this manifest boundary in CI.

## Imported lineage

The initial in-tree snapshot was assembled on 2026-08-13 from:

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
  through `65a6738df8eceae6771b83e95fe40fe3c805bf43`: the unified text editing core,
  editor alignment and operations, multiline input, paste handling, combo-box
  corrections, word/line deletion, click-drag selection, message tracking,
  doc-test repair, and toggler rendering repair.

The upstream commits `836a93a1d17ac46dee3781b0879fa461ee7b4b82`
and `2b275718d19a5cf306537e1d2417a1f5e9d94ef4` were deliberately not imported:
they describe their secure-input work as a draft and do not yet form a complete
feature contract.

The upstream MIT license remains at `engine/iced/LICENSE`. Original commit IDs
are recorded here, and the Nana fork head is retained as merge ancestry, so the
import can be audited without treating the old fork repository as a build
dependency.

## Upstream sync

1. Clone `sena-nana/iced` into a scratch directory and add
   `https://github.com/iced-rs/iced.git` as `upstream`.
2. Compute the previous upstream base and review new upstream commits in order.
   Classify platform, text, accessibility, WGPU compatibility, bug-fix, and
   security changes separately from runtime/tree/renderer redesign.
3. Apply candidates to the scratch fork, preserving Nana's WGPU 30 and hosted
   rendering behavior. Do not import draft APIs or upstream version migrations
   merely because they are newer.
4. Run the affected Iced crate tests in the scratch checkout. Then replace the
   in-tree snapshot mechanically, keeping this document's lineage current.
5. Run the NanaUI dependency boundary check, locked dependency checks, and the
   smallest affected NanaUI/Gallery/hosted validation before delivery.

Security and platform correctness changes take priority. Deep changes to the
widget tree, application/message runtime, recursive event dispatch, extraction,
or renderer orchestration are migration inputs: they must be evaluated against
NanaUI's own `UiWorld`, context/action, scene, and event-driven idle goals.

## Current dependency inventory

NanaUI still uses the compatibility engine for:

- widget construction and the retained `Tree` used by the current renderer;
- text shaping/editing, layout primitives, focus/IME projection, and AccessKit;
- event conversion, subscriptions/tasks, and current message delivery;
- Winit window integration and the WGPU compatibility renderer;
- component Gallery and hosted renderer compatibility.

These are migration facts, not stable public contracts. New Nana-native APIs
must not expose Iced types merely because the compatibility path still consumes
them internally.

## Exit metrics

An Iced subsystem can leave NanaUI's core path only when its Nana-owned
replacement has equivalent behavior for retained identity, hierarchy, input,
focus/IME, accessibility, layout/text, rendering, idle wakeups, and affected
component/consumer fixtures. The replacement must also avoid a second retained
world and must meet the established performance baselines. WGPU may remain a
compatibility or reference backend when it is still the best validated option.
