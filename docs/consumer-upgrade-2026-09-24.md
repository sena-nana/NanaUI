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
- `nana_text::SharedText::replace_range` is public. Each call draws a fresh stamp. It edits in place only when this value alone holds an owned buffer; a shared buffer, or one built from an `Arc<str>`, is copied once. A reversed range panics, whichever buffer backs the value.

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
- `AppContext::focused_text_editor` also reports a focused `NumberInput` (its draft). `FocusedTextEditor::is_numeric()` identifies it. A host that routes editor keys itself steps the field on plain ArrowUp/ArrowDown (`step_focused_number_input`) and commits it on Enter (`commit_focused_number_input`) instead of moving the caret or submitting. `RuntimeInputAdapter` already does this.

### nana-text

- `EditSession` supports multiple selections:
  - `with_selections`, `selections`, `additional_selections`, `set_selections`, `set_primary_selection`, `add_selections`, `collapse_selections`;
  - multi-cursor `insert` / `delete` / `move_caret` / `cut`;
  - `selected_texts`.
- Batch edits: `splice(edits, primary, additional)` and `assign(&SharedText, selections)`, a minimal-diff replacement.
- `EditorGeometry::sync_stamped`, `editable::collapse_edge`, `editable::normalize_selections` / `remap_offset` / `remap_selection`, and `editable::diff::changed_range`.
- `EditSession::assign` while composing: a change the text leaves ambiguous (`"abab"` → `"ab"` lost either `"ab"`) is placed clear of the composition where it can be, so the composition survives, and the primary selection moves with the composition in `assign` and `splice` alike. This is `nana-text` behavior for direct session users; Runtime's value writes re-place the preedit over the selection they write, as before.
- `EditState` gains a public `additional: Vec<EditSelection>` field; code that builds it with a struct literal or destructures it exhaustively must add it.
- `TextWorkCounters` gains `editor_text_bytes_compared`.

## Behavior changes

- **Left/Right with a selection:** without Shift, Left/Right collapse the selection onto its edge on that side of the screen instead of stepping one grapheme from the focus. In right-to-left text the logical end is on the left. Fields that cannot be probed (masked, empty) collapse onto the logical edge.
- **Empty preedit:** the empty preedit a platform reports between keystrokes is no longer treated as a composition.
  - Line numbers, extra cursors, inlays, completions and hover all stay visible.
  - It still blocks form submit until the commit or cancel arrives, as before.
- **Selection sets:** the world and components normalize selections the same way. Overlapping or touching selections merge, and ends inside a grapheme cluster snap back to the cluster boundary. Before, an extra cursor could sit inside an emoji ZWJ sequence.
  - A selection that covers the one it merges with keeps its own direction and affinity (the primary's, when both are the same span). Two carets that meet at a soft-wrapped line end stay there, and Shift+Home from two cursors keeps its focus on the line start. Before, every merge produced a forward selection with downstream affinity.
- **Undo:**
  - A cut is its own step, and typing right after it no longer merges into it.
  - Inserting a snippet is one step, and the single-line length limit now applies to it.
- **NumberInput IME:** committed and surrounding-deleted text goes through the component. Before, the next keystroke overwrote it.
- **NumberInput editing:** the draft is a full editor.
  - Undo and redo reach it. Stepping, committing (Enter or blur) and reverting (Escape) are each their own undo step. `set_number_value` and accessibility `SetValue` clear the history. Undo restores the draft only; the number is parsed again on the next commit.
  - Left/Right, word deletes, Shift+ArrowUp/Down selection and pointer caret placement work in it. A press on the spinner places no caret.
  - A read-only field's text can be selected and copied.
  - Accessibility `Click` focuses it, `SetSelection` selects in the draft, and `SetValue` parses, snaps and clamps the text by the field's policy and commits it (emitting `NumberChanged`). Text that is not a number is refused. Before, all three returned `false`.
  - `NumberChanged` is emitted only when the number moves. Before, `set_number_value` and a step emitted it whenever the draft was rewritten, even with the same number.
- **Android:** the IME buffer is the session's committed text with the preedit in place of the selection it stands for (`display_text()`, not the masked or folded text the editor draws); before, the preedit was inserted next to the focus.
  - The selection and composing region cross to GameTextInput in UTF-16 code units, the Java side's indices. Before, UTF-8 byte offsets were passed through as they were, which put the IME's composing region and cursor in the wrong place in any non-ASCII text.
- **Folding:** pressing Right across a collapsed fold leaves a caret, not a selection covering the hidden lines.
- **Undo after undo:** typing after an undo or redo starts a new step instead of merging into the step the undo stepped back onto.
- **Cut and copy over an atom:** a selection that cuts into an atom copies the whole atom, which is what a cut deletes.
- **Accessibility:** a secure (password) field without a label no longer falls back to its text for its accessible name.
