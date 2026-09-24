# Consumer upgrade notes (2026-09-24)

This round lands Issue #182: nana-text's `EditSession` is now the only storage for an editor's committed text, selections and IME composition. For the division of responsibilities, see "Editor storage: EditSession (#182)" in [Text engine](text-engine.md).

## API changes

### Text values are shared (`TextValue`)

- New type `nana_ui_runtime::TextValue`, an alias of `nana_text::SharedText`.
  - A text buffer behind an `Arc`, plus an optional `TextStamp`. Cloning it only bumps a reference count.
  - It dereferences to `&str`, and compares directly with `&str` / `String`.
  - Build one with `.into()` from `String` / `&str` / `Arc<str>`. Wrapping an `Arc<str>` copies nothing.
  - To get a `String`, call `.to_string()` / `.into_string()`.
- These fields change from `String` / `Arc<str>` to `TextValue`:

  | Field | Was |
  | --- | --- |
  | `TextContent.value` | `String` |
  | `TextInputState.value` | `String` |
  | `TextInputPresentation.display_value` | `String` |
  | `ComponentTextRegion.content` | `Arc<str>` |
  | `AccessibilityNode.value` | `Option<Arc<str>>` |
  | `TextChanged.value` | `String` |
  | the scene's `ScenePrimitiveKind::Text.content` | `String` |

  - Construction sites add `.into()`: `TextContent { value: label.into() }`.
  - Where a `String` is needed, change `.value.clone()` to `.value.to_string()`.
  - `TextInputState::new` / `replace_value` now take `impl Into<TextValue>`.
- `nana_text::SharedText::replace_range` is public. Each call draws a fresh stamp, and it copies only when the buffer is shared.

### Reading editor state from the world

- `UiWorld::text_input(id)` returns `Option<TextInputView<'_>>`.
  - It borrows the session and copies nothing.
  - Fields: `value: &str`, `selection`, `additional_selections: &[TextSelection]`.
  - Methods: `selections()`, `value_shared()`, `to_state()`, `session()`.
  - It compares directly with `TextInputState`.
  - `.cloned()` becomes `.map(|input| input.to_state())`.
  - Code that needs a `String` changes `.value.clone()` to `.value.to_owned()`.
- `UiWorld::ime(id)` returns `Option<ImeView<'_>>`, with `text: &str` and `selection`.
  - Use `to_composition()` to get an `ImeComposition`, and `ImeComposition::view()` for the reverse.
  - They compare directly with each other.
- `UiWorld::focused_text_input` / `AppContext::focused_text_input` now return `(id, TextInputView<'_>)`.
- `TextSelection` is now an alias of `nana_text::EditSelection`. Fields, constructors and `ordered` / `is_valid_for` are unchanged; `EditSelection` also adds `range` / `is_collapsed`.
- `ExtractedNode.ime` / `ExtractedNode.text_input` are removed and replaced by `editable: bool`. The scene only ever checked whether they were present.
- `TextEditOrigin` gains `Cut`.

### nana-text

- `EditSession` supports multiple selections:
  - `with_selections`, `selections`, `additional_selections`, `set_selections`, `set_primary_selection`, `add_selections`, `collapse_selections`;
  - multi-cursor `insert` / `delete` / `move_caret` / `cut`;
  - `selected_texts`.
- Batch edits: `splice(edits, primary, additional)` and `assign(&SharedText, selections)`, a minimal-diff replacement.
- `EditorGeometry::sync_stamped`, `editable::collapse_edge`, `editable::normalize_selections` / `remap_offset` / `remap_selection`, and `editable::diff::changed_range`.
- `TextWorkCounters` gains `editor_text_bytes_compared`.

## Behavior changes

- **Left/Right with a selection:** without Shift, Left/Right collapse the selection onto its edge on that side of the screen instead of stepping one grapheme from the focus. In right-to-left text the logical end is on the left. Fields that cannot be probed (masked, empty) collapse onto the logical edge.
- **Empty preedit:** the empty preedit a platform reports between keystrokes is no longer treated as a composition.
  - Line numbers, extra cursors, inlays, completions and hover all stay visible.
  - It still blocks form submit until the commit or cancel arrives, as before.
- **Selection sets:** the world and components normalize selections the same way. Overlapping or touching selections merge, and ends inside a grapheme cluster snap back to the cluster boundary. Before, an extra cursor could sit inside an emoji ZWJ sequence.
- **Undo:**
  - A cut is its own step, and typing right after it no longer merges into it.
  - Inserting a snippet is one step, and the single-line length limit now applies to it.
- **NumberInput IME:** committed and surrounding-deleted text goes through the component. Before, the next keystroke overwrote it.
- **Android:** the IME buffer is the editor's displayed text. The preedit replaces the selection it stands for; before, it was inserted next to the focus.
- **Folding:** pressing Right across a collapsed fold leaves a caret, not a selection covering the hidden lines.
