---
name: nanaui-window-materials
description: Maintain NanaUI's platform-owned native window boundary. Use when changing nana-window, title-bar chrome, raw handles, native materials, transparency, resize behavior, or platform fallbacks.
---

# NanaUI Window and Platform Boundary

Read [`docs/window.md`](../../../docs/window.md) and the platform contracts in [`docs/architecture.md`](../../../docs/architecture.md).

- Keep raw window handles and platform APIs inside `nana-window` and `nana-ui-platform`. Ordinary controls consume public outcomes and commands only.
- Clear an existing effect before reapplying it. Return the effect actually applied or an explicit fallback; never report a requested effect after failure.
- Use a readable opaque fallback when a native material is unavailable. Do not silently substitute a different material family.
- Keep title-bar drag, client-area chrome, native controls, scaling, input, IME, clipboard, display, and fullscreen ownership in the platform boundary. Framework UI supplies layout slots and public state.
- Route Scene/GPU changes to `$nanaui-gpu-integration` and evidence to `$nanaui-validation`.

Require a real target-platform window before claiming native material, resize, cleanup, or visual acceptance; compilation for another target is not runtime evidence.
