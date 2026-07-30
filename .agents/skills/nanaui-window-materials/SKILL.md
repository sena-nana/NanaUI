---
name: nanaui-window-materials
description: Maintain NanaUI's platform-owned native window material boundary. Use when changing nana-window, MaterialOutcome, appearance or fallback behavior, raw window handle access, macOS Vibrancy, Windows Mica or Acrylic, transparent windows, material cleanup, or target-platform evidence.
---

# NanaUI Window Materials

## Rules

- Read [`window-materials.md`](../../../docs/window-materials.md) before editing.
- Keep raw handles and platform APIs in `nana-window`; ordinary UI consumes only the public
  outcome.
- Clear an existing effect before reapplying it. Return the effect actually applied or an explicit
  fallback, never the requested effect after failure.
- Use an opaque readable fallback when native material is unavailable.
- Route Surface and renderer work to `$nanaui-gpu-integration`, and verification to
  `$nanaui-validation`.

## Validation

Check `nana-window` and affected targets. Require a real target-platform window before claiming
Vibrancy, Mica, Acrylic, resize, cleanup, or visual acceptance; cross-compilation is not runtime
evidence.
