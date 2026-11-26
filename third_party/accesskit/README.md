# NanaUI's `accesskit_windows` fork

[中文](README.zh-CN.md)

`third_party/accesskit/windows` holds NanaUI's patched `accesskit_windows` 0.35.1 crate, published to crates.io as `nana-accesskit-windows` (`0.35.1-nana.1`), while `accesskit`, `accesskit_macos`, `accesskit_unix`, and `accesskit_android` resolve directly from crates.io.

## Changes from upstream `accesskit_windows` 0.35.1

### Upstream PR #816
- Localized `Switch` control type (`UIA_LocalizedControlTypePropertyId` defaults to `"switch"` when no `role_description` is set) and `ScrollPattern` (`IScrollProvider`) support in `src/node.rs`.

### Stability and no-panic hardening
- `src/adapter.rs`: `QueuedEvents::raise()` ignores transient/disconnected/teardown COM errors from `UiaRaiseAutomationEvent` and `UiaRaiseAutomationPropertyChangedEvent` instead of panicking (preventing aborts under `panic = "abort"` release builds).
- `src/util.rs`, `src/context.rs`, `src/node.rs`, `src/text.rs`: `client_top_left()` returns `Result<Point>` mapping Win32 `ClientToScreen` failure to `UIA_E_ELEMENTNOTAVAILABLE` instead of panicking, propagated via `?` in `BoundingRectangle`, `ElementProviderFromPoint`, `RangeFromPoint`, and `GetBoundingRectangles`; also removes fallible `.unwrap()` calls on `IValueProvider::SetValue` UTF-16 decoding, `UiaGetReservedNotSupportedValue`, `UiaGetReservedMixedAttributeValue`, and `ITextRangeProvider` endpoint/node lookups.
- `src/subclass.rs`: replaces raw `SetWindowLongPtrW(GWLP_WNDPROC)` + `SetPropW`/`GetPropW`/`RemovePropW` with ComCtl32 `SetWindowSubclass` / `GetWindowSubclass` / `RemoveWindowSubclass` / `DefSubclassProc` (via `windows` feature `Win32_UI_Shell`) and `Rc`-counted reentrancy-safe lifetime management, ensuring safe stacking with `nana-window` subclassing and safe UIA provider disconnection (`UiaDisconnectProvider` + `UiaReturnRawElementProvider(hwnd, 0, 0, None)`) on `WM_NCDESTROY` / uninstall.
- `src/adapter.rs`, `src/context.rs`: clears stale `platform_nodes` on `Placeholder -> Active` tree replacement.

### Windows UIA hot-path performance
- `src/adapter.rs`: checks `UiaClientsAreListening()` in `update_if_active` and `update_window_focus_state`. When no UIA clients are listening for events, the tree is still updated and removed node IDs are still evicted from `context.platform_nodes`, while skipping property diffing, UTF-16/BSTR/VARIANT work, `PlatformNode` creation, and `QueuedEvent` generation.
- `src/adapter.rs`: replaces `self.queue.insert(0, ...)` for `UIA_Text_TextChangedEventId` with a dedicated `text_changed_events` buffer prepended once at the end (avoiding $O(N^2)$ shifting) and gates `node_updated` live-region name recomputation on `live != Live::Off`.
- `src/node.rs`, `src/util.rs`: compares `FullNodeId` iterators before materializing `PlatformNode`s / `Vec<IUnknown>` for `UIA_ControllerForPropertyId` (also fixing omission of newly added trailing controls); borrows `description` directly as `StrWrapper`; guards `UIA_NamePropertyId` and `UIA_AriaPropertiesPropertyId` diffing on underlying field changes; formats integer `AriaProperties` without intermediate `String` allocation.

### Additional UIA semantic mappings
- `aria-invalid`: exposes `UIA_IsDataValidForFormPropertyId` (with property-change events) and emits `invalid=true|grammar|spelling` in `UIA_AriaPropertiesPropertyId`.
- Landmarks: exposes `UIA_LandmarkTypePropertyId` and `UIA_LocalizedLandmarkTypePropertyId` for `Main`, `Navigation`, `Search`, `Form`, `Banner`/`Header`, `ContentInfo`/`Footer`, `Complementary`, and `Region` per W3C Core-AAM / Chromium UIA mapping.
- `aria-level`: fixes off-by-one bug in `aria_properties()` so `level` is 1-based, matching `UIA_LevelPropertyId` and WAI-ARIA.
- Menu & tooltip lifecycle events: emits `UIA_MenuOpenedEventId` / `UIA_MenuClosedEventId` for `Role::Menu` and `Role::MenuListPopup`, and `UIA_ToolTipOpenedEventId` / `UIA_ToolTipClosedEventId` for `Role::Tooltip` on add, visibility/role transitions, and remove.

Once these fixes are upstreamed and released on crates.io, `nana-ui` can switch `accesskit_windows` back to the upstream crate and remove this vendored copy.
