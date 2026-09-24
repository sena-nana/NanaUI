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
- `TextEditOrigin` gains `Cut` and `Step` (a numeric field's arrow key or spinner; a run of steps merges into one undo step). Exhaustive matches add both arms.
- `AppContext::focused_text_editor` also reports a focused `NumberInput` (its draft). `FocusedTextEditor::is_numeric()` identifies it. A host that routes editor keys itself steps the field on plain ArrowUp/ArrowDown (`step_focused_number_input`) and commits it on Enter (`commit_focused_number_input`) instead of moving the caret or submitting. Enter with Shift or Alt commits too, and Alt+ArrowUp/Down move the caret as in any single-line field. It consumes the arrows even when nothing moves, so they never reach an enclosing table or tree. An Enter that commits nothing is not consumed, so a dialog or form can still confirm on it. `RuntimeInputAdapter` already does this.
- New `AppContext::focused_text_editor_composing`: a plain editor (TextArea, TextInput, NumberInput) is focused with an IME composition in progress. While it holds, `RuntimeInputAdapter` consumes the editor's caret-navigation keys (arrows, Home/End, PageUp/PageDown), so a composing field inside a table, tree or select no longer loses focus to that navigation. Composite search surfaces (command palette, search dropdown, context menu) keep their list navigation while composing.
- `AppContext::replace_focused_text` (typing) refuses text carrying the control characters hosts report with command keys (`"\r"` for Enter, `"\u{1b}"` for Escape, `"\u{8}"` for Backspace) and returns `false`; Tab and newline still type, and paste keeps its text as it is. Before, an Enter or Escape a NumberInput did not use landed in its draft, through `RuntimeInputAdapter` or any host that forwards key text.
- `NumberSteppers::contains(x, y)` hits either spinner half whether or not it is enabled.
- `nana_ui_core::NumberFieldSpec::parse_unsnapped(text)` reads the number a draft spells before bounds or the grid apply; `parse` is it plus `snap`.

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
  - Undo and redo reach it. Committing (Enter or blur) and reverting (Escape) are each their own undo step; a commit also ends a run of steps when the draft already read the number. A run of steps (a held arrow key, repeated spinner presses) is one. `set_number_value` and accessibility `SetValue` clear the history whenever they change the field, even when the draft already read the new number. Undo and redo restore the draft and the committed number the field held at that point, which the history records with the text (emitting `NumberChanged` when the number moves). Undoing a step takes the number back even when the draft was empty; undoing or redoing typing never commits the typed number. Only edits that change the text are undo steps: a commit whose draft already read the number, or a run of steps that ends on the text it started from, rides on the edit before it (redo returns to the number it left), and still ends the redo branch and the typing run. With no edit before it (a fresh or cleared history) it is an undo step of its own.
  - Steps, spinner presses, Escape and accessibility `SetValue` do nothing while an IME composition is in progress; the composition owns the draft until it commits or cancels.
  - `set_number_value` with the number the field already holds changes nothing: a controlled field echoing `NumberChanged` back keeps the user's draft and undo history. Before, it rewrote the draft whenever the text differed.
  - A step starts from the typed draft when it parses (`21` then ArrowUp gives `22`). Before, it started from the committed value and discarded the draft. A draft that does not parse still steps from the committed value. The field's accessible `numeric_value` is this same number (the parsed draft clamped to the bounds, or the committed value), so the spinner's enabled halves, a spinner press and the arrow keys agree. A step that cannot move away from that number (including one a bound off the grid snaps back) changes nothing, even when the committed value differs. The published `numeric_minimum` / `numeric_maximum` are the reachable bounds on the field's grid (a maximum of 10 on a grid of 3 publishes 9).
  - Left/Right, word deletes, Shift+ArrowUp/Down selection and pointer caret placement work in it. A press on the spinner places no caret, including on a half that cannot move.
  - A read-only field's text can be selected and copied.
  - The draft never takes control characters: a newline or tab typed or pasted into it is dropped and the rest kept (a pasted `12\r\n` inserts `12`).
  - A hover card set to preserve editor focus keeps a focused NumberInput editing, as it does a TextInput.
  - Accessibility `Click` focuses it, `SetSelection` selects in the draft, and `SetValue` commits a number within the field's bounds, snapped to its grid, shows it in place of any draft (emitting `NumberChanged` when it moves), and reports success. A number outside the bounds, or text that is not a number, is refused and changes nothing, the pending draft included. Before, all three returned `false`.
  - `NumberChanged` is emitted only when the number moves. Before, `set_number_value` and a step emitted it whenever the draft was rewritten, even with the same number.
- **Android:** the IME buffer is the session's committed text with the preedit in place of the selection it stands for (`display_text()`, not the masked or folded text the editor draws); before, the preedit was inserted next to the focus.
  - The selection and composing region cross to GameTextInput in UTF-16 code units, the Java side's indices. Before, UTF-8 byte offsets were passed through as they were, which put the IME's composing region and cursor in the wrong place in any non-ASCII text.
- **Folding:** pressing Right across a collapsed fold leaves a caret, not a selection covering the hidden lines.
- **Undo after undo:** typing after an undo or redo starts a new step instead of merging into the step the undo stepped back onto.
- **Cut and copy over an atom:** a selection that cuts into an atom copies the whole atom, which is what a cut deletes.
- **Accessibility:** a secure (password) field without a label no longer falls back to its text for its accessible name.
