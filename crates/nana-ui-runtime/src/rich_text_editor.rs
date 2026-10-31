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
use std::sync::Arc;

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

#[path = "rich_clipboard.rs"]
mod clipboard;
pub use clipboard::RichClipboardCommand;

#[path = "rich_ime.rs"]
mod ime;
pub use ime::RichImeSnapshot;

/// 新输入文字的样式来源；富文本粘贴保留剪贴板携带的样式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RichTypingStylePolicy {
    /// 沿用光标位置或工具栏设置的输入样式。
    #[default]
    Inherit,
    /// 使用节点默认样式，供纯文本输入为主的编辑器选择。
    Plain,
}

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
    /// Clear only the named character attributes.
    ClearAttrs(RichAttrMask),
    /// Apply to the current or selected LF paragraphs. None inherits the node.
    SetParagraphAlignment(Option<nana_ui_core::TextAlignSpec>),
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

/// Where text typed at a caret takes its attributes from. Attributes the
/// caret was given (a toolbar's bold with nothing selected) apply under
/// either; a rich paste keeps the styling it carries under either.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RichTypingStylePolicy {
    /// The character before the caret, else the one after: typing continues
    /// the text it lands in.
    #[default]
    Inherit,
    /// The node's own style: neighbouring spans do not carry over, for an
    /// editor mostly typed into as plain text.
    Plain,
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
    pub const BACKGROUND_COLOR: Self = Self(1 << 12);
    pub const STRETCH: Self = Self(1 << 13);
    pub const EMPHASIS: Self = Self(1 << 14);

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
    /// Drop the selected fields, preserving every other attribute.
    fn cleared(self, mut style: RichSpanStyle) -> RichSpanStyle {
        if self.contains(Self::SIZE) {
            style.shape.size_px = None;
            style.shape.size_em = None;
        }
        if self.contains(Self::FAMILY) {
            style.shape.family = None;
            style.shape.family_alias = None;
        }
        if self.contains(Self::WEIGHT) {
            style.shape.weight = None;
        }
        if self.contains(Self::STRETCH) {
            style.shape.stretch_percent = None;
        }
        if self.contains(Self::ITALIC) {
            style.shape.italic = None;
        }
        if self.contains(Self::LETTER_SPACING) {
            style.shape.letter_spacing_px = None;
        }
        if self.contains(Self::FEATURES) {
            style.shape.features = None;
        }
        if self.contains(Self::COLOR) {
            style.paint.color = None;
        }
        if self.contains(Self::BACKGROUND_COLOR) {
            style.paint.background_color = None;
        }
        if self.contains(Self::STRETCH) {
            style.shape.stretch_percent = None;
        }
        if self.contains(Self::EMPHASIS) {
            style.paint.emphasis = None;
        }
        if self.contains(Self::DECORATION) {
            style.paint.decoration = None;
        }
        if self.contains(Self::DECORATION_COLOR) {
            style.paint.decoration_color = None;
        }
        if self.contains(Self::STROKE) {
            style.paint.stroke = None;
        }
        if self.contains(Self::SHADOWS) {
            style.paint.shadows = None;
        }
        if self.contains(Self::EFFECT) {
            style.effect = None;
        }
        if self.contains(Self::EMPHASIS) {
            style.emphasis = Default::default();
        }
        if self.contains(Self::EMPHASIS_COLOR) {
            style.paint.emphasis_color = None;
        }
        style
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
    pub paragraph_alignment: Option<nana_ui_core::TextAlignSpec>,
    pub paragraph_alignment_mixed: bool,
}

/// What a [`RichTextEditor`] reports.
#[derive(Debug, Clone, PartialEq)]
#[expect(
    clippy::large_enum_variant,
    reason = "one event per selection change; boxing `attrs` breaks every consumer's match"
)]
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

/// 跟踪实际指针移动，避免取消预编辑后的布局变化重新解释静止指针。
#[derive(Debug, Clone, Copy, PartialEq)]
struct DragState {
    pointer: u64,
    position: (f32, f32),
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
    /// 输入、选区及预编辑操作用于重新启动绘制光标的可见阶段。
    caret_revision: u64,
    dragging: Option<DragState>,
    click: Option<(u64, std::time::Duration, f32, f32, u8)>,
    drag_range: Option<Range<usize>>,
    drag_unit: u8,
    preferred_x: Option<f32>,
    /// Counts each move of the caret and each change of the document or
    /// preedit; input routing restarts the caret blink when it changes.
    caret_revision: u64,
}

/// A rich text field: edits a [`RichText`] the application owns.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RichTextEditor {
    /// The document. The editor reports every change with
    /// [`RichTextEditorEvent::Changed`].
    pub value: RichText,
    pub read_only: bool,
    pub disabled: bool,
    /// 键盘输入、输入法及纯文本粘贴共用的样式策略。
    pub typing_style_policy: RichTypingStylePolicy,
    pub style: NodeStyle,
    /// Requested viewport offset. User scrolling leaves the selection intact.
    pub scroll_offset: crate::ScrollOffset,
    state: EditState,
}

impl RichTextEditor {
    pub fn new(value: impl Into<RichText>) -> Self {
        let mut style = NodeStyle::default();
        Arc::make_mut(&mut style.layout).cursor = Some(nana_ui_core::CursorSpec::Text);
        Arc::make_mut(&mut style.layout).overflow_x = nana_ui_core::OverflowSpec::Hidden;
        Arc::make_mut(&mut style.layout).overflow_y = nana_ui_core::OverflowSpec::Hidden;
        Self {
            value: value.into(),
            read_only: false,
            disabled: false,
            typing_style_policy: RichTypingStylePolicy::default(),
            style,
            scroll_offset: crate::ScrollOffset::default(),
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

    /// 配置新输入样式；已有文档及富文本粘贴保留自身样式。
    pub fn typing_style_policy(mut self, policy: RichTypingStylePolicy) -> Self {
        self.typing_style_policy = policy;
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

    pub(crate) fn caret_revision(&self) -> u64 {
        self.state.caret_revision
    }

    pub fn can_undo(&self) -> bool {
        self.editable() && !self.state.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        self.editable() && !self.state.redo.is_empty()
    }

    fn range(&self) -> Range<usize> {
        self.state.anchor.min(self.state.focus)..self.state.anchor.max(self.state.focus)
    }

    /// 显示命中包含预编辑字节，编辑状态始终落在已提交文档的字符边界。
    fn document_offset(&self, display_byte: usize) -> usize {
        let range = self.range();
        let byte = match &self.state.preedit {
            Some((preedit, _)) if display_byte > range.start => {
                let end = range.start + preedit.len();
                if display_byte < end {
                    range.start
                } else {
                    range.end + display_byte - end
                }
            }
            _ => display_byte,
        };
        floor_char(self.value.text(), byte.min(self.value.text().len()))
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
        let mut style = self.input_style();
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
    /// attributes, else (under [`RichTypingStylePolicy::Inherit`]) the
    /// character before it (the one typing continues), else the one after.
    fn typing_style(&self) -> RichSpanStyle {
        if self.typing_style_policy == RichTypingStylePolicy::Plain {
            return self.state.typing.clone().unwrap_or_default();
        }
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

    /// 将选区观察样式与新输入样式分开，保持工具栏状态准确。
    fn input_style(&self) -> RichSpanStyle {
        match self.typing_style_policy {
            RichTypingStylePolicy::Inherit => self.typing_style(),
            RichTypingStylePolicy::Plain => RichSpanStyle::default(),
        }
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
        self.state.preferred_x = None;
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
        let style = typed.then(|| self.input_style());
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
            RichEditCommand::SetParagraphAlignment(align) => {
                if !self.editable()
                    || self
                        .value
                        .selected_paragraphs(self.range())
                        .all(|i| self.value.paragraph_alignment(i) == align)
                {
                    return false;
                }
                self.remember(false);
                self.value.set_paragraph_alignments(self.range(), align);
                true
            }
            RichEditCommand::ClearAttrs(mask) => {
                if !self.editable() {
                    return false;
                }
                let range = self.range();
                if range.is_empty() {
                    self.state.typing = Some(mask.cleared(self.typing_style()));
                } else {
                    self.remember(false);
                    let piece = self.value.slice(range.clone());
                    for (span, style) in piece.spans().iter() {
                        self.value.set_span(
                            span.start + range.start..span.end + range.start,
                            mask.cleared(style.clone()),
                        );
                    }
                }
                true
            }
            RichEditCommand::ClearFormatting => {
                let range = self.range();
                if !self.editable() {
                    return false;
                }
                self.remember(false);
                self.value.set_paragraph_alignments(range.clone(), None);
                if range.is_empty() {
                    self.state.typing = Some(RichSpanStyle::default());
                    return true;
                }
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
        let paragraphs = self.value.selected_paragraphs(self.range());
        let alignment = self.value.paragraph_alignment(paragraphs.start);
        let mixed = paragraphs
            .clone()
            .any(|i| self.value.paragraph_alignment(i) != alignment);
        let range = self.range();
        if range.is_empty() {
            return RichSelectionAttrs {
                style: self.typing_style(),
                mixed: RichAttrMask::default(),
                paragraph_alignment: alignment,
                paragraph_alignment_mixed: mixed,
            };
        }
        let mut attrs = summarize(&self.value, range);
        attrs.paragraph_alignment = alignment;
        attrs.paragraph_alignment_mixed = mixed;
        attrs
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
                (
                    piece.shape.family_alias.as_ref(),
                    piece.shape.family.as_ref(),
                ) != (
                    first.shape.family_alias.as_ref(),
                    first.shape.family.as_ref(),
                ),
                RichAttrMask::FAMILY,
            ),
            (
                (piece.shape.size_px, piece.shape.size_em)
                    != (first.shape.size_px, first.shape.size_em),
                RichAttrMask::SIZE,
            ),
            (
                piece.shape.weight != first.shape.weight,
                RichAttrMask::WEIGHT,
            ),
            (
                piece.shape.stretch_percent != first.shape.stretch_percent,
                RichAttrMask::STRETCH,
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
                piece.paint.background_color != first.paint.background_color,
                RichAttrMask::BACKGROUND_COLOR,
            ),
            (
                piece.shape.stretch_percent != first.shape.stretch_percent,
                RichAttrMask::STRETCH,
            ),
            (
                piece.paint.emphasis != first.paint.emphasis,
                RichAttrMask::EMPHASIS,
            ),
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
            (piece.emphasis != first.emphasis, RichAttrMask::EMPHASIS),
            (
                piece.paint.emphasis_color != first.paint.emphasis_color,
                RichAttrMask::EMPHASIS_COLOR,
            ),
        ];
        for (differs, bit) in checks {
            if differs {
                mixed |= bit.0;
            }
        }
    }
    let mask = RichAttrMask(mixed);
    RichSelectionAttrs {
        style: mask.cleared(first),
        mixed: mask,
        ..Default::default()
    }
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

fn selection_unit(text: &str, byte: usize, count: u8) -> Range<usize> {
    let byte = floor_char(text, byte.min(text.len()));
    if count == 3 {
        let start = text[..byte].rfind('\n').map_or(0, |i| i + 1);
        let end = text[byte..].find('\n').map_or(text.len(), |i| byte + i + 1);
        return start..end;
    }
    if count == 2 {
        return text
            .split_word_bound_indices()
            .find_map(|(start, word)| {
                (byte >= start && byte < start + word.len()).then_some(start..start + word.len())
            })
            .unwrap_or(byte..byte);
    }
    byte..byte
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

    fn move_vertical(&self, byte: usize, x: f32, down: bool, page: Option<f32>) -> Option<usize> {
        let (_, top, height) = caret_box(&self.layout, byte)?;
        let y = if let Some(page) = page {
            top + height * 0.5 + if down { page } else { -page }
        } else {
            let line = line_of(&self.layout, byte)?;
            let target = if down { line + 1 } else { line.checked_sub(1)? };
            let line = self.layout.lines.get(target)?;
            line.metrics.top_y_px + line.metrics.height_px * 0.5
        };
        Some(self.layout.hit_test_text(&self.text, x, y).caret.byte)
    }

    fn line_edge(&self, byte: usize, end: bool) -> Option<usize> {
        let line = &self.layout.lines[line_of(&self.layout, byte)?];
        if !end {
            return Some(line.source.start);
        }
        let mut edge = floor_char(&self.text, line.source.end.min(self.text.len()));
        if self.text[..edge].ends_with('\n') && edge > line.source.start {
            edge -= 1;
        }
        Some(edge)
    }
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
            self.state.caret_revision = self.state.caret_revision.wrapping_add(1);
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
        if world.scroll_request(id) != Some(self.scroll_offset) {
            mutations.set_scroll_offset(id, self.scroll_offset);
        }
        let (display, marks) = self.display();
        if world.inline_content(id).as_ref() != Some(&crate::InlineContent::from(display.clone())) {
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
            if changed {
                view.state.caret_revision = view.state.caret_revision.wrapping_add(1);
            }
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

    /// The focused rich editor's caret revision: what restarts its blink.
    pub(crate) fn rich_editor_caret_revision(&self, document: DocumentId) -> u64 {
        self.focused_rich_editor(document)
            .and_then(|entity| self.read(entity, RichTextEditor::caret_revision).ok())
            .unwrap_or(0)
    }

    /// Whether `document`'s focused rich editor takes typing (and the IME).
    pub(crate) fn rich_editor_accepts_input(&self, document: DocumentId) -> bool {
        self.focused_rich_editor(document)
            .is_some_and(|entity| self.read(entity, RichTextEditor::editable).unwrap_or(false))
    }

    /// Where the focused rich editor's caret shows in the window: what the
    /// IME anchors its candidates to. An edit is routed before the frame
    /// that lays it out, and the layout it replaces cannot place the new
    /// caret; the editor's content box stands in until then, as the field
    /// does for a text input without caret geometry. The IME never loses
    /// its anchor while typing: a session that loses one is torn down.
    pub(crate) fn rich_editor_caret_bounds(
        &self,
        document: DocumentId,
    ) -> Option<crate::LayoutBox> {
        let entity = self.focused_rich_editor(document)?;
        let id = entity.stable_id();
        let (content, scroll) = self.world().document_text_pointer_context(id)?;
        let caret = self.read(entity, |view| view.display().1.caret).ok()??;
        let placed = self
            .world()
            .text_layout(id)
            .and_then(|(_, layout)| caret_box(layout, caret));
        let bounds = match placed {
            Some((x, top, height)) => crate::LayoutBox {
                x: content.x + x - scroll.x,
                y: content.y + top - scroll.y,
                width: nana_ui_core::HAIRLINE,
                height,
            },
            None => content,
        };
        Some(
            self.world()
                .project_input_bounds(id, bounds)
                .unwrap_or(bounds),
        )
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
        if primary && (!shift || key.eq_ignore_ascii_case("z")) {
            let lower = key.to_ascii_lowercase();
            match lower.as_str() {
                "z" if shift => return self.rich_edit(entity, RichEditCommand::Redo).map(Some),
                "z" => return self.rich_edit(entity, RichEditCommand::Undo).map(Some),
                "y" => return self.rich_edit(entity, RichEditCommand::Redo).map(Some),
                "a" => return self.rich_edit(entity, RichEditCommand::SelectAll).map(Some),
                "c" | "x" | "v" => {
                    let command = match lower.as_str() {
                        "c" => RichClipboardCommand::Copy,
                        "x" => RichClipboardCommand::Cut,
                        _ => RichClipboardCommand::Paste,
                    };
                    return self.rich_clipboard(entity, command, services).map(Some);
                }
                _ => {}
            }
        }
        let geometry = Geometry::of(self.world(), entity.stable_id());
        let page_height = self
            .world()
            .component_content_box(entity.stable_id())
            .map_or(100.0, |content| content.height);
        let handled = match key {
            "ArrowLeft" | "ArrowRight" | "ArrowUp" | "ArrowDown" | "Home" | "End" | "PageUp"
            | "PageDown" => self.edit_with(entity, |view, _| {
                let text = view.value.text().to_owned();
                let range = view.range();
                let focus = view.state.focus;
                let collapsed = range.is_empty();
                let vertical = matches!(key, "ArrowUp" | "ArrowDown" | "PageUp" | "PageDown");
                let x = view
                    .state
                    .preferred_x
                    .or_else(|| {
                        geometry
                            .as_ref()
                            .and_then(|g| caret_box(&g.layout, focus).map(|c| c.0))
                    })
                    .unwrap_or(0.0);
                view.state.preferred_x = vertical.then_some(x);
                let target = match key {
                    "ArrowLeft" if primary => previous_word(&text, focus),
                    "ArrowRight" if primary => next_word(&text, focus),
                    "ArrowLeft" if !shift && !collapsed => range.start,
                    "ArrowRight" if !shift && !collapsed => range.end,
                    "ArrowLeft" => previous_grapheme(&text, focus),
                    "ArrowRight" => next_grapheme(&text, focus),
                    "Home" if primary => 0,
                    "End" if primary => text.len(),
                    "ArrowUp" if primary => {
                        let start = text[..focus].rfind('\n').map_or(0, |i| i + 1);
                        if start < focus {
                            start
                        } else {
                            text[..start.saturating_sub(1)]
                                .rfind('\n')
                                .map_or(0, |i| i + 1)
                        }
                    }
                    "ArrowDown" if primary => text[focus..]
                        .find('\n')
                        .map_or(text.len(), |i| focus + i + 1),
                    "ArrowUp" | "ArrowDown" | "PageUp" | "PageDown" => geometry
                        .as_ref()
                        .filter(|geometry| *geometry.text == *text)
                        .and_then(|geometry| {
                            geometry.move_vertical(
                                focus,
                                x,
                                matches!(key, "ArrowDown" | "PageDown"),
                                matches!(key, "PageUp" | "PageDown").then_some(page_height),
                            )
                        })
                        .unwrap_or(if matches!(key, "ArrowDown" | "PageDown") {
                            text.len()
                        } else {
                            0
                        }),
                    "Home" | "End" => geometry
                        .as_ref()
                        .filter(|geometry| *geometry.text == *text)
                        .and_then(|geometry| geometry.line_edge(focus, key == "End"))
                        .unwrap_or(if key == "End" { text.len() } else { 0 }),
                    _ => focus,
                };
                let target = floor_char(&text, target.min(text.len()));
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
                view.state.coalesce = false;
                view.insert_text(text);
                view.state.coalesce = false;
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
            CompositionInput::DeleteSurrounding {
                before_bytes,
                after_bytes,
            } => view.delete_ime_surrounding(*before_bytes, *after_bytes),
            CompositionInput::Enabled => false,
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
        now: std::time::Duration,
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
                self.read(*entity, |view| {
                    view.state
                        .dragging
                        .is_some_and(|drag| drag.pointer == pointer)
                })
                .unwrap_or(false)
            })
        };
        let Some(entity) = entity else {
            return Ok(false);
        };
        let id = entity.stable_id();
        if phase >= 2 {
            self.set_rich_drag_scroll(None);
        }
        let mut scrolled = false;
        if phase == 1 {
            let (px, py) = self
                .world()
                .pointer_layout_position(id, x, y)
                .unwrap_or((x, y));
            if let Some(content) = self.world().component_content_box(id) {
                let dy = if py < content.y {
                    (py - content.y).max(-32.0)
                } else if py > content.y + content.height {
                    (py - content.y - content.height).min(32.0)
                } else {
                    0.0
                };
                let dx = if px < content.x {
                    (px - content.x).max(-32.0)
                } else if px > content.x + content.width {
                    (px - content.x - content.width).min(32.0)
                } else {
                    0.0
                };
                if (dx != 0.0 || dy != 0.0)
                    && self.scroll_node_by(id, crate::ScrollOffset { x: dx, y: dy })?
                {
                    scrolled = true;
                    self.set_rich_drag_scroll(Some((
                        document,
                        id,
                        pointer,
                        x,
                        y,
                        now + std::time::Duration::from_millis(16),
                    )));
                } else {
                    self.set_rich_drag_scroll(None);
                }
            }
        }
        let hit = self.rich_editor_hit(id, x, y);
        if phase == 0 {
            self.focus_node(document, id)?;
            self.press_pointer(document, pointer, id)?;
            let mut mutations = MutationQueue::new();
            mutations.capture_pointer(pointer, id);
            self.commit_mutations(mutations)?;
        }
        if phase >= 2 {
            self.release_pointer(document, pointer);
            self.release_pointer_capture(document, pointer);
        }
        self.edit_with(entity, |view, cx| {
            match phase {
                0 => {
                    view.state.dragging = Some(DragState {
                        pointer,
                        position: (x, y),
                    });
                    view.state.preedit = None;
                    view.state.preferred_x = None;
                    let count = match view.state.click {
                        Some((p, at, px, py, count))
                            if p == pointer
                                && now.saturating_sub(at)
                                    <= std::time::Duration::from_millis(500)
                                && (px - x).abs() < 4.0
                                && (py - y).abs() < 4.0 =>
                        {
                            count % 3 + 1
                        }
                        _ => 1,
                    };
                    view.state.click = Some((pointer, now, x, y, count));
                    view.state.drag_unit = count;
                    if let Some((byte, object)) = hit {
                        let range = selection_unit(view.value.text(), byte, count);
                        view.state.drag_range = Some(range.clone());
                        view.state.focus = range.end;
                        if !shift {
                            view.state.anchor = range.start;
                        }
                        if let Some((id, offset)) = object {
                            cx.emit(RichTextEditorEvent::ObjectActivated { id, offset });
                        }
                    }
                }
                1 => {
                    if (scrolled
                        || view
                            .state
                            .dragging
                            .is_some_and(|drag| drag.position != (x, y)))
                        && let Some((byte, _)) = hit
                    {
                        let range = selection_unit(view.value.text(), byte, view.state.drag_unit);
                        if view.state.drag_unit == 1 {
                            view.state.focus = byte;
                        } else if let Some(start) = &view.state.drag_range {
                            if byte < start.start {
                                view.state.anchor = start.end;
                                view.state.focus = range.start;
                            } else if view.state.drag_unit > 1 {
                                view.state.anchor = start.start;
                                view.state.focus = range.end;
                            } else {
                                view.state.focus = byte;
                            }
                        }
                    }
                    view.state.dragging = Some(DragState {
                        pointer,
                        position: (x, y),
                    });
                }
                2 => {
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
        self.read(Entity::<RichTextEditor>::from_stable_id(id), |view| {
            let byte = view.document_offset(byte);
            let object = object.and_then(|(id, display_offset)| {
                let offset = view.document_offset(display_offset);
                view.value
                    .object_at(offset)
                    .filter(|object| object.id == id)
                    .map(|_| (id, offset))
            });
            (byte, object)
        })
        .ok()
    }

    /// Run an edit on the editor's state and report what it changed.
    fn edit_with(
        &mut self,
        entity: Entity<RichTextEditor>,
        edit: impl FnOnce(&mut RichTextEditor, &mut crate::ViewContext<'_, RichTextEditor>) -> bool,
    ) -> Result<bool, FrameworkError> {
        self.update_component(entity, |view, cx| {
            let before = view.snapshot();
            let preedit = view.state.preedit.clone();
            let attrs = view.selection_attrs();
            let changed = edit(view, cx);
            if changed {
                view.state.caret_revision = view.state.caret_revision.wrapping_add(1);
            }
            view.report(&before, &attrs, cx);
            changed
        })
    }

    /// Position a context-menu caret without destroying an existing selection.
    pub fn prepare_rich_editor_context_menu(
        &mut self,
        entity: Entity<RichTextEditor>,
        x: f32,
        y: f32,
    ) -> Result<(), FrameworkError> {
        let byte = self
            .rich_editor_hit(entity.stable_id(), x, y)
            .map(|hit| hit.0);
        if self.read(entity, |view| view.range().is_empty())?
            && let Some(byte) = byte
        {
            self.rich_edit(entity, RichEditCommand::Select(byte..byte))?;
        }
        Ok(())
    }

    pub(crate) fn rich_drag_deadline(&self) -> Option<std::time::Duration> {
        self.rich_drag_scroll()
            .and_then(|(doc, id, pointer, _, _, deadline)| {
                (self.world().is_mounted(id)
                    && self.world().focused(doc) == Some(id)
                    && self.world().pointer_capture(doc, pointer) == Some(id))
                .then_some(deadline)
            })
    }

    pub(crate) fn advance_rich_drag(&mut self, now: std::time::Duration) -> Option<StableNodeId> {
        let (doc, id, pointer, x, y, deadline) = self.rich_drag_scroll()?;
        if self.rich_drag_deadline().is_none() {
            self.set_rich_drag_scroll(None);
            return None;
        }
        if deadline > now {
            return None;
        }
        self.rich_editor_pointer(doc, Some(id), pointer, 1, x, y, false, now)
            .ok()
            .filter(|done| *done)
            .map(|_| id)
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

    /// Both policies, with and without attributes set on the caret: only the
    /// neighbour's style depends on the policy, never what the caret was given.
    #[test]
    fn the_typing_policy_decides_only_whether_the_neighbour_style_carries() {
        let bold = RichSpanStyle::new().bold();
        let red = PaintColor::srgb([1.0, 0.0, 0.0, 1.0]);
        for policy in [RichTypingStylePolicy::Inherit, RichTypingStylePolicy::Plain] {
            let bold_ab = || {
                RichTextEditor::new(RichText::new("ab").with_span(0..2, bold.clone()))
                    .typing_style_policy(policy)
            };
            let mut view = bold_ab();
            view.apply_command(RichEditCommand::Select(2..2));
            view.insert_text("c");
            let weight = view.value.style_at(2).and_then(|style| style.shape.weight);
            match policy {
                RichTypingStylePolicy::Inherit => assert_eq!(weight, Some(700), "{policy:?}"),
                RichTypingStylePolicy::Plain => assert_eq!(weight, None, "{policy:?}"),
            }
            view.state.preedit = Some(("k".into(), 1));
            let preedit_weight = view
                .display()
                .0
                .style_at(3)
                .and_then(|style| style.shape.weight);
            assert_eq!(
                preedit_weight, weight,
                "{policy:?}: preedit commits as typed"
            );
            assert_eq!(view.selection_attrs().style.shape.weight, weight);

            let mut view = bold_ab();
            view.apply_command(RichEditCommand::Select(2..2));
            view.apply_command(RichEditCommand::SetAttrs(RichSpanStyle::new().color(red)));
            view.insert_text("c");
            let typed = view.value.style_at(2).cloned().unwrap_or_default();
            assert_eq!(typed.paint.color, Some(red), "{policy:?}: the caret's own");
            match policy {
                RichTypingStylePolicy::Inherit => assert_eq!(typed.shape.weight, Some(700)),
                RichTypingStylePolicy::Plain => assert_eq!(typed.shape.weight, None),
            }
        }
    }

    #[test]
    fn stretch_and_emphasis_selection_summary_supports_undo() {
        let style = RichSpanStyle::new().stretch(75.0).emphasis(true);
        let mut view = RichTextEditor::new(RichText::new("abcd").with_span(0..2, style.clone()));
        view.apply_command(RichEditCommand::Select(0..2));
        assert_eq!(view.selection_attrs().style, style);
        view.apply_command(RichEditCommand::Select(0..4));
        let mixed = view.selection_attrs();
        assert!(mixed.mixed.contains(RichAttrMask::STRETCH));
        assert!(mixed.mixed.contains(RichAttrMask::EMPHASIS));
        assert_eq!(mixed.style.shape.stretch_percent, None);
        assert_eq!(mixed.style.paint.emphasis, None);
        view.apply_command(RichEditCommand::SetAttrs(
            RichSpanStyle::new().stretch(125.0).emphasis(false),
        ));
        assert_eq!(
            view.selection_attrs().style.shape.stretch_percent,
            Some(125.0)
        );
        assert!(view.undo());
        assert_eq!(view.value.style_at(0), Some(&style));
        assert_eq!(view.value.style_at(3), None);
        assert!(view.redo());
        assert_eq!(view.value.style_at(3).unwrap().paint.emphasis, Some(false));
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
    fn stretch_and_background_are_their_own_toolbar_attributes() {
        let yellow = PaintColor::srgb([1.0, 0.9, 0.2, 1.0]);
        let marked = RichSpanStyle::new().stretch(75.0).background_color(yellow);
        let mut view = RichTextEditor::new(RichText::new("abcd").with_span(0..2, marked));
        view.apply_command(RichEditCommand::Select(0..4));
        let mixed = view.selection_attrs().mixed;
        assert!(mixed.contains(RichAttrMask::STRETCH));
        assert!(mixed.contains(RichAttrMask::BACKGROUND_COLOR));
        assert!(!mixed.contains(RichAttrMask::COLOR));
        view.apply_command(RichEditCommand::ClearAttrs(RichAttrMask::BACKGROUND_COLOR));
        let style = view.value.style_at(0).expect("the stretch stays");
        assert_eq!(style.shape.stretch_percent, Some(75.0));
        assert_eq!(style.paint.background_color, None);
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

#[cfg(test)]
mod typography_tests {
    use super::*;
    use nana_ui_core::TextAlignSpec;

    #[test]
    fn paragraph_selection_split_merge_and_undo() {
        let mut editor = RichTextEditor::new(RichText::new("one\ntwo\n"));
        editor.apply_command(RichEditCommand::Select(0..4));
        editor.apply_command(RichEditCommand::SetParagraphAlignment(Some(
            TextAlignSpec::Center,
        )));
        assert_eq!(
            editor.value.paragraph_alignment(0),
            Some(TextAlignSpec::Center)
        );
        assert_eq!(editor.value.paragraph_alignment(1), None);
        editor.apply_command(RichEditCommand::SelectAll);
        assert!(editor.selection_attrs().paragraph_alignment_mixed);
        editor.apply_command(RichEditCommand::Undo);
        assert_eq!(editor.value.paragraph_alignment(0), None);
        editor.apply_command(RichEditCommand::Redo);
        editor.apply_command(RichEditCommand::Select(2..2));
        editor.apply_command(RichEditCommand::InsertText("\n".into()));
        assert_eq!(
            editor.value.paragraph_alignment(1),
            Some(TextAlignSpec::Center)
        );
        editor.apply_command(RichEditCommand::Select(2..3));
        editor.delete(2..3);
        assert_eq!(
            editor.value.paragraph_alignment(0),
            Some(TextAlignSpec::Center)
        );
        editor.apply_command(RichEditCommand::Select(
            editor.value.text().len()..editor.value.text().len(),
        ));
        editor.apply_command(RichEditCommand::SetParagraphAlignment(Some(
            TextAlignSpec::End,
        )));
        assert_eq!(
            editor.value.paragraph_alignment(2),
            Some(TextAlignSpec::End)
        );
    }

    #[test]
    fn clearing_one_attribute_preserves_others_and_undo() {
        let rich = RichText::new("abc").with_span(
            0..3,
            RichSpanStyle::new()
                .bold()
                .size_em(1.5)
                .family("Body")
                .family_alias("body"),
        );
        let mut editor = RichTextEditor::new(rich.clone());
        editor.apply_command(RichEditCommand::SelectAll);
        editor.apply_command(RichEditCommand::ClearAttrs(RichAttrMask::SIZE));
        let style = editor.value.style_at(1).unwrap();
        assert_eq!(style.shape.size_em, None);
        assert_eq!(style.shape.weight, Some(700));
        assert_eq!(style.shape.family_alias.as_deref(), Some("body"));
        editor.apply_command(RichEditCommand::Undo);
        assert_eq!(editor.value, rich);
        editor.apply_command(RichEditCommand::SetAttrs(RichSpanStyle::new().size(32.0)));
        assert_eq!(editor.value.style_at(0).unwrap().shape.size_em, None);
        editor.apply_command(RichEditCommand::ClearFormatting);
        editor.apply_command(RichEditCommand::Undo);
        assert_eq!(editor.value.style_at(0).unwrap().shape.size_px, Some(32.0));
    }

    #[test]
    fn identical_resolved_fonts_and_sizes_keep_distinct_authored_values() {
        let rich = RichText::new("ab")
            .with_span(
                0..1,
                RichSpanStyle::new()
                    .family("Same")
                    .family_alias("name")
                    .size_em(1.0),
            )
            .with_span(1..2, RichSpanStyle::new().family("Same").size(28.0));
        let mut editor = RichTextEditor::new(rich);
        editor.apply_command(RichEditCommand::SelectAll);
        let attrs = editor.selection_attrs();
        assert!(attrs.mixed.contains(RichAttrMask::SIZE));
        assert!(attrs.mixed.contains(RichAttrMask::FAMILY));
    }
}
