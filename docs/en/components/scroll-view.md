# ScrollView

`ScrollView` is a scroll container. The authority for position is Runtime's `ScrollOffset`, and the authority for size is `ScrollMetrics`. At the end of any commit that changes layout boxes, children, or style, Runtime measures again and clamps the offset into range.

## Basic usage

There is no `<ScrollView>` in the control table. The node is clickable but not focusable.

:::api

```rust view
use nana_ui::runtime::view;
use nana_ui::runtime::{ScrollAxes, ScrollView};

view! {
    <Widget
        of={ScrollView::new(ScrollAxes::Vertical)
            .label("Log")
            .follow_end(true)}
    >
        <Column>
            <Text>"Latest line"</Text>
        </Column>
    </Widget>
}
```

```rust rust
use nana_ui::runtime::view::{column, text, widget};
use nana_ui::runtime::{ScrollAxes, ScrollView};

widget(
    ScrollView::new(ScrollAxes::Vertical)
        .label("Log")
        .follow_end(true),
)
.children(column().children(text("Latest line")))
```

:::

## Offset

When the origin is at the right or bottom edge, the offset on that axis is negative. Scrollbars do not keep a separate copy of the offset.

## Axes

The `axes` of `ScrollView::new(axes)` is `ScrollAxes::Horizontal`, `Vertical`, or `Both`. Overflow on those axes is written as `Scroll`.

## Smooth wheel

A regular mouse's line-based wheel moves with a 180 ms ease-out cubic transition by default; continued scrolling in the same direction accumulates the target, and scrolling in the opposite direction turns around immediately from the currently visible position. Horizontal axes, negative offsets, and nested containers share this behavior: once the target reaches a boundary, further wheel input passes to the outer container.

Pixel scrolling such as from a touchpad keeps the displacement the system provides and adds no transition. Dragging the scrollbar, `scroll_to` / `scroll_by`, scrolling focus into view, anchor restoration, and accessibility scrolling complete immediately and interrupt any wheel transition running in that container. With reduced motion, a running transition jumps to its target and later wheel input moves immediately.

`ScrollOffset` is always the current effective position; `ScrollChanged` and `UserScroll` report the current offset as it actually moves, never the target ahead of time. When the content range shrinks, both the position and the target are clamped into the new range. No scroll deadline remains after a transition ends.

## Scrollbar in the padding

`.gutter_theme(token)` extends the scroll area by this distance on the left and right and then insets the content by the same distance: the content stays on the container's content line, the scroll area and the scrollbar at its edge extend into the container's padding, the scrollbar does not cover content, and the padding also scrolls the list. When the padding cannot fit the theme's scrollbar lane, the lane narrows. The outer extension, the content inset, and the width available to the lane all resolve against the installed theme and change together when the theme changes. The container's horizontal padding must be at least this wide; a `scroll_foot_fade` laid on top uses `.clear_of_theme(token)` with the same value.

In a Card or a settings group card, use `.gutter_theme(ThemeToken::PanelPaddingX)`: this token is the card's own horizontal padding (`ThemeMetrics::panel_padding_x`, 16 by default, 10 under `COMPACT_METRICS`), so the scroll area and scrollbar reach exactly to the card edge under any metrics.

`.gutter(px)` is the pixel version and holds only under the theme it was written for: `gutter(16.0)` matches `gutter_theme(ThemeToken::PanelPaddingX)` under the default theme but extends 6 beyond the card padding under COMPACT. Whichever is called later replaces the earlier one.

## Room for heading tools

`.heading_room()` extends the scroll area upward by `heading_overhang()` and then insets the content by the same distance, so tools that overhang from the top heading row are not clipped by the scroll area. See [heading_row](heading-row.md).

## Sending the wheel to a scroll area

A wheel turn walks up from the node under the pointer to the first scroll area that can still move. A region outside any scroll area, such as a header held above a list, finds none on the way up, so turning the wheel over it moves nothing. `AppContext::set_wheel_target(node, Some(scroll))` hands the turns over `node` that nothing inside it takes to `scroll`, as if the pointer were over `scroll`, and on up from there instead of through `node`'s own ancestors:

```rust
cx.set_wheel_target(header.stable_id(), Some(list));
```

- Anything inside `node` that scrolls (a nested scroll area, a multiline editor) still takes the turn first.
- `None` lets `node` bubble to its own ancestors again. The redirect holds while both nodes exist, so it can be set up on a subtree before it is mounted; it is dropped when either node is despawned.
- The redirect belongs to the node: a turn over one of its descendants that has none of its own bubbles up to `node` and goes on to `scroll` from there.

When a node inside a scroll area has a `PointerDragged` handler (see dragging in [Controls](../reference/components.md)), the wheel still scrolls. A finger is offered to the scroll area first: if it can scroll that way it scrolls, and no drag starts.

## Edge fade

`.edge_fade(top, bottom)` fades the content at the top and bottom edges of the scrollport, with lengths in logical pixels. The scroll area and everything it holds are composited as one layer, and that layer's alpha falls linearly to 0 at the edge within each fade band: whatever is behind shows through, whether a transparent background or the gaps between floating cards on a stage, and no surface color is drawn. The scroll area's scrollbar does not fade; floating surfaces opened from inside the scroll area (`Popover`, `ActionMenu`, `HoverCard`, `Tooltip`, open `Select` dropdown menus, and similar root-painted or fixed overlays) and positioned boxes that escape the scroll area's clip chain do not fade with it either (while any nested scroll area inside such an overlay still keeps its own edge fade).

```rust
use nana_ui::runtime::{ScrollAxes, ScrollView};

ScrollView::new(ScrollAxes::Vertical).edge_fade(14.0, 64.0)
```

- A fade band extends only as far as there is content beyond that edge: the top band is `min(top, distance scrolled past the top)` and the bottom band is `min(bottom, distance remaining below)`. At the start there is no top band, at the end there is no bottom band, and while scrolling both bands grow and shrink rather than appearing or disappearing suddenly.
- When the content fits, both bands are 0 and the painter does no compositing for it.
- The fade is computed each frame from the current offset and has no animation of its own; reduced motion is the same.
- Write 0 to turn off that band; both 0 means no fade.
- It lives in the scroll area's style (`PaintStyle::scroll_edge_fade`, type `nana_ui_core::ScrollEdgeFade`), so write it after `.style(…)`; setting the field directly in the style works the same. It takes effect only on boxes that clip vertical overflow.

When what is behind is a known solid surface and you only want the fade over the bottom edge, [scroll_foot_fade](scroll-foot-fade.md) is cheaper: it does not need to isolate the scroll area into a layer.

## Following the end

`.follow_end(true)` keeps the vertical position on the newest content after layout publishes new scroll geometry; turn it off while you read older content.

## Scrollbars

Scrollbars are overlays: in every mode they take no layout space and change no boxes.

### Visibility

Without `.scrollbars`, the scroll area follows the theme's policy (`ScrollbarRecipe::visibility`, built-in `AutoHide`); once set, the value belongs to this scroll area and the theme policy does not override it. The type is `nana_ui_core::ScrollbarVisibility`:

- `AutoHide`: shown while the pointer is in the container (including in scroll areas nested inside it), right after the user scrolls (wheel, drag, touch, keyboard focus scrolling into view, accessibility scrolling), and while the lane is active or being dragged. After all of these end, it stays for the theme's `hide_delay` and then fades out. Program calls to `scroll_to` and `follow_end` do not make it appear.
- `Always`: drawn whenever that axis can scroll, with a persistent track.
- `Hidden`: no scrollbar is drawn. Wheel and `scroll_to` work as usual.

To make every scroll area that follows the policy persistent, an application uses `ThemeDefinition::with_scrollbar_visibility(ScrollbarVisibility::Always)`.

### Geometry

The lane is the hover and press area, `thickness` wide, located `track_inset` inward from the padding box edge (minus the border): the vertical bar on the right, the horizontal bar at the bottom, each inset by `track_inset` at both ends. When both axes have bars, each yields the corner so they do not overlap. The thumb is centered in the lane, normally `thumb_thickness` wide, and grows symmetrically to `thumb_active_thickness` while the lane is active or being dragged; the track is a capsule as wide as the active thumb.

If the container leaves padding of at least `track_inset + thickness` at the trailing end, the scrollbar sits entirely in that space and does not touch content. The framework does not reserve this space for you.

### Stacking and pointer

The scrollbar is drawn above all descendants of its scroll area regardless of their z-index; it is still inside its own ancestors' clipping and stacking context, so later siblings and viewport-fixed overlays can still cover it. A scroll area that can show scrollbars is therefore a stacking group.

Only a visible lane receives the pointer, including one that is fading in; when a scroll area itself sets `pointer-events: none` so its empty interior passes through, a visible scrollbar lane still receives hover and drag. When the pointer is over the lane, the hover target is the scroll area itself: rows beneath show no hover background or tooltip, the lane activates, and the thumb thickens. Pressing on the lane grabs the thumb; pressing on the empty track first centers the thumb on the press point.

### Motion and style

Fade-in, fade-out, and thickening play according to the theme's `ScrollbarRecipe::motion`; with reduced motion they go straight to the end state. No deadline remains while idle.

Sizes are in `ThemeMetrics::scrollbar`; colors, strokes, motion, and the visibility policy are in `ScrollbarRecipe` (supporting both ordinary surfaces and `media_track` / `media_track_hot` over media or transparent surfaces, plus optional track and thumb borders), installed with `ThemeDefinition::with_scrollbar`. A single node can change size, opt into `over_media: true` (e.g., over a stage), or override colors and borders with `PaintStyle::scrollbar` (`ScrollbarSkin`). CSS `::-webkit-scrollbar` and `::-webkit-scrollbar-thumb` write exactly this; a thumb width set by CSS does not thicken on hover. When the scroll area extends into the container's padding and the scrollbar sits in that padding, give the padding width with `ScrollbarSkin::room`: when the padding cannot fit the theme's lane plus the insets on both sides, the lane narrows until it fits and the thumb never exceeds the lane; this resolves against the installed theme only at paint time, and when the theme itself is narrower the theme wins.

`overflow: auto` and `overflow: scroll` share the same `ScrollOffset` and clipping and do not draw a second set of thumbs. Custom scrollbar chrome belongs only to `ScrollView`.

While dragging, only the grab anchor is kept in `ScrollbarDragState`; the offset stays in the world. `scrollbars_revealed` answers whether this scroll area requests showing its auto-hide scrollbars; whether they were actually drawn is `AppContext::scrollbars_visible`.

## Lending the scrollport

The body of `SidebarFrame` borrows `project_scrollport` so it does not lay a second scrollbar over its own visuals.

## Viewport

After layout, nodes listening to `ScrollViewportChanged` receive the new viewport width and height; virtual lists use it to narrow their window.

## Properties

`.label` is the accessible name. When content exceeds the viewport, that axis publishes the current offset and the reachable range; hidden scrollbars do not affect this range.

| Property | Type | Description |
| --- | --- | --- |
| `axes` | `ScrollAxes` | `Horizontal`, `Vertical`, or `Both`. `ScrollView::new(axes)`. Overflow on those axes is written as `Scroll` |
| `.label` | — | Accessible name |
| `.follow_end` | — | When `true`, keeps the vertical position on the newest content after layout publishes new scroll geometry; turn it off while reading older content |
| `.edge_fade(top, bottom)` | `f32`, `f32` | Length (logical pixels) of the content fade at the top and bottom edges of the scrollport, composited as one layer, so it works over a transparent background; each band extends only as far as there is content beyond the edge; the scrollbar does not fade. See [Edge fade](#edge-fade) |
| `.scrollbars` | `nana_ui_core::ScrollbarVisibility` | Unset follows the theme policy (built-in `AutoHide`); `AutoHide` appears on hover or scroll and fades out after a delay; `Always` is drawn with a track whenever scrolling is possible; `Hidden` draws nothing, and wheel and `scroll_to` work as usual |

## Events

| Event | Payload | Description |
| --- | --- | --- |
| `ScrollViewportChanged` | — | After layout, nodes listening to it receive the new viewport width and height; virtual lists use it to narrow their window |

## Slots

| Slot | Description |
| --- | --- |
| Children | The scrolled content. `.children` on the function side |

## See also

[Overview](index.md) · [Controls](../reference/components.md)
