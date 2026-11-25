# Mobile Host

The mobile host is the layer shared by the two **experimental** hosts, iOS and Android (`platform/mobile-core`, crate `nana-mobile-host`). For now it exists only to answer one question: can NanaUI plus hybrid native reach the performance and feel of a native app? It is not a product delivery target; do not promise mobile support externally on the basis of it.

Platform details for the two hosts are in [iOS](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/ios.md) and [Android](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/android.md). The iOS host is an rlib, and an app puts its own view into it through `IosApp` (see [iOS](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/ios.md#在宿主上做一个-app)).

## Layers

```text
Native shell (UIKit view / GameActivity)
  display link, touch and keys, text proxy, camera layer, lifecycle
    ↓ lowered to platform-neutral calls
nana-mobile-host
  MobileRuntime: one RuntimeDocument, input routing, IME lifecycle, safe area, FrameDemand
  MobileRenderer: one Device/Queue, SceneWgpuPainter, HostTextureRegistry
    ↓
nana-ui (gpu only, not hosted)
```

Mobile does not use `run_runtime`, `RuntimeProgram`, or `EmbeddedRuntime`; those are all in `hosted` and depend on winit. The shell owns the window and the event loop; `MobileRuntime` does only the platform-neutral part.

`MobileRuntime` installs NanaUI's built-in light and dark themes by default, with metrics switched to `TOUCH_METRICS`. When an application has its own design system, call `set_theme(MobileTheme { light, dark, page })` when mounting the view: both definitions are compiled as given (if compilation fails the call is rejected and the previous theme stays), so touch definitions should start from `TOUCH_METRICS` and change only what the application's own design decides. `page` is the background color role the root node paints inside and outside the safe area; the default is `Background`, and applications whose whole screen is one card with groups as lighter blocks on it use `Surface`. `background_rgba()` also reports this role's color.

## Hybrid native rules

Native participates in only two ways:

1. **Providing behavior**: NanaUI paints the pixels and the platform supplies the behavior. For example, text input goes through the platform text system (`UITextInput` plus `UITextInteraction` on iOS, GameTextInput on Android), so the platform handles the keyboard, composition, candidates, magnifier, selection handles, and edit menu. Accessibility goes through the platform's native tree.
2. **Whole native layers**: placed above or below the NanaUI surface, outside layout. For example, the camera preview layer sits under the transparent surface and the page leaves the corresponding area transparent; system panels (permissions, share, QR scanning) are presented modally on top.

For a native layer underneath to actually show through, two things are needed:

- **The surface must not be opaque.** A transparent surface (`SurfaceOptions { transparent: true }`) picks `PreMultiplied`, `PostMultiplied`, then `Inherit`. Metal offers only `Opaque` and `PostMultiplied`, and only the latter clears `CAMetalLayer.opaque`; if it falls back to `Opaque`, every transparent pixel shows as black.
- **The root node does not paint the page background.** After `MobileRuntime::set_background_passthrough(true)`, the root node paints no background, and the shell paints the same theme background with `background_rgba()` below the native layer, so the page looks the same as when opaque.

A `GpuTextureView` in the page can be shown only when its host texture is already registered: if any host texture in a frame cannot be sampled, `paint` fails for the whole frame. The measurement page's preview area is therefore a fixed-size transparent container, and the texture view inside is shown only after the CPU preview slot is registered (`Probe::sync_preview`, which both hosts call before preparing each frame).

**Do not embed native controls in NanaUI layout or scrolling content.** Native layers move on the system compositor thread while NanaUI moves on its own frames, so the two misalign during scrolling; clipping and z-order would also require splitting layers for compositing.

The text proxy needs the editor's geometry, so the runtime provides `AppContext::focused_text_offset_at` (which byte offset a point lands on) and `focused_text_caret_rect` (the caret rectangle at an offset). `MobileRuntime::text_offset_at`, `text_caret_rect`, and `text_input_bounds` forward these queries in logical pixels; `set_text_selection` writes a selection changed by the platform back to the editor.

## Frames: on demand

`MobileRuntime::frame_demand()` tells the shell when the display link next needs to fire. There are three answers:

| Value | Meaning | What the shell does |
| --- | --- | --- |
| `Now` | There are unpresented changes: input, viewport, appearance, pending reactive writes, compositor animation, host texture updates | Produce a frame on the next vsync |
| `At(t)` | The deadline of the next animation, caret blink, or timer | Pause the display link and resume at `t` |
| `Idle` | Nothing to do | Pause the display link |

Each time the display link fires, call `prepare_frame(now)`. It polls ready view tasks, advances due animations and compositor clocks, then runs layout, and returns whether this frame has new content; when it returns `false` the shell does not paint. When a view task becomes ready on another thread, it calls the wake function installed with `set_task_wake`, and the shell requests a frame on the main thread.

**A settled document produces no frames**, and there are no heartbeat frames. `a_settled_document_asks_for_no_frames` pins this down.

## Lifecycle

`MobileRenderer` holds the Device/Queue, painter, and host texture table from the first surface until the app exits. On entering the background the shell only calls `detach()` to drop the surface; on returning to the foreground it attaches a new surface with `attach()` and then calls `request_present()`. The document and its state are kept throughout.

## Geometry

`MobileViewport` includes the physical size, the scale (UIKit's `contentScaleFactor`, Android's `densityDpi / 160`), and the four-edge safe area (`SafeArea`, in physical pixels). The `MobileRuntime` root node fills the whole surface and pads the application view by the safe area. In landscape, the left and right edges where the notch and navigation bar sit are avoided the same way.

The mobile host installs the touch density `TOUCH_METRICS` by default (see [theme.md](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/theme.md)).

## Input

`push_touch` routes each active touch finger with its distinct `PointerId` concurrently through `nana-ui-input` (the first active finger sets `is_primary = true` and subsequent concurrent fingers set `is_primary = false`, enabling multi-touch pinch-zoom and two-finger pan; duplicate `Down` or unknown-pointer samples return `false`). Touch-drag scrolling, flinging, and bounce are handled by the runtime (see [runtime-scene.md](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/runtime-scene.md#触摸滚动与-scrollphysics)). `set_scroll_physics` selects the physics profile: the iOS host uses `Ios`, and the Android host uses `Android`. Flings and bounces run on animation deadlines, so `frame_demand` returns `At` while they are in progress and returns to `Idle` once they stop. `push_composition` receives composition events from the platform text system. When the primary touch taps outside the editor, any unfinished composition is committed first, then focus moves.

## Measurement page

`platform/mobile-probe` (`nana-mobile-probe`) is the measurement page both hosts mount: a search box, a switch and a slider, a preview area, and a vertically scrolling grid of 200 buttons. The preview area has no background, so the native camera layer shows through from below; after turning on "CPU preview," the host instead uploads frames to `PREVIEW_SLOT`, for comparison with the native preview layer.

## Acceptance

- `cargo test -p nana-mobile-host -p nana-mobile-probe`: frame demand, safe area, touch density, composition, selection write-back, and the inverse relationship between the caret rectangle and point hits; in addition, under both physics profiles the grid must drag, fling, and return to Idle after stopping (`a_swipe_scrolls_the_grid_flings_and_settles_idle`). `every_preview_path_paints` actually paints both the native preview and CPU preview paths on the local GPU (skipped without a GPU); whole-frame failures such as a missing host texture can only be caught this way.
- `scripts/check-ios-arm64.sh` and `scripts/check-android-arm64.sh`: cross-compilation.
- Performance and feel count only on real-device data; simulator numbers do not count as evidence. Targets and the comparison method are in [iOS](https://github.com/sena-nana/NanaUI/blob/main/docs/reference/ios.md#验收).
