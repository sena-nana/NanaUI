//! `RichTextEditor`: editing an application-owned [`RichText`] in place.
//!
//! The editor is a plain text node showing the document — the same node kind,
//! the same `SetRichText` path and the same retained `nana-text` layout a
//! [`crate::RichTextView`] uses. Its lines are therefore the display's lines
//! by construction, not by two layouts agreeing: what the streamer types is
//! what the dialogue box will show.
//!
//! What the component adds is the editing state the application does not
//! own: the selection, the IME preedit (drawn styled like what it will
//! commit to, underlined), the attributes the next typed character takes, and
//! an undo history of document snapshots. Every change of the document is
//! reported as [`RichTextEditorEvent::Changed`]; the application keeps the
//! value and hands it back, and an equal value hands nothing back.
//!
//! Caret, selection and hit testing read the node's retained layout. The
//! scene draws the caret and the selection from that same layout at
//! extraction, so they never lag the glyphs. Editor-only objects (marker
//! chips) take no room on the line and show only here.

use std::ops::Range;
use std::sync::{Arc, Mutex};

use nana_text::TextLayout;
use nana_ui_core::{RichObject, RichSpanStyle, RichText};
use nana_ui_input::{CompositionInput, HostServices, InputModifiers};
use unicode_segmentation::UnicodeSegmentation;

use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, AppContext, ComponentView, DocumentId, Entity,
    FrameworkError, InteractionState, MutationQueue, NodeKind, NodeStyle, RichEditorMarks,
    StableNodeId, UiWorld,
};

/// Undo steps kept per editor.
const HISTORY_LIMIT: usize = 200;

/// What the application asks of a [`RichTextEditor`] through
/// [`AppContext::rich_edit`].
#[derive(Debug, Clone, PartialEq)]
pub enum RichEditCommand {
    /// Lay `style` over the selection, field by field (a toolbar's bold, a
    /// colour, an effect). With nothing selected it becomes what the next
    /// typed character is styled with.
    SetAttrs(RichSpanStyle),
    /// Drop every span over the selection (or the typing attributes).
    ClearFormatting,
    /// Replace the selection with `text`, styled as typing would style it.
    InsertText(String),
    /// Replace the selection with an inline object.
    InsertObject(RichObject),
    /// Replace the object at byte `offset`.
    SetObject {
        offset: usize,
        object: RichObject,
    },
    /// Set a ruby annotation above the selection (`Some`), replacing any it
    /// overlaps, or remove every annotation it touches (`None`). Horizontal
    /// text only.
    SetRuby(Option<String>),
    /// Select a byte range (snapped to characters).
    Select(Range<usize>),
    SelectAll,
    Undo,
    Redo,
}

/// One attribute of [`RichSelectionAttrs`], as a bit of [`RichAttrMask`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct RichAttrMask(pub u16);

impl RichAttrMask {
    pub const FAMILY: Self = Self(1 << 0);
    pub const SIZE: Self = Self(1 << 1);
    pub const WEIGHT: Self = Self(1 << 2);
    pub const ITALIC: Self = Self(1 << 3);
    pub const LETTER_SPACING: Self = Self(1 << 4);
    pub const FEATURES: Self = Self(1 << 5);
    pub const COLOR: Self = Self(1 << 6);
    pub const DECORATION: Self = Self(1 << 7);
    pub const DECORATION_COLOR: Self = Self(1 << 8);
    pub const STROKE: Self = Self(1 << 9);
    pub const SHADOWS: Self = Self(1 << 10);
    pub const EFFECT: Self = Self(1 << 11);

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// The attributes across a selection, tri-state per field: a field set in
/// `style` and not in `mixed` is that value everywhere; unset and not mixed
/// is unset everywhere; a bit in `mixed` means the selection disagrees. What
/// a toolbar shows as pressed, released or indeterminate.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RichSelectionAttrs {
    pub style: RichSpanStyle,
    pub mixed: RichAttrMask,
}

/// What a [`RichTextEditor`] reports.
#[derive(Debug, Clone, PartialEq)]
pub enum RichTextEditorEvent {
    /// The document changed. The application keeps this value.
    Changed(RichText),
    /// The selection or the attributes at it changed.
    SelectionChanged {
        selection: Range<usize>,
        attrs: RichSelectionAttrs,
    },
    /// An inline object was clicked.
    ObjectActivated { id: u64, offset: usize },
    /// An edit removed an inline object (with its character).
    ObjectRemoved { id: u64, offset: usize },
    /// A paste found nothing it could insert as text: the application may
    /// paste from its own clipboard formats (an image) with
    /// [`RichEditCommand::InsertObject`].
    PasteRequested,
}

#[derive(Debug, Clone, PartialEq)]
struct Snapshot {
    value: RichText,
    anchor: usize,
    focus: usize,
}

/// Runtime-owned editing state: not the application's, and kept when the
/// application hands the component back with the same document.
#[derive(Debug, Clone, PartialEq, Default)]
struct EditState {
    anchor: usize,
    focus: usize,
    /// IME composition: the preedit text and its cursor (bytes into it).
    preedit: Option<(String, usize)>,
    /// What the next typed character is styled with, when the caret was
    /// given attributes of its own.
    typing: Option<RichSpanStyle>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// The last edit was typing at a caret, so the next one joins its step.
    coalesce: bool,
    dragging: Option<u64>,
}

/// A rich text field: edits a [`RichText`] the application owns.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RichTextEditor {
    /// The document. The editor reports every change with
    /// [`RichTextEditorEvent::Changed`].
    pub value: RichText,
    pub read_only: bool,
    pub disabled: bool,
    pub style: NodeStyle,
    state: EditState,
}

impl RichTextEditor {
    pub fn new(value: impl Into<RichText>) -> Self {
        let mut style = NodeStyle::default();
        Arc::make_mut(&mut style.layout).cursor = Some(nana_ui_core::CursorSpec::Text);
        Self {
            value: value.into(),
            read_only: false,
            disabled: false,
            style,
            state: EditState::default(),
        }
    }

    pub fn read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        let cursor = self.style.layout.cursor;
        self.style = style;
        if self.style.layout.cursor.is_none() {
            Arc::make_mut(&mut self.style.layout).cursor = cursor;
        }
        self
    }

    /// The node's own font size: what every span that does not set one is
    /// typed at. Set it to the display's, so the two lay out alike.
    pub fn font_size(mut self, size: f32) -> Self {
        Arc::make_mut(&mut self.style.layout).font_size = Some(size);
        self
    }

    pub fn font_family(mut self, family: impl Into<String>) -> Self {
        Arc::make_mut(&mut self.style.layout).font_family = Some(family.into());
        self
    }

    pub fn line_height(mut self, ratio: f32) -> Self {
        Arc::make_mut(&mut self.style.layout).line_height =
            Some(nana_ui_core::LineHeightSpec::Relative(ratio));
        self
    }

    pub fn width(mut self, width: nana_ui_core::LengthSpec) -> Self {
        Arc::make_mut(&mut self.style.layout).width = Some(width);
        self
    }

    /// The selection as `(anchor, focus)` bytes of the document.
    pub fn selection(&self) -> (usize, usize) {
        (self.state.anchor, self.state.focus)
    }

    /// The IME preedit, while one is open.
    pub fn preedit(&self) -> Option<&str> {
        self.state.preedit.as_ref().map(|(text, _)| text.as_str())
    }

    fn range(&self) -> Range<usize> {
        self.state.anchor.min(self.state.focus)..self.state.anchor.max(self.state.focus)
    }

    fn editable(&self) -> bool {
        !self.read_only && !self.disabled
    }

    /// What the node shows: the document with the preedit spliced in over
    /// the selection, styled as it will commit and underlined.
    fn display(&self) -> (RichText, RichEditorMarks) {
        let range = self.range();
        let Some((preedit, cursor)) = &self.state.preedit else {
            return (
                self.value.clone(),
                RichEditorMarks {
                    selection: range,
                    caret: self.editable().then_some(self.state.focus),
                    show_editor_objects: true,
                },
            );
        };
        let mut display = self.value.clone();
        display.replace_range(range.clone(), preedit);
        let inserted = range.start..range.start + preedit.len();
        let mut style = self.typing_style();
        let mut decoration = style.paint.decoration.unwrap_or_default();
        decoration.underline = true;
        style.paint.decoration = Some(decoration);
        display.set_span(inserted, style);
        (
            display,
            RichEditorMarks {
                selection: range.start..range.start,
                caret: Some(range.start + (*cursor).min(preedit.len())),
                show_editor_objects: true,
            },
        )
    }

    /// What a character typed at the caret is styled with: the caret's own
    /// attributes, else the character before it (the one typing continues),
    /// else the one after.
    fn typing_style(&self) -> RichSpanStyle {
        if let Some(typing) = &self.state.typing {
            return typing.clone();
        }
        let at = self.range().start;
        let before = self.value.text()[..at]
            .char_indices()
            .next_back()
            .map(|(index, _)| index);
        before
            .and_then(|index| self.value.style_at(index))
            .or_else(|| {
                (at < self.value.text().len())
                    .then(|| self.value.style_at(at))
                    .flatten()
            })
            .cloned()
            .unwrap_or_default()
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            value: self.value.clone(),
            anchor: self.state.anchor,
            focus: self.state.focus,
        }
    }

    fn remember(&mut self, coalesce: bool) {
        if !(coalesce && self.state.coalesce) {
            let snapshot = self.snapshot();
            self.state.undo.push(snapshot);
            if self.state.undo.len() > HISTORY_LIMIT {
                self.state.undo.remove(0);
            }
        }
        self.state.redo.clear();
        self.state.coalesce = coalesce;
    }

    fn set_caret(&mut self, offset: usize) {
        self.state.anchor = offset;
        self.state.focus = offset;
    }

    /// Replace the selection with `piece`. `typed` styles plain typing with
    /// the caret's attributes and lets consecutive typing share an undo step.
    fn replace_selection(&mut self, piece: &RichText, typed: bool) -> bool {
        if !self.editable() {
            return false;
        }
        let range = self.range();
        if range.is_empty() && piece.text().is_empty() {
            return false;
        }
        let style = typed.then(|| self.typing_style());
        self.remember(typed && range.is_empty() && !piece.text().contains('\n'));
        self.value.replace_with(range.clone(), piece);
        let inserted = range.start..range.start + piece.text().len();
        if let Some(style) = style
            && !inserted.is_empty()
        {
            self.value.set_span(inserted.clone(), style);
        }
        self.set_caret(inserted.end);
        true
    }

    fn insert_text(&mut self, text: &str) -> bool {
        let text: String = text
            .chars()
            .filter(|ch| *ch == '\n' || *ch == '\t' || !ch.is_control())
            .collect();
        if text.is_empty() {
            return false;
        }
        self.replace_selection(&RichText::new(text), true)
    }

    fn delete(&mut self, range: Range<usize>) -> bool {
        if !self.editable() || range.is_empty() {
            return false;
        }
        self.remember(false);
        self.value.replace_range(range.clone(), "");
        self.set_caret(range.start);
        true
    }

    fn apply_command(&mut self, command: RichEditCommand) -> bool {
        match command {
            RichEditCommand::SetAttrs(style) => {
                let range = self.range();
                if range.is_empty() {
                    let typing = self.typing_style().overlay(&style);
                    self.state.typing = Some(typing);
                    return true;
                }
                if !self.editable() {
                    return false;
                }
                self.remember(false);
                self.value.apply_span(range, &style);
                true
            }
            RichEditCommand::ClearFormatting => {
                let range = self.range();
                if range.is_empty() {
                    self.state.typing = Some(RichSpanStyle::default());
                    return true;
                }
                if !self.editable() {
                    return false;
                }
                self.remember(false);
                self.value.set_span(range, RichSpanStyle::default());
                true
            }
            RichEditCommand::InsertText(text) => self.insert_text(&text),
            RichEditCommand::InsertObject(object) => {
                let mut piece = RichText::default();
                piece.insert_object(0, object);
                self.replace_selection(&piece, true)
            }
            RichEditCommand::SetObject { offset, object } => {
                if !self.editable() || self.value.object_at(offset).is_none() {
                    return false;
                }
                self.remember(false);
                self.value.set_object(offset, object)
            }
            RichEditCommand::SetRuby(annotation) => {
                let range = self.range();
                if !self.editable() {
                    return false;
                }
                match annotation.filter(|text| !text.is_empty()) {
                    Some(text) if !range.is_empty() => {
                        self.remember(false);
                        self.value.set_ruby(range, text);
                    }
                    Some(_) => return false,
                    None => {
                        if !self.value.rubies().iter().any(|(base, _)| {
                            if range.is_empty() {
                                base.contains(&range.start)
                            } else {
                                base.start < range.end && range.start < base.end
                            }
                        }) {
                            return false;
                        }
                        self.remember(false);
                        self.value.clear_ruby(range);
                    }
                }
                true
            }
            RichEditCommand::Select(range) => {
                let len = self.value.text().len();
                self.state.anchor = floor_char(self.value.text(), range.start.min(len));
                self.state.focus = floor_char(self.value.text(), range.end.min(len));
                self.state.typing = None;
                self.state.coalesce = false;
                true
            }
            RichEditCommand::SelectAll => {
                self.state.anchor = 0;
                self.state.focus = self.value.text().len();
                self.state.typing = None;
                true
            }
            RichEditCommand::Undo => self.undo(),
            RichEditCommand::Redo => self.redo(),
        }
    }

    fn undo(&mut self) -> bool {
        let Some(snapshot) = self.state.undo.pop() else {
            return false;
        };
        let current = self.snapshot();
        self.state.redo.push(current);
        self.restore(snapshot);
        true
    }

    fn redo(&mut self) -> bool {
        let Some(snapshot) = self.state.redo.pop() else {
            return false;
        };
        let current = self.snapshot();
        self.state.undo.push(current);
        self.restore(snapshot);
        true
    }

    fn restore(&mut self, snapshot: Snapshot) {
        self.value = snapshot.value;
        self.state.anchor = snapshot.anchor;
        self.state.focus = snapshot.focus;
        self.state.typing = None;
        self.state.coalesce = false;
    }

    /// The attributes across the selection, or what typing at the caret
    /// would use.
    pub fn selection_attrs(&self) -> RichSelectionAttrs {
        let range = self.range();
        if range.is_empty() {
            return RichSelectionAttrs {
                style: self.typing_style(),
                mixed: RichAttrMask::default(),
            };
        }
        summarize(&self.value, range)
    }

    /// Emit what changed between `before` and now.
    fn report(
        &self,
        before: &Snapshot,
        attrs: &RichSelectionAttrs,
        cx: &mut crate::ViewContext<'_, Self>,
    ) {
        if self.value != before.value {
            for (offset, object) in before.value.objects() {
                let kept = self
                    .value
                    .objects()
                    .iter()
                    .any(|(_, current)| current.id == object.id);
                if !kept {
                    cx.emit(RichTextEditorEvent::ObjectRemoved {
                        id: object.id,
                        offset: *offset,
                    });
                }
            }
            cx.emit(RichTextEditorEvent::Changed(self.value.clone()));
        }
        let now = self.selection_attrs();
        if (self.state.anchor, self.state.focus) != (before.anchor, before.focus) || now != *attrs {
            cx.emit(RichTextEditorEvent::SelectionChanged {
                selection: self.range(),
                attrs: now,
            });
        }
    }
}

/// The tri-state summary of the spans over `range`.
fn summarize(value: &RichText, range: Range<usize>) -> RichSelectionAttrs {
    // Every character's style, uniform or not: a gap is the default style.
    let mut pieces: Vec<RichSpanStyle> = Vec::new();
    let mut cursor = range.start;
    for (span, style) in value.spans().iter() {
        if span.end <= range.start || span.start >= range.end {
            continue;
        }
        if span.start > cursor {
            pieces.push(RichSpanStyle::default());
        }
        pieces.push(style.clone());
        cursor = span.end;
    }
    if cursor < range.end {
        pieces.push(RichSpanStyle::default());
    }
    let Some(first) = pieces.first().cloned() else {
        return RichSelectionAttrs::default();
    };
    let mut mixed = 0u16;
    for piece in &pieces[1..] {
        let checks = [
            (
                piece.shape.family != first.shape.family,
                RichAttrMask::FAMILY,
            ),
            (
                piece.shape.size_px != first.shape.size_px,
                RichAttrMask::SIZE,
            ),
            (
                piece.shape.weight != first.shape.weight,
                RichAttrMask::WEIGHT,
            ),
            (
                piece.shape.italic != first.shape.italic,
                RichAttrMask::ITALIC,
            ),
            (
                piece.shape.letter_spacing_px != first.shape.letter_spacing_px,
                RichAttrMask::LETTER_SPACING,
            ),
            (
                piece.shape.features != first.shape.features,
                RichAttrMask::FEATURES,
            ),
            (piece.paint.color != first.paint.color, RichAttrMask::COLOR),
            (
                piece.paint.decoration != first.paint.decoration,
                RichAttrMask::DECORATION,
            ),
            (
                piece.paint.decoration_color != first.paint.decoration_color,
                RichAttrMask::DECORATION_COLOR,
            ),
            (
                piece.paint.stroke != first.paint.stroke,
                RichAttrMask::STROKE,
            ),
            (
                piece.paint.shadows != first.paint.shadows,
                RichAttrMask::SHADOWS,
            ),
            (piece.effect != first.effect, RichAttrMask::EFFECT),
        ];
        for (differs, bit) in checks {
            if differs {
                mixed |= bit.0;
            }
        }
    }
    let mut style = first;
    let mask = RichAttrMask(mixed);
    if mask.contains(RichAttrMask::FAMILY) {
        style.shape.family = None;
    }
    if mask.contains(RichAttrMask::SIZE) {
        style.shape.size_px = None;
    }
    if mask.contains(RichAttrMask::WEIGHT) {
        style.shape.weight = None;
    }
    if mask.contains(RichAttrMask::ITALIC) {
        style.shape.italic = None;
    }
    if mask.contains(RichAttrMask::LETTER_SPACING) {
        style.shape.letter_spacing_px = None;
    }
    if mask.contains(RichAttrMask::FEATURES) {
        style.shape.features = None;
    }
    if mask.contains(RichAttrMask::COLOR) {
        style.paint.color = None;
    }
    if mask.contains(RichAttrMask::DECORATION) {
        style.paint.decoration = None;
    }
    if mask.contains(RichAttrMask::DECORATION_COLOR) {
        style.paint.decoration_color = None;
    }
    if mask.contains(RichAttrMask::STROKE) {
        style.paint.stroke = None;
    }
    if mask.contains(RichAttrMask::SHADOWS) {
        style.paint.shadows = None;
    }
    if mask.contains(RichAttrMask::EFFECT) {
        style.effect = None;
    }
    RichSelectionAttrs { style, mixed: mask }
}

fn floor_char(text: &str, mut offset: usize) -> usize {
    offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    offset
}

fn previous_grapheme(text: &str, offset: usize) -> usize {
    text[..offset]
        .grapheme_indices(true)
        .next_back()
        .map_or(0, |(index, _)| index)
}

fn next_grapheme(text: &str, offset: usize) -> usize {
    text[offset..]
        .grapheme_indices(true)
        .nth(1)
        .map_or(text.len(), |(index, _)| offset + index)
}

fn previous_word(text: &str, offset: usize) -> usize {
    let mut last = 0;
    for (index, word) in text[..offset].split_word_bound_indices() {
        if !word.trim().is_empty() {
            last = index;
        }
    }
    last
}

fn next_word(text: &str, offset: usize) -> usize {
    for (index, word) in text[offset..].split_word_bound_indices() {
        if !word.trim().is_empty() {
            return offset + index + word.len();
        }
    }
    text.len()
}

/// The line of `layout` a caret at `byte` sits on.
fn line_of(layout: &TextLayout, byte: usize) -> Option<usize> {
    let lines = &layout.lines;
    if lines.is_empty() {
        return None;
    }
    let mut found = 0;
    for (index, line) in lines.iter().enumerate() {
        if line.source.start <= byte {
            found = index;
        }
        if byte < line.source.end {
            break;
        }
    }
    Some(found)
}

/// Where a caret at `byte` draws, as `(x, top, height)` in layout space.
/// The scene draws the editor's caret with this, from the retained layout.
pub fn caret_box(layout: &TextLayout, byte: usize) -> Option<(f32, f32, f32)> {
    let line = line_of(layout, byte)?;
    let caret = nana_text::CaretPosition::new(byte, nana_text::Affinity::Downstream, line as u32);
    let geometry = layout.caret_geometry(caret).or_else(|| {
        layout.caret_geometry(nana_text::CaretPosition::new(
            byte,
            nana_text::Affinity::Upstream,
            line as u32,
        ))
    })?;
    Some((geometry.x_px, geometry.top_y_px, geometry.height_px))
}

/// The node's laid-out text, read for navigation and hit testing.
struct Geometry {
    layout: Arc<TextLayout>,
    text: Arc<str>,
}

impl Geometry {
    fn of(world: &UiWorld, id: StableNodeId) -> Option<Self> {
        let (_, layout) = world.text_layout(id)?;
        let text = Arc::<str>::from(world.text(id)?);
        Some(Self {
            layout: Arc::clone(layout),
            text,
        })
    }

    fn vertical(&self, byte: usize, down: bool) -> Option<usize> {
        let line = line_of(&self.layout, byte)?;
        let target = if down { line + 1 } else { line.checked_sub(1)? };
        let target = self.layout.lines.get(target)?;
        let (x, _, _) = caret_box(&self.layout, byte)?;
        let y = target.metrics.top_y_px + target.metrics.height_px * 0.5;
        Some(self.layout.hit_test_text(&self.text, x, y).caret.byte)
    }

    fn line_edge(&self, byte: usize, end: bool) -> Option<usize> {
        let line = &self.layout.lines[line_of(&self.layout, byte)?];
        if !end {
            return Some(line.source.start);
        }
        let mut edge = line.source.end.min(self.text.len());
        if self.text[..edge].ends_with('\n') && edge > line.source.start {
            edge -= 1;
        }
        Some(edge)
    }
}

/// The in-process rich clipboard: the last piece copied out of a rich
/// editor, keyed by its plain text. A paste whose clipboard text is that text
/// pastes the piece with its styling and objects; anything else pastes as
/// plain text.
static RICH_CLIPBOARD: Mutex<Option<(u64, RichText)>> = Mutex::new(None);

fn text_key(text: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::hash::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

impl ComponentView for RichTextEditor {
    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    /// A new document from the application replaces the editor's; the
    /// selection, preedit and history survive an equal one.
    fn reconcile(&mut self, next: Self) {
        let state = std::mem::take(&mut self.state);
        let same = self.value == next.value;
        *self = next;
        self.state = state;
        if !same {
            let len = self.value.text().len();
            self.state.anchor = floor_char(self.value.text(), self.state.anchor.min(len));
            self.state.focus = floor_char(self.value.text(), self.state.focus.min(len));
            self.state.preedit = None;
            self.state.typing = None;
            self.state.coalesce = false;
        }
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Text
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let (display, marks) = self.display();
        if world.rich_text(id) != Some(&display) {
            mutations.set_rich_text(id, display);
        }
        if world.rich_editor_marks(id) != Some(&marks) {
            mutations.set_rich_editor_marks(id, Some(marks));
        }
        project_common(
            id,
            world,
            mutations,
            &self.style,
            InteractionState {
                pointer_events: true,
                focusable: !self.disabled,
            },
            AccessibilityState {
                role: AccessibilityRole::TextInput,
                value: Some(Arc::from(self.value.text())),
                editable: self.editable(),
                multiline: true,
                disabled: self.disabled,
                ..AccessibilityState::default()
            },
        );
    }
}

impl AppContext {
    /// Apply `command` to a rich text editor, as its toolbar or the
    /// application's shortcuts do. Returns whether anything changed.
    pub fn rich_edit(
        &mut self,
        entity: Entity<RichTextEditor>,
        command: RichEditCommand,
    ) -> Result<bool, FrameworkError> {
        self.update_component(entity, |view, cx| {
            let before = view.snapshot();
            let attrs = view.selection_attrs();
            let changed = view.apply_command(command);
            view.report(&before, &attrs, cx);
            changed
        })
    }

    /// The focused rich text editor of `document`.
    pub fn focused_rich_editor(&self, document: DocumentId) -> Option<Entity<RichTextEditor>> {
        let id = self.world().focused(document)?;
        let entity = Entity::from_stable_id(id);
        self.read(entity, |view: &RichTextEditor| !view.disabled)
            .ok()
            .filter(|enabled| *enabled)
            .map(|_| entity)
    }

    /// Whether `document`'s focused rich editor takes typing (and the IME).
    pub(crate) fn rich_editor_accepts_input(&self, document: DocumentId) -> bool {
        self.focused_rich_editor(document)
            .is_some_and(|entity| self.read(entity, RichTextEditor::editable).unwrap_or(false))
    }

    /// Where the focused rich editor's caret is, in the document's layout
    /// space: what the IME anchors its candidates to.
    pub(crate) fn rich_editor_caret_bounds(
        &self,
        document: DocumentId,
    ) -> Option<crate::LayoutBox> {
        let entity = self.focused_rich_editor(document)?;
        let id = entity.stable_id();
        let (content, scroll) = self.world().document_text_pointer_context(id)?;
        let caret = self.read(entity, |view| view.display().1.caret).ok()??;
        let (_, layout) = self.world().text_layout(id)?;
        let (x, top, height) = caret_box(layout, caret)?;
        Some(crate::LayoutBox {
            x: content.x + x - scroll.x,
            y: content.y + top - scroll.y,
            width: nana_ui_core::HAIRLINE,
            height,
        })
    }

    /// A key on the focused rich editor. `None` when no rich editor is
    /// focused; `Some(false)` for a key it leaves to generic routing (Tab).
    pub(crate) fn rich_editor_key(
        &mut self,
        document: DocumentId,
        key: &str,
        text: Option<&str>,
        modifiers: InputModifiers,
        services: &mut dyn HostServices,
    ) -> Result<Option<bool>, FrameworkError> {
        let Some(entity) = self.focused_rich_editor(document) else {
            return Ok(None);
        };
        if self.read(entity, |view| view.state.preedit.is_some())? {
            // The IME owns the keys while it composes.
            return Ok(Some(false));
        }
        let primary = (modifiers.control || modifiers.meta) && !modifiers.alt;
        let shift = modifiers.shift;
        if primary {
            let lower = key.to_ascii_lowercase();
            match lower.as_str() {
                "z" if shift => return self.rich_edit(entity, RichEditCommand::Redo).map(Some),
                "z" => return self.rich_edit(entity, RichEditCommand::Undo).map(Some),
                "y" => return self.rich_edit(entity, RichEditCommand::Redo).map(Some),
                "a" => return self.rich_edit(entity, RichEditCommand::SelectAll).map(Some),
                "c" | "x" => {
                    let piece = self.read(entity, |view| {
                        let range = view.range();
                        (!range.is_empty()).then(|| view.value.slice(range))
                    })?;
                    let Some(piece) = piece else {
                        return Ok(Some(false));
                    };
                    if services.write_clipboard(piece.text()).is_err() {
                        return Ok(Some(false));
                    }
                    if let Ok(mut clipboard) = RICH_CLIPBOARD.lock() {
                        *clipboard = Some((text_key(piece.text()), piece));
                    }
                    if lower == "x" {
                        self.edit_with(entity, |view, _| {
                            let range = view.range();
                            view.delete(range)
                        })?;
                    }
                    return Ok(Some(true));
                }
                "v" => {
                    let clipboard = services.read_clipboard();
                    let Ok(Some(text)) = clipboard
                        .as_ref()
                        .map(|text| text.as_ref().filter(|text| !text.is_empty()).cloned())
                    else {
                        self.update_component(entity, |_, cx| {
                            cx.emit(RichTextEditorEvent::PasteRequested)
                        })?;
                        return Ok(Some(true));
                    };
                    let piece = RICH_CLIPBOARD
                        .lock()
                        .ok()
                        .and_then(|clipboard| clipboard.clone())
                        .filter(|(key, piece)| *key == text_key(&text) && piece.text() == text)
                        .map(|(_, piece)| piece);
                    return self
                        .edit_with(entity, |view, _| match &piece {
                            Some(piece) => view.replace_selection(piece, false),
                            None => view.insert_text(&text),
                        })
                        .map(Some);
                }
                _ => {}
            }
        }
        let geometry = Geometry::of(self.world(), entity.stable_id());
        let handled = match key {
            "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown" | "Home" | "End" => self
                .edit_with(entity, |view, _| {
                    let text = view.value.text().to_owned();
                    let range = view.range();
                    let focus = view.state.focus;
                    let collapsed = range.is_empty();
                    let target = match key {
                        "ArrowLeft" if primary => previous_word(&text, focus),
                        "ArrowRight" if primary => next_word(&text, focus),
                        "ArrowLeft" if !shift && !collapsed => range.start,
                        "ArrowRight" if !shift && !collapsed => range.end,
                        "ArrowLeft" => previous_grapheme(&text, focus),
                        "ArrowRight" => next_grapheme(&text, focus),
                        "Home" if primary => 0,
                        "End" if primary => text.len(),
                        "ArrowUp" | "ArrowDown" => geometry
                            .as_ref()
                            .filter(|geometry| *geometry.text == *text)
                            .and_then(|geometry| geometry.vertical(focus, key == "ArrowDown"))
                            .unwrap_or(if key == "ArrowDown" { text.len() } else { 0 }),
                        "Home" | "End" => geometry
                            .as_ref()
                            .filter(|geometry| *geometry.text == *text)
                            .and_then(|geometry| geometry.line_edge(focus, key == "End"))
                            .unwrap_or(if key == "End" { text.len() } else { 0 }),
                        _ => focus,
                    };
                    view.state.focus = target;
                    if !shift {
                        view.state.anchor = target;
                    }
                    view.state.typing = None;
                    view.state.coalesce = false;
                    true
                })?,
            "Backspace" | "Delete" => self.edit_with(entity, |view, _| {
                let text = view.value.text().to_owned();
                let range = view.range();
                let range = if !range.is_empty() {
                    range
                } else if key == "Backspace" {
                    let start = if primary {
                        previous_word(&text, range.start)
                    } else {
                        previous_grapheme(&text, range.start)
                    };
                    start..range.start
                } else {
                    let end = if primary {
                        next_word(&text, range.end)
                    } else {
                        next_grapheme(&text, range.end)
                    };
                    range.end..end
                };
                view.delete(range)
            })?,
            "Enter" if !primary => self.edit_with(entity, |view, _| view.insert_text("\n"))?,
            "Tab" | "Escape" => return Ok(Some(false)),
            _ => match text {
                Some(text) if !primary => {
                    self.edit_with(entity, |view, _| view.insert_text(text))?
                }
                _ => false,
            },
        };
        Ok(Some(handled))
    }

    /// IME composition into the focused rich editor. `None` when none is.
    pub(crate) fn rich_editor_composition(
        &mut self,
        document: DocumentId,
        composition: &CompositionInput,
    ) -> Result<Option<bool>, FrameworkError> {
        let Some(entity) = self.focused_rich_editor(document) else {
            return Ok(None);
        };
        if !self.read(entity, RichTextEditor::editable)? {
            return Ok(Some(false));
        }
        let handled = self.edit_with(entity, |view, _| match composition {
            CompositionInput::Start => {
                view.state.preedit = Some((String::new(), 0));
                true
            }
            CompositionInput::Update { text, selection } => {
                let cursor = selection.map_or(text.len(), |(_, end)| end.min(text.len()));
                view.state.preedit = if text.is_empty() {
                    Some((String::new(), 0))
                } else {
                    Some((text.clone(), cursor))
                };
                true
            }
            CompositionInput::Commit(text) => {
                view.state.preedit = None;
                view.insert_text(text);
                true
            }
            CompositionInput::Disabled => {
                // A composition the IME abandoned still commits what it shows.
                let leftover = view
                    .state
                    .preedit
                    .take()
                    .map(|(text, _)| text)
                    .filter(|text| !text.is_empty());
                if let Some(text) = leftover {
                    view.insert_text(&text);
                }
                true
            }
            CompositionInput::End => view.state.preedit.take().is_some(),
            CompositionInput::Enabled | CompositionInput::DeleteSurrounding { .. } => false,
        })?;
        Ok(Some(handled))
    }

    /// A pointer on a rich editor: a press places the caret (Shift extends),
    /// a drag selects, a press on an inline object reports it. `phase`: 0
    /// press, 1 move, 2 release, 3 cancel.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn rich_editor_pointer(
        &mut self,
        document: DocumentId,
        target: Option<StableNodeId>,
        pointer: u64,
        phase: u8,
        x: f32,
        y: f32,
        shift: bool,
    ) -> Result<bool, FrameworkError> {
        let entity = if phase == 0 {
            target
                .map(Entity::<RichTextEditor>::from_stable_id)
                .filter(|entity| {
                    self.read(*entity, |view: &RichTextEditor| !view.disabled)
                        .unwrap_or(false)
                })
        } else {
            self.focused_rich_editor(document).filter(|entity| {
                self.read(*entity, |view| view.state.dragging == Some(pointer))
                    .unwrap_or(false)
            })
        };
        let Some(entity) = entity else {
            return Ok(false);
        };
        let id = entity.stable_id();
        let hit = self.rich_editor_hit(id, x, y);
        if phase == 0 {
            self.focus_node(document, id)?;
            self.press_pointer(document, pointer, id)?;
        }
        if phase >= 2 {
            self.release_pointer(document, pointer);
        }
        self.edit_with(entity, |view, cx| {
            match phase {
                0 => {
                    view.state.dragging = Some(pointer);
                    view.state.preedit = None;
                    if let Some((byte, object)) = hit {
                        view.state.focus = byte;
                        if !shift {
                            view.state.anchor = byte;
                        }
                        if let Some((id, offset)) = object {
                            cx.emit(RichTextEditorEvent::ObjectActivated { id, offset });
                        }
                    }
                }
                1 => {
                    if let Some((byte, _)) = hit {
                        view.state.focus = byte;
                    }
                }
                2 => {
                    if let Some((byte, _)) = hit {
                        view.state.focus = byte;
                    }
                    view.state.dragging = None;
                }
                _ => view.state.dragging = None,
            }
            view.state.typing = None;
            view.state.coalesce = false;
            true
        })
    }

    /// The document byte under `(x, y)`, and the inline object there.
    fn rich_editor_hit(
        &self,
        id: StableNodeId,
        x: f32,
        y: f32,
    ) -> Option<(usize, Option<(u64, usize)>)> {
        let (content, scroll) = self.world().document_text_pointer_context(id)?;
        let (layout_x, layout_y) = self
            .world()
            .pointer_layout_position(id, x, y)
            .unwrap_or((x, y));
        let local_x = layout_x - content.x + scroll.x;
        let local_y = layout_y - content.y + scroll.y;
        let geometry = Geometry::of(self.world(), id)?;
        let byte = geometry
            .layout
            .hit_test_text(&geometry.text, local_x, local_y)
            .caret
            .byte;
        let object = geometry.layout.objects.iter().find_map(|placed| {
            let rect = placed.rect;
            // A zero-width chip is still something to click: a few px either
            // side of where it sits.
            let half = (rect.width * 0.5).max(4.0);
            let centre = rect.x + rect.width * 0.5;
            let line = geometry.layout.lines.get(placed.line as usize)?;
            let inside = (local_x - centre).abs() <= half
                && local_y >= line.metrics.top_y_px
                && local_y <= line.metrics.top_y_px + line.metrics.height_px;
            inside.then_some((placed.id, placed.offset))
        });
        Some((byte, object))
    }

    /// Run an edit on the editor's state and report what it changed.
    fn edit_with(
        &mut self,
        entity: Entity<RichTextEditor>,
        edit: impl FnOnce(&mut RichTextEditor, &mut crate::ViewContext<'_, RichTextEditor>) -> bool,
    ) -> Result<bool, FrameworkError> {
        self.update_component(entity, |view, cx| {
            let before = view.snapshot();
            let attrs = view.selection_attrs();
            let changed = edit(view, cx);
            view.report(&before, &attrs, cx);
            changed
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nana_ui_core::PaintColor;

    fn editor(text: &str) -> RichTextEditor {
        RichTextEditor::new(text)
    }

    #[test]
    fn typing_inherits_the_caret_style_and_one_word_is_one_undo_step() {
        let bold = RichSpanStyle::new().bold();
        let mut view = RichTextEditor::new(RichText::new("ab").with_span(0..2, bold.clone()));
        view.apply_command(RichEditCommand::Select(2..2));
        view.insert_text("c");
        view.insert_text("d");
        assert_eq!(view.value.text(), "abcd");
        assert_eq!(
            view.value.style_at(3),
            Some(&bold),
            "typing continues the bold"
        );
        assert!(view.undo());
        assert_eq!(view.value.text(), "ab", "one step for the run of typing");
        assert!(view.redo());
        assert_eq!(view.value.text(), "abcd");
    }

    #[test]
    fn attrs_on_a_caret_style_what_is_typed_next() {
        let red = PaintColor::srgb([1.0, 0.0, 0.0, 1.0]);
        let mut view = editor("ab");
        view.apply_command(RichEditCommand::Select(1..1));
        view.apply_command(RichEditCommand::SetAttrs(RichSpanStyle::new().color(red)));
        assert_eq!(view.value.text(), "ab", "nothing to restyle yet");
        view.insert_text("X");
        assert_eq!(view.value.text(), "aXb");
        assert_eq!(
            view.value.style_at(1).and_then(|s| s.paint.color),
            Some(red)
        );
        assert_eq!(view.value.style_at(0), None);
    }

    #[test]
    fn a_selection_summary_is_tri_state() {
        let bold = RichSpanStyle::new().bold();
        let mut view = RichTextEditor::new(RichText::new("abcd").with_span(0..2, bold.clone()));
        view.apply_command(RichEditCommand::Select(0..2));
        let all_bold = view.selection_attrs();
        assert_eq!(all_bold.style.shape.weight, Some(700));
        assert!(all_bold.mixed.is_empty());
        view.apply_command(RichEditCommand::Select(0..4));
        let mixed = view.selection_attrs();
        assert!(mixed.mixed.contains(RichAttrMask::WEIGHT));
        assert_eq!(mixed.style.shape.weight, None);
        view.apply_command(RichEditCommand::SetAttrs(bold));
        assert!(
            view.selection_attrs().mixed.is_empty(),
            "bold over all of it"
        );
    }

    #[test]
    fn the_preedit_shows_styled_and_underlined_without_touching_the_document() {
        let mut view = editor("ab");
        view.apply_command(RichEditCommand::Select(1..1));
        view.state.preedit = Some(("ka".into(), 2));
        let (display, marks) = view.display();
        assert_eq!(display.text(), "akab");
        assert!(
            display
                .style_at(1)
                .and_then(|style| style.paint.decoration)
                .is_some_and(|decoration| decoration.underline)
        );
        assert_eq!(marks.caret, Some(3));
        assert_eq!(view.value.text(), "ab", "the preedit is not committed text");
    }

    #[test]
    fn objects_insert_at_the_caret_and_a_chip_is_one_character() {
        let mut view = editor("ab");
        view.apply_command(RichEditCommand::Select(1..1));
        view.apply_command(RichEditCommand::InsertObject(RichObject::chip(
            3, "wait", 0,
        )));
        assert_eq!(view.value.objects().len(), 1);
        assert_eq!(view.value.objects()[0].0, 1);
        let caret = view.selection().1;
        assert_eq!(caret, 1 + nana_ui_core::OBJECT_REPLACEMENT.len_utf8());
        let text = view.value.text().to_owned();
        assert_eq!(
            previous_grapheme(&text, caret),
            1,
            "a caret steps over it in one move"
        );
    }

    #[test]
    fn read_only_keeps_the_document_but_moves_the_selection() {
        let mut view = editor("ab").read_only(true);
        assert!(!view.insert_text("x"));
        assert!(view.apply_command(RichEditCommand::SelectAll));
        assert_eq!(view.selection(), (0, 2));
        assert!(
            !view.apply_command(RichEditCommand::SetAttrs(RichSpanStyle::new().bold())),
            "a read-only selection cannot be restyled"
        );
        assert_eq!(view.value.text(), "ab");
    }
}
