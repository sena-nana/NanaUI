# TimelineEditor

`TimelineEditor` is an editable time base for animation, audio or data editing. One node draws the time ruler and playhead, a collapsible track tree on the left, keys and clips on the tracks, and the Bézier curves and tangent handles in the curve view. The content belongs to the application: the application fills `tracks`, editing is reported through events, and the application writes the updated data back. It has no template tag; put it in the view with `widget(TimelineEditor::new(tracks))`.

> `Timeline` in animation (the action description) is something else. This component is `TimelineEditor`, and its data and event types start with `Timeline`.

This type is behind the Cargo feature `timeline` (included in `components`).

## Basic usage

```rust
use nana_ui::runtime::view::widget;
use nana_ui::runtime::{
    TimelineEditor, TimelineKey, TimelineKeysMoved, TimelinePlayheadChanged, TimelineTrack,
};

widget(
    TimelineEditor::new([
        TimelineTrack::new("head", "Head", []).expanded(true),
        TimelineTrack::new("x", "Angle X", [TimelineKey::new(0.0, 0.0), TimelineKey::new(1.0, 30.0)])
            .depth(1),
        TimelineTrack::new("blink", "Blink", [TimelineKey::new(0.8, 1.0)]),
    ])
    .fps(30.0)
    .playhead(0.5),
)
.on(|e: &TimelinePlayheadChanged| { /* e.time, e.committed */ })
.on(|e: &TimelineKeysMoved| { /* on release (e.committed), move e.keys by e.dt seconds and e.dv */ })
```

## Ruler and playhead

- Tick units are frames (`TimelineUnit::Frames`, following `.fps()`) or seconds. Numbered ticks are at least 64 px apart, taking the first of 1 / 2 / 5 × 10ⁿ that is wide enough; minor ticks in between take a tenth, a fifth or half of the step — whichever is the first no narrower than 6 px (frame units use whole frames only). Ticks densify or thin out automatically as you zoom.
- Press on the ruler and drag to move the playhead; during the drag it emits `TimelinePlayheadChanged { committed: false }`, and once with `committed: true` on release. With `.snap(true)` on (the default) it lands on whole frames.

## Tracks

- `TimelineTrack::depth(n)` sets the nesting depth: a track belongs to the nearest preceding track with a smaller depth. `.expanded(bool)` makes it a branch.
- Click the triangle to the left of a branch to collapse or expand it; collapsed child tracks are hidden, and it emits `TimelineTrackToggled { track, expanded }`.
- Tracks and the key area scroll vertically together (`view.scroll`).

## Keys

- Click a key to select it; Shift-click adds it to or removes it from the selection; drag a box over empty space to select the keys inside (Shift adds). Selection changes emit `TimelineSelectionChanged { keys }`; the selection is kept in `selection` (sorted by `TimelineKeyRef { track, key }`).
- Dragging a selected key moves the whole selection. With snapping on, the first selected key lands on a whole frame. During the drag the component draws the moved state itself and emits `TimelineKeysMoved { committed: false }`; on release it emits `committed: true` once, and the application updates the data then.
- Keys such as delete, select all and fit view belong to the application's shortcuts: the application reads `selection` to delete keys, and calls `cx.fit_timeline(entity)` to fit the view.

## Clips

A clip is a span of time on a track: `TimelineClip::new(id, start, length, label)`, placed on a track with `TimelineTrack::clips([...])`. `id` is unique across the whole timeline, and events refer to clips by `id`. Clips on the same track do not overlap.

```rust
use nana_ui::runtime::{
    TimelineClip, TimelineClipSelectionChanged, TimelineClipTone, TimelineClipTrimmed,
    TimelineClipsDeleteRequested, TimelineClipsMoved,
};

widget(
    TimelineEditor::new([
        TimelineTrack::new("motion", "Motion", []).clips([
            TimelineClip::new("wave", 0.0, 1.2, "Wave"),
            TimelineClip::new("nod", 1.5, 0.5, "Nod").resizable(false),
        ]),
        TimelineTrack::new("sound", "Sound", [])
            .clips([TimelineClip::new("chime", 0.4, 0.8, "Chime").tone(TimelineClipTone::Warning)]),
    ])
    .selected_clips(selected.clone()),
)
.on(|e: &TimelineClipSelectionChanged| { /* store e.clips, pass them back through .selected_clips next time */ })
.on(|e: &TimelineClipsMoved| { /* when e.committed, move e.clips by e.dt seconds; e.track is the track they moved to */ })
.on(|e: &TimelineClipTrimmed| { /* when e.committed, set e.clip to e.start / e.length */ })
.on(|e: &TimelineClipsDeleteRequested| { /* delete e.clips */ })
```

- Clips are drawn only in the dope sheet view: a rounded rectangle fills the track row (2 px inset top and bottom), the fill is a light tint of the tone, the border darkens when selected, and the name shows on one line inside the clip, elided when it does not fit and hidden when too narrow. Keys are drawn above clips, and clicking a key prefers the key. Collapsed tracks draw no clips. The curve view ignores clips.
- Tones (`TimelineClipTone`): `Accent` uses the accent color; `Info` / `Success` / `Warning` / `Neutral` follow the theme's status colors.
- Selection: click a clip to select it, Shift-click to add or remove; clicking an already selected clip keeps the whole group selected so they drag together; clicking empty space in the key area clears the clip selection (Shift box-selection keeps it). Changes emit `TimelineClipSelectionChanged { clips }`. The application owns the selection and passes it back with `.selected_clips(ids)`.
- Move: dragging a clip moves all selected clips and emits `TimelineClipsMoved { clips, dt, track, committed }`. When exactly one clip is selected, dragging it onto another leaf track's row changes tracks, and `track` is the target track's index in `tracks`; it is `None` when the track did not change. If the target track cannot hold it, the clip stays on its original track.
- Trim: for `resizable` clips, when the pointer is within 6 px of an edge (at most a third of the clip's width for narrow clips) it counts as the edge; hovering shows trim handles and the cursor becomes a horizontal resize. Dragging the start edge changes `start` while keeping the end time; dragging the end edge changes `length`; it emits `TimelineClipTrimmed { clip, start, length, committed }`. Trimming never makes a clip shorter than `min_clip_length` (0.05 seconds by default) or start it before 0. Trimming selects this clip alone (unchanged if it was already in the selection).
- Snapping (`.snap(true)`): a moving or trimming edge within 6 px snaps to other clips' edges and to the playhead; when nothing is in range, the trimming edge — or the pressed clip's start while moving — lands on a whole frame.
- No overlap: a move or trim that meets a stationary clip on the same track (the target track when changing tracks) stops at its edge; moves never go earlier than 0.
- During the drag the component draws the moved or trimmed state itself and emits `committed: false`; on release it emits `committed: true` once, and only if something changed, and the application updates the data then. Esc does not cancel a drag.
- When the timeline has focus and clips are selected, Delete or Backspace emits `TimelineClipsDeleteRequested { clips }`, and the application deletes them and updates the selection. The application's own shortcuts receive the key first and can intercept it.
- The screen-reader value includes the total and selected clip counts when clips exist.

## Curve view

- `.mode(TimelineMode::Curves)`: the keys of each visible track are connected by cubic Bézier curves; the control points are the key's out tangent and the next key's in tangent (`TimelineKey::out_handle` / `in_handle`, `[dt, dv]` relative to the key). Curves stay horizontal beyond the first and last keys.
- A selected key shows two tangent handles. Dragging a handle emits `TimelineHandleMoved { key, out, handle, broken }`. Handles stay on the same side of the key (the out tangent points back, the in tangent points forward). When `broken` is false, the handle on the other side turns to the opposite direction and keeps its length; the component draws it that way during the drag, and the application commits the same change. If the key itself is `broken`, or Alt is held during the drag, the two handles move independently.
- In the curve view, dragging a key changes time and value together (`dv`).
- `fit()` / `cx.fit_timeline()` brings the selected keys (all keys when nothing is selected) into view: time fills the key area, values and tangent handles fill the height.

## View

- The wheel zooms around the time under the pointer; Shift + wheel scrolls tracks; Ctrl + wheel pans time. Middle-button drag pans (and pans values too in the curve view).
- View changes emit `TimelineViewChanged { view }`. The application can store `view` and restore it with `.view(...)` next time. Re-rendering does not overwrite a view the user has zoomed or panned.

## Properties

| Property | Type | Description |
| --- | --- | --- |
| `TimelineEditor::new(tracks)` | `TimelineTrack` | Tracks and keys |
| `.fps(f64)` | `f64` | Frames per second, 30 by default |
| `.unit(TimelineUnit)` | `Frames` / `Seconds` | Ruler unit |
| `.mode(TimelineMode)` | `DopeSheet` / `Curves` | Dope sheet view or curve view |
| `.view(TimelineView)` | `start`, `pixels_per_second`, `value_min`, `value_max`, `scroll` | Visible window |
| `.playhead(f64)` | seconds | Playhead |
| `.snap(bool)` | `bool` | Snap drags to whole frames (clips also snap to other clips' edges and the playhead), on by default |
| `.selected_clips(ids)` | `Arc<str>` | Selected clips |
| `.min_clip_length(seconds)` | `f64` | Shortest clip length after trimming, 0.05 by default |
| `.clips_editable(bool)` | `bool` | On by default. When off, clips can still be selected and the playhead still drags, but clips cannot be moved or trimmed and no delete request is sent; for read-only editors |
| `.header_width(px)` | `f32` | Track header column width, 140 by default |
| `.label(text)` | `Arc<str>` | Screen-reader name |

## Events

| Event | Payload | Description |
| --- | --- | --- |
| `TimelinePlayheadChanged` | `time`, `committed` | Playhead dragging / released |
| `TimelineSelectionChanged` | `keys` | Selected keys changed |
| `TimelineKeysMoved` | `keys`, `dt`, `dv`, `committed` | Keys dragging / released |
| `TimelineHandleMoved` | `key`, `out`, `handle`, `broken`, `committed` | Tangent handle dragging / released |
| `TimelineClipSelectionChanged` | `clips` | Selected clips changed |
| `TimelineClipsMoved` | `clips`, `dt`, `track`, `committed` | Clips dragging / released |
| `TimelineClipTrimmed` | `clip`, `start`, `length`, `committed` | Clip edge dragging / released |
| `TimelineClipsDeleteRequested` | `clips` | Delete / Backspace pressed while focused |
| `TimelineTrackToggled` | `track`, `expanded` | Branch collapsed or expanded |
| `TimelineViewChanged` | `view` | Zoom, pan or scroll |

## See also

[PropertyGrid](https://github.com/sena-nana/NanaUI/blob/main/docs/components/property-grid.md) · [Overview](https://github.com/sena-nana/NanaUI/blob/main/docs/components/index.md)
