//! Retained text state per node, and the dirty graph that decides what a
//! mutation costs the text pipeline (Issue #95).
//!
//! A text node does not carry a single `dirty = true`. Every mutation that can
//! touch text names *what* changed as a [`TextDirty`] class, and
//! [`TextDirty::work`] is the one place that says which text work that class
//! implies:
//!
//! ```text
//! CONTENT / FONT / SHAPE_STYLE -> shape + layout + scene text geometry
//! CONSTRAINT                    -> layout + scene text geometry
//! EDIT_STATE                    -> editor overlay / caret
//! PAINT                         -> scene paint only
//! TRANSFORM / OPACITY           -> compositor only
//! GLYPH_PRESENTATION            -> compositor only (per-glyph effects)
//! ```
//!
//! [`TextNodeState::invalidate`] turns a class into revision bumps, and a
//! resolved node remembers the revisions it was resolved at
//! ([`TextStamp`]). Deciding that a node needs no text work is therefore a
//! comparison of a few integers — before its text is cloned, hashed or looked
//! up — and a paint or compositor class cannot reach a shaping or layout
//! revision because it does not bump one.
//!
//! Today the text passes consume the shape and layout half of the graph.
//! Scene extraction still re-extracts a render-dirty node whole, so the paint,
//! geometry, overlay and compositor work classes (and the `paint` / `edit`
//! revisions) are the contract a renderer drawing retained layouts (#97) and
//! the editable path (#96) key on, not yet a finer extraction.

use std::sync::Arc;

use nana_text::{
    TextConstraints as NanaTextConstraints, TextEngineEpoch, TextKind, TextLayout, TextLayoutId,
    TextSource, TextSpan, TextStyle as NanaTextStyle,
};
use nana_ui_core::{
    AttributedRanges, LineHeightSpec, RichShapeStyle, RichSpanStyle, RichText, TextAlignSpec,
};

use crate::{ComputedStyle, TextHorizontalAlignment, TextMetrics, TextShapeConstraints};

/// What changed about a text node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TextDirty(u16);

impl TextDirty {
    pub const NONE: Self = Self(0);
    /// The authored text.
    pub const CONTENT: Self = Self(1 << 0);
    /// The font set the text resolves against: a face registered, replaced or
    /// removed, or a fallback policy change.
    pub const FONT: Self = Self(1 << 1);
    /// A style input to shaping: family, size, weight, slant, letter spacing,
    /// features, variation axes, kerning, direction.
    pub const SHAPE_STYLE: Self = Self(1 << 2);
    /// An input to line layout only: the container box, wrapping, clamping,
    /// alignment, line height, word/line breaking, writing mode.
    pub const CONSTRAINT: Self = Self(1 << 3);
    /// Caret, selection or IME composition of an editable node.
    pub const EDIT_STATE: Self = Self(1 << 4);
    /// Colour, text shadow, decoration colour, selection colour.
    pub const PAINT: Self = Self(1 << 5);
    /// The node's transform.
    pub const TRANSFORM: Self = Self(1 << 6);
    /// The node's opacity.
    pub const OPACITY: Self = Self(1 << 7);
    /// Per-glyph presentation of rich text: which effect a span plays, and
    /// when its glyphs reveal. Sampled by the compositor every frame; like
    /// transform and opacity it never reaches a shaping, layout or paint
    /// revision, so a span that only changes its effect costs no text work.
    pub const GLYPH_PRESENTATION: Self = Self(1 << 8);
    /// The language the text shapes in (`locl` forms, fallback faces): its
    /// own, its subtree's, the application's or the engine's.
    pub const LANGUAGE: Self = Self(1 << 9);

    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    /// The classes both name.
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The text work this class implies. The whole dependency graph.
    pub const fn work(self) -> TextWork {
        let mut work = TextWork::NONE.0;
        if self.intersects(
            Self::CONTENT
                .union(Self::FONT)
                .union(Self::SHAPE_STYLE)
                .union(Self::LANGUAGE),
        ) {
            work |= TextWork::SHAPE.0 | TextWork::LAYOUT.0 | TextWork::SCENE_GEOMETRY.0;
        }
        if self.intersects(Self::CONSTRAINT) {
            work |= TextWork::LAYOUT.0 | TextWork::SCENE_GEOMETRY.0;
        }
        if self.intersects(Self::EDIT_STATE) {
            work |= TextWork::EDITOR_OVERLAY.0;
        }
        if self.intersects(Self::PAINT) {
            work |= TextWork::SCENE_PAINT.0;
        }
        if self.intersects(
            Self::TRANSFORM
                .union(Self::OPACITY)
                .union(Self::GLYPH_PRESENTATION),
        ) {
            work |= TextWork::COMPOSITOR.0;
        }
        TextWork(work)
    }
}

impl std::ops::BitOr for TextDirty {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        self.union(other)
    }
}

impl std::ops::BitOrAssign for TextDirty {
    fn bitor_assign(&mut self, other: Self) {
        *self = self.union(other);
    }
}

/// Work a [`TextDirty`] class implies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct TextWork(u16);

impl TextWork {
    pub const NONE: Self = Self(0);
    /// Shape the text again (a shape cache may still answer it).
    pub const SHAPE: Self = Self(1 << 0);
    /// Lay the shaped runs out again.
    pub const LAYOUT: Self = Self(1 << 1);
    /// Re-extract the node's text geometry into the scene.
    pub const SCENE_GEOMETRY: Self = Self(1 << 2);
    /// Rebuild the editor overlay: caret, selection, composition underline.
    pub const EDITOR_OVERLAY: Self = Self(1 << 3);
    /// Re-extract the node's text paint (colour) only.
    pub const SCENE_PAINT: Self = Self(1 << 4);
    /// Compositor presentation only.
    pub const COMPOSITOR: Self = Self(1 << 5);

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

/// Per-node revisions, one per class that can make resolved text stale.
///
/// There is deliberately no transform or opacity revision: those classes
/// imply no text work, so nothing a resolved stamp compares can move.
///
/// `u32` keeps the retained record small: the per-node "no text work" check
/// runs over every candidate of a scope, and it is the record's size, not the
/// comparison, that such a scan pays for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TextRevisions {
    pub content: u32,
    pub shape: u32,
    pub constraint: u32,
    pub paint: u32,
    pub edit: u32,
}

/// Which backend a node was resolved by, and against which font set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextBackendEpoch {
    /// The host's [`crate::TextShaper::shape`], by shaper type, at the host's
    /// font generation. Another shaper is another measurement. The type is
    /// kept as a hash of its name, read once per pass: every candidate
    /// compares this epoch, and a name would be a string comparison each.
    Host { shaper: u64, font_generation: u64 },
    /// A `nana-text` engine.
    Engine(TextEngineEpoch),
}

/// A plain text node's retained `nana-text` layout as extraction hands it to
/// the scene: the node's generational handle, and the immutable layout it
/// named when extracted.
///
/// Renderer-neutral IR, not a shaping backend's buffer. Two values are equal
/// only when they are the same handle to the same layout, so a relayout is a
/// scene change and an unchanged layout is not.
#[derive(Debug, Clone)]
pub struct RetainedTextLayout {
    pub id: TextLayoutId,
    pub layout: Arc<TextLayout>,
}

impl PartialEq for RetainedTextLayout {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && Arc::ptr_eq(&self.layout, &other.layout)
    }
}

/// The revisions and backend a node's current metrics (and layout) were
/// resolved at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct TextStamp {
    content: u32,
    shape: u32,
    constraint: u32,
    backend: TextBackendEpoch,
    /// Whether the node counts as a text node (it has text, or is a `Text`
    /// node) when resolved. The content revision is part of the stamp, so
    /// while the stamp is current this is still true.
    pub text_node: bool,
    /// Whether the node's text was measured, even as an empty line box, as
    /// opposed to a box without text resolved to nothing. Only a measured node
    /// can hold metrics a new font set changes.
    pub measured: bool,
    /// The constraints and alignment the revisions stood for, for a measured
    /// node. Only compared by the debug check that a revision bump was not
    /// missed.
    #[cfg(debug_assertions)]
    pub constraints: Option<(TextShapeConstraints, TextHorizontalAlignment)>,
}

/// Retained text state of one node.
#[derive(Debug, Clone, Default)]
pub(crate) struct TextNodeState {
    pub revisions: TextRevisions,
    /// The shared source a `nana-text` engine lays out, and the revisions it
    /// was built at: the content revision, and — for rich text whose spans
    /// resolve shaping fields over the node's computed style, or that holds
    /// inline objects — the shape and constraint revisions (zero otherwise).
    /// Built once per such key, only for a node an engine actually resolves;
    /// boxed so a node that never is pays a pointer, not a source.
    source: Option<Box<(u32, u32, TextSource)>>,
    stamp: Option<TextStamp>,
    /// The layout this node retains in its world's layout store, or null.
    pub layout: TextLayoutId,
    /// The language the node last resolved in, and whether a language change
    /// invalidated it since: together they say whether that change was one
    /// the text depends on.
    shaped_language: Option<nana_text::font::LanguageTag>,
    language_pending: bool,
    /// Whether a typography scale change reached this text since it last
    /// resolved: the pass that lays it out again counts that as the scale's.
    scale_pending: bool,
}

impl TextNodeState {
    /// Whether a pass has resolved this node before: a later resolution is a
    /// change to text that already exported metrics, not its first layout.
    pub(crate) fn resolved_before(&self) -> bool {
        self.stamp.is_some()
    }

    /// A typography scale change reached this text, if it had resolved
    /// before: text laid out for the first time under a scale is not one a
    /// change reached. Returns whether it had.
    pub(crate) fn note_scale(&mut self) -> bool {
        let resolved = self.resolved_before();
        self.scale_pending |= resolved;
        resolved
    }

    /// Whether a typography scale change reached this text since it last
    /// resolved, clearing it.
    pub(crate) fn take_scale_pending(&mut self) -> bool {
        std::mem::take(&mut self.scale_pending)
    }

    /// Applies `dirty` as revision bumps, through [`TextDirty::work`]: a
    /// revision moves exactly when its work is implied. Classes that imply no
    /// text work bump nothing, so they cannot make a resolved node stale.
    pub fn invalidate(&mut self, dirty: TextDirty) {
        if dirty.intersects(TextDirty::LANGUAGE) {
            self.language_pending = true;
        }
        let work = dirty.work();
        let revisions = &mut self.revisions;
        if dirty.intersects(TextDirty::CONTENT) {
            revisions.content = revisions.content.wrapping_add(1);
        }
        if work.intersects(TextWork::SHAPE) {
            revisions.shape = revisions.shape.wrapping_add(1);
        }
        if work.intersects(TextWork::LAYOUT) {
            revisions.constraint = revisions.constraint.wrapping_add(1);
        }
        if work.intersects(TextWork::SCENE_PAINT) {
            revisions.paint = revisions.paint.wrapping_add(1);
        }
        if work.intersects(TextWork::EDITOR_OVERLAY) {
            revisions.edit = revisions.edit.wrapping_add(1);
        }
    }

    /// True when only the constraint revision moved since the stamp.
    pub(crate) fn constraint_moved_without_reshaping(&self) -> bool {
        self.stamp.is_some_and(|stamp| {
            stamp.content == self.revisions.content
                && stamp.shape == self.revisions.shape
                && stamp.constraint != self.revisions.constraint
        })
    }

    /// True when the node resolved before and shapes again now: its content,
    /// font, shaping style or language moved since the stamp, not only its
    /// box.
    pub(crate) fn reshapes(&self) -> bool {
        self.stamp
            .is_some_and(|stamp| stamp.shape != self.revisions.shape)
    }

    /// True when the node was resolved by `backend` at its current content,
    /// shape and constraint revisions. O(1), reads no text.
    pub fn is_current(&self, backend: TextBackendEpoch) -> bool {
        self.stamp.is_some_and(|stamp| {
            stamp.backend == backend
                && stamp.content == self.revisions.content
                && stamp.shape == self.revisions.shape
                && stamp.constraint == self.revisions.constraint
        })
    }

    pub fn stamp(&self) -> Option<&TextStamp> {
        self.stamp.as_ref()
    }

    /// Records that the node was resolved at its current revisions.
    ///
    /// Returns whether a language change had invalidated it although it
    /// resolved in the language it had before: work a language dependency
    /// that coarse costs.
    pub fn mark_resolved(
        &mut self,
        backend: TextBackendEpoch,
        #[cfg_attr(not(debug_assertions), allow(unused_variables))] constraints: Option<(
            TextShapeConstraints,
            TextHorizontalAlignment,
        )>,
        text_node: bool,
        language: Option<&nana_text::font::LanguageTag>,
    ) -> bool {
        let wasted_language = std::mem::take(&mut self.language_pending)
            && self.stamp.is_some()
            && self.shaped_language.as_ref() == language;
        self.shaped_language = language.cloned();
        let measured = constraints.is_some();
        self.stamp = Some(TextStamp {
            content: self.revisions.content,
            shape: self.revisions.shape,
            constraint: self.revisions.constraint,
            backend,
            text_node,
            measured,
            #[cfg(debug_assertions)]
            constraints,
        });
        wasted_language
    }

    /// Drops the copy of the text built for the engine.
    pub fn release_source(&mut self) {
        self.source = None;
    }

    /// The source for the node's current content, building it only when the
    /// content revision moved since the last build. Returns whether it copied.
    ///
    /// `rich` is the node's application-owned rich text, when it has one and
    /// it still holds `text`: its shaping tier becomes the source's spans,
    /// each a full `nana-text` style resolved over `base` — so a theme change
    /// that moves the node's own font restyles every span that does not
    /// override it. Such a source is also rebuilt when the shape revision
    /// moves. Paint and presentation fields never reach the source.
    pub fn source_for(
        &mut self,
        text: &str,
        rich: Option<&RichText>,
        base: &ComputedStyle,
    ) -> (&TextSource, bool) {
        let shaped = rich.filter(|rich| rich.text() == text).filter(|rich| {
            !rich.objects().is_empty()
                || !rich.rubies().is_empty()
                || rich
                    .spans()
                    .iter()
                    .any(|(_, style)| !style.shape.is_empty())
        });
        let revision = self.revisions.content;
        let shape = match shaped {
            // An object's box is a layout input, so a source holding objects
            // is rebuilt when the constraint revision moves too.
            Some(rich) if !rich.objects().is_empty() => {
                (self.revisions.shape ^ self.revisions.constraint.rotate_left(16)) | 1 << 31
            }
            // Never zero, so a rich source is never mistaken for a plain one.
            Some(_) => self.revisions.shape | 1 << 31,
            None => 0,
        };
        let stale = self
            .source
            .as_ref()
            .is_none_or(|built| built.0 != revision || built.1 != shape);
        if stale {
            let mut source = TextSource::new(text);
            if let Some(rich) = shaped {
                source.set_spans(rich_text_spans(
                    rich,
                    &nana_text_style(base),
                    base.font_size,
                ));
                if !rich.objects().is_empty() {
                    source.set_objects(rich_text_objects(rich));
                    source.set_labels(
                        rich.objects()
                            .iter()
                            .filter_map(|(offset, object)| {
                                Some(nana_text::ObjectLabel {
                                    offset: *offset,
                                    text: Arc::clone(object.tag_label()?),
                                })
                            })
                            .collect(),
                    );
                }
                if !rich.rubies().is_empty() {
                    source.set_rubies(
                        rich.rubies()
                            .iter()
                            .map(|(range, text)| nana_text::RubySpan {
                                range: range.clone(),
                                text: Arc::clone(text),
                            })
                            .collect(),
                    );
                }
            }
            self.source = Some(Box::new((revision, shape, source)));
        }
        let (_, _, source) = self.source.as_deref().expect("built above");
        (source, stale)
    }
}

/// The `nana-text` spans of a rich text's shaping tier, each resolved over
/// `base` (the node's own style). Paint and presentation fields are not read.
pub(crate) fn rich_text_spans(
    rich: &RichText,
    base: &NanaTextStyle,
    base_size: f32,
) -> Vec<TextSpan> {
    rich.spans()
        .iter()
        .filter(|(_, style)| !style.shape.is_empty())
        .map(|(range, style)| TextSpan {
            range,
            style: shape_over(base, base_size, &style.shape),
            composition: None,
        })
        .collect()
}

/// A rich text's inline objects as `nana-text` objects: the box each takes on
/// its line. Editor-only chips take none.
pub(crate) fn rich_text_objects(rich: &RichText) -> Vec<nana_text::InlineObject> {
    rich.objects()
        .iter()
        .map(|(offset, object)| {
            let [width_px, ascent_px, descent_px] = object.line_box();
            nana_text::InlineObject {
                offset: *offset,
                id: object.id,
                metrics: nana_text::InlineObjectMetrics {
                    width_px,
                    ascent_px,
                    descent_px,
                },
            }
        })
        .collect()
}

/// `shape` laid over a node's resolved `base` style.
///
/// A span that changes the size keeps the node's line-height *ratio*: an
/// absolute line height of 20px on 16px text becomes 1.25 times the span's
/// size. Rich text is read as one paragraph whose emphasised words may be
/// larger, and a line that holds a larger word grows to hold it.
fn shape_over(base: &NanaTextStyle, base_size: f32, shape: &RichShapeStyle) -> NanaTextStyle {
    let mut style = base.clone();
    if let Some(family) = &shape.family {
        style.font_family = Some(nana_font_family(family));
    }
    if let Some(size) = shape.size_px.filter(|size| size.is_finite() && *size > 0.0) {
        style.font_size_px = size;
        if let Some(LineHeightSpec::Absolute(height)) = style.line_height
            && base_size > 0.0
        {
            style.line_height = Some(LineHeightSpec::Relative(height / base_size));
        }
    }
    if let Some(weight) = shape.weight {
        style.font_weight = weight;
    }
    if let Some(italic) = shape.italic {
        style.italic = italic;
    }
    if let Some(spacing) = shape
        .letter_spacing_px
        .filter(|spacing| spacing.is_finite())
    {
        style.letter_spacing_px = spacing;
    }
    if let Some(features) = &shape.features {
        style.features = features.to_vec();
    }
    style
}

/// What changed between two rich texts on one node, by tier: the text
/// itself, the shaping tier, the paint tier, the presentation tier.
///
/// Each tier is compared as its own normalized projection, so restyling a
/// word's colour is `PAINT` however the spans were cut, and changing only an
/// effect index is `GLYPH_PRESENTATION` — no shaping, layout or paint work.
/// `None` is plain text: no spans.
pub(crate) fn classify_rich_change(
    previous: Option<&RichText>,
    next: Option<&RichText>,
) -> TextDirty {
    if previous == next {
        return TextDirty::NONE;
    }
    let empty = AttributedRanges::<RichSpanStyle>::default();
    let before = previous.map_or(&empty, RichText::spans);
    let after = next.map_or(&empty, RichText::spans);
    let mut dirty = TextDirty::NONE;
    if let (Some(before), Some(after)) = (previous, next)
        && before.text() != after.text()
    {
        dirty |= TextDirty::CONTENT;
    }
    let shape = |style: &RichSpanStyle| (!style.shape.is_empty()).then(|| style.shape.clone());
    if before.map(shape) != after.map(shape)
        || previous.map_or(&[][..], RichText::rubies) != next.map_or(&[][..], RichText::rubies)
    {
        // An annotation is shaped, and holds its base together on a line.
        dirty |= TextDirty::SHAPE_STYLE;
    }
    let paint = |style: &RichSpanStyle| (!style.paint.is_empty()).then(|| style.paint.clone());
    if before.map(paint) != after.map(paint) {
        dirty |= TextDirty::PAINT;
    }
    let effect = |style: &RichSpanStyle| style.effect;
    if before.map(effect) != after.map(effect) {
        dirty |= TextDirty::GLYPH_PRESENTATION;
    }
    // Objects at the same offsets (the text is the same): a new box relays
    // the paragraph out; new content only repaints it.
    let before = previous.map_or(&[][..], RichText::objects);
    let after = next.map_or(&[][..], RichText::objects);
    if before.len() != after.len()
        || before
            .iter()
            .zip(after)
            .any(|((at, old), (to, new))| at != to || old.line_box() != new.line_box())
    {
        dirty |= TextDirty::CONSTRAINT;
    }
    if before
        .iter()
        .zip(after)
        .any(|((_, old), (_, new))| old.id != new.id || old.content != new.content)
    {
        dirty |= TextDirty::PAINT;
    }
    dirty
}

/// The style inputs to shaping and layout that a [`ComputedStyle`] change can
/// move, classified. Paint and presentation fields map to their own classes;
/// fields no text work reads map to nothing.
pub(crate) fn classify_computed_style_change(
    previous: &ComputedStyle,
    next: &ComputedStyle,
) -> TextDirty {
    let mut dirty = TextDirty::NONE;
    if previous.font_size.to_bits() != next.font_size.to_bits()
        || previous.font_weight != next.font_weight
        || previous.italic != next.italic
        || previous.font_family != next.font_family
        || previous.letter_spacing.to_bits() != next.letter_spacing.to_bits()
        || previous.font_features != next.font_features
        || previous.font_variations != next.font_variations
        || previous.font_kerning != next.font_kerning
        || previous.direction != next.direction
        || previous.text_orientation != next.text_orientation
    {
        dirty |= TextDirty::SHAPE_STYLE;
    }
    if previous.language != next.language {
        dirty |= TextDirty::LANGUAGE;
    }
    if previous.line_height != next.line_height
        || previous.word_break != next.word_break
        || previous.line_break != next.line_break
        || previous.writing_mode != next.writing_mode
    {
        dirty |= TextDirty::CONSTRAINT;
    }
    if previous.color != next.color
        || previous.foreground != next.foreground
        || previous.selection_background != next.selection_background
        || previous.selection_color != next.selection_color
        || previous.paint_colors.color != next.paint_colors.color
        || previous.paint_colors.selection_background != next.paint_colors.selection_background
        || previous.paint_colors.selection_color != next.paint_colors.selection_color
    {
        dirty |= TextDirty::PAINT;
    }
    if previous.opacity.to_bits() != next.opacity.to_bits() {
        dirty |= TextDirty::OPACITY;
    }
    dirty
}

/// The `nana-text` kind a plain text node lays out as.
pub(crate) fn text_kind(constraints: &TextShapeConstraints) -> TextKind {
    if constraints.wrap || constraints.max_lines.is_some() || constraints.preserve_lines {
        TextKind::Paragraph
    } else {
        TextKind::Label
    }
}

/// The line height the host shaper has always used when none is declared.
const DEFAULT_LINE_HEIGHT: LineHeightSpec = LineHeightSpec::Relative(1.2);

/// A resolved [`ComputedStyle`] as a `nana-text` base style.
pub(crate) fn nana_text_style(style: &ComputedStyle) -> NanaTextStyle {
    NanaTextStyle {
        font_family: style.font_family.as_deref().map(nana_font_family),
        font_size_px: style.font_size.max(f32::MIN_POSITIVE),
        font_weight: style.font_weight.unwrap_or(400),
        italic: style.italic,
        line_height: Some(style.line_height.unwrap_or(DEFAULT_LINE_HEIGHT)),
        letter_spacing_px: if style.letter_spacing.is_finite() {
            style.letter_spacing
        } else {
            0.0
        },
        features: style.font_features.clone(),
        variations: style.font_variations.clone(),
        kerning: style.font_kerning,
        language: style.language.clone(),
    }
}

/// A family list with the generic the host shaper has always implied: a named
/// family that says `mono` falls back to `monospace`, anything else to
/// `sans-serif` (which `nana-text` appends itself).
///
/// Public because the *painter* has to ask the engine for the same layout this
/// node was measured with, and it builds its style from the scene rather than
/// from `ComputedStyle`. Two spellings of this rule would be two font
/// selections for one node.
pub fn nana_font_family(family: &str) -> Arc<str> {
    let lowered = family.to_ascii_lowercase();
    let has_generic = lowered.split(',').any(|name| {
        matches!(
            name.trim().trim_matches(|c| c == '"' || c == '\''),
            "serif" | "sans-serif" | "monospace" | "cursive" | "fantasy" | "system-ui"
        )
    });
    if !has_generic && lowered.contains("mono") {
        Arc::from(format!("{family}, monospace"))
    } else {
        Arc::from(family)
    }
}

/// A plain text node's shape constraints and authored alignment as
/// `nana-text` layout constraints.
pub(crate) fn nana_text_constraints(
    style: &ComputedStyle,
    constraints: &TextShapeConstraints,
    alignment: TextHorizontalAlignment,
) -> NanaTextConstraints {
    let mut nana = NanaTextConstraints {
        max_width_px: constraints.max_width,
        max_height_px: constraints.max_height,
        wrap: constraints.wrap.then_some(constraints.wrap_break),
        word_break: style.word_break,
        line_break: style.line_break,
        max_lines: constraints.max_lines,
        ellipsis: constraints.ellipsis,
        preserve_lines: constraints.preserve_lines,
        // The used direction: `text-orientation: upright` reads `ltr`.
        base_direction: style.writing_context().direction,
        align: match alignment {
            TextHorizontalAlignment::Start => TextAlignSpec::Start,
            TextHorizontalAlignment::Center => TextAlignSpec::Center,
            TextHorizontalAlignment::End => TextAlignSpec::End,
        },
        writing_mode: style.writing_mode,
        text_orientation: style.text_orientation,
        ..NanaTextConstraints::default()
    };
    // The box dimension lines *stack* along — the height, or the width of a
    // vertical paragraph — is a *truncation* budget: `nana-text` drops the
    // lines that do not fit it. A box too short for its text is an overflow,
    // not a shorter paragraph — CSS clips it at paint and the text is still
    // there to select and to hit-test. So the engine only gets it when
    // truncation was actually asked for; `max_lines` travels on its own and
    // clamps either way.
    let vertical = nana.wants_vertical_writing();
    let stacking = if vertical {
        &mut nana.max_width_px
    } else {
        &mut nana.max_height_px
    };
    if !constraints.ellipsis {
        *stacking = None;
    }
    nana
}

/// The Runtime metrics contract read off a layout: the page size of its lines
/// ([`TextLayout::physical_size`] — the widest line and the summed line boxes,
/// crossed over for vertical text), and the first line's ascent.
///
/// A vertical layout reports no ascent: its lines hang from a central
/// baseline, and handing that to a box layout that aligns alphabetic
/// baselines would put a column's midpoint on a horizontal neighbour's text
/// line.
pub(crate) fn text_metrics_of_layout(layout: &TextLayout) -> TextMetrics {
    let (width, height) = layout.physical_size();
    let ascent = layout
        .lines
        .first()
        .filter(|_| !layout.is_vertical())
        .map(|line| (line.metrics.baseline_y_px - line.metrics.top_y_px).max(0.0))
        .filter(|ascent| ascent.is_finite());
    TextMetrics {
        width,
        height,
        ascent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [TextDirty; 10] = [
        TextDirty::CONTENT,
        TextDirty::FONT,
        TextDirty::SHAPE_STYLE,
        TextDirty::LANGUAGE,
        TextDirty::CONSTRAINT,
        TextDirty::EDIT_STATE,
        TextDirty::PAINT,
        TextDirty::TRANSFORM,
        TextDirty::OPACITY,
        TextDirty::GLYPH_PRESENTATION,
    ];

    #[test]
    fn the_dependency_graph_is_the_one_the_issue_draws() {
        let shape_class = TextWork::SHAPE;
        for class in [
            TextDirty::CONTENT,
            TextDirty::FONT,
            TextDirty::SHAPE_STYLE,
            TextDirty::LANGUAGE,
        ] {
            let work = class.work();
            assert!(work.contains(shape_class), "{class:?} must reshape");
            assert!(work.contains(TextWork::LAYOUT), "{class:?} must relayout");
            assert!(work.contains(TextWork::SCENE_GEOMETRY));
            assert!(!work.intersects(TextWork::COMPOSITOR));
        }
        let constraint = TextDirty::CONSTRAINT.work();
        assert!(constraint.contains(TextWork::LAYOUT));
        assert!(constraint.contains(TextWork::SCENE_GEOMETRY));
        assert!(
            !constraint.intersects(TextWork::SHAPE),
            "a constraint change lays the existing shaped runs out again"
        );
        assert_eq!(TextDirty::EDIT_STATE.work(), TextWork::EDITOR_OVERLAY);
        assert_eq!(TextDirty::PAINT.work(), TextWork::SCENE_PAINT);
        assert_eq!(TextDirty::TRANSFORM.work(), TextWork::COMPOSITOR);
        assert_eq!(TextDirty::OPACITY.work(), TextWork::COMPOSITOR);
        assert_eq!(TextDirty::GLYPH_PRESENTATION.work(), TextWork::COMPOSITOR);
        assert!(TextDirty::NONE.work().is_empty());
    }

    #[test]
    fn only_shaping_and_layout_classes_can_make_a_resolved_node_stale() {
        let backend = TextBackendEpoch::Host {
            shaper: 7,
            font_generation: 0,
        };
        for class in ALL {
            let mut node = TextNodeState::default();
            let _ = node.mark_resolved(backend, None, true, None);
            node.invalidate(class);
            let needs_text_work = class.work().intersects(TextWork::SHAPE.union_layout());
            assert_eq!(
                !node.is_current(backend),
                needs_text_work,
                "{class:?}: staleness must follow the dependency graph"
            );
        }
    }

    #[test]
    fn paint_and_compositor_classes_never_move_a_shaping_or_layout_revision() {
        for class in [
            TextDirty::PAINT,
            TextDirty::TRANSFORM,
            TextDirty::OPACITY,
            TextDirty::EDIT_STATE,
            TextDirty::GLYPH_PRESENTATION,
        ] {
            let mut node = TextNodeState::default();
            let before = node.revisions;
            node.invalidate(class);
            assert_eq!(node.revisions.content, before.content, "{class:?}");
            assert_eq!(node.revisions.shape, before.shape, "{class:?}");
            assert_eq!(node.revisions.constraint, before.constraint, "{class:?}");
        }
    }

    #[test]
    fn a_glyph_presentation_change_moves_no_revision_at_all() {
        let mut node = TextNodeState::default();
        let before = node.revisions;
        node.invalidate(TextDirty::GLYPH_PRESENTATION);
        assert_eq!(node.revisions, before);
        assert!(TextDirty::GLYPH_PRESENTATION.0 > u8::MAX as u16);
    }

    #[test]
    fn a_new_font_set_makes_a_resolved_node_stale_without_any_revision_bump() {
        let mut node = TextNodeState::default();
        let _ = node.mark_resolved(
            TextBackendEpoch::Host {
                shaper: 7,
                font_generation: 3,
            },
            None,
            true,
            None,
        );
        assert!(node.is_current(TextBackendEpoch::Host {
            shaper: 7,
            font_generation: 3,
        }));
        assert!(!node.is_current(TextBackendEpoch::Host {
            shaper: 7,
            font_generation: 4,
        }));
    }

    #[test]
    fn the_source_is_built_once_per_content_revision() {
        let style = ComputedStyle::default();
        let mut node = TextNodeState::default();
        let (_, copied) = node.source_for("hello", None, &style);
        assert!(copied);
        let (source, copied) = node.source_for("hello", None, &style);
        assert!(!copied, "an unchanged revision reuses the built source");
        assert_eq!(source.text(), "hello");
        node.invalidate(TextDirty::CONTENT);
        let (source, copied) = node.source_for("bye", None, &style);
        assert!(copied);
        assert_eq!(source.text(), "bye");
        node.invalidate(TextDirty::SHAPE_STYLE);
        let (_, copied) = node.source_for("bye", None, &style);
        assert!(
            !copied,
            "plain text does not resolve its style into the source"
        );
    }

    #[test]
    fn a_rich_source_resolves_its_shaping_spans_over_the_node_style() {
        let style = ComputedStyle {
            font_size: 16.0,
            line_height: Some(LineHeightSpec::Absolute(20.0)),
            ..ComputedStyle::default()
        };
        let rich = RichText::new("big word")
            .with_span(0..3, RichSpanStyle::new().size(32.0).bold())
            .with_span(
                4..8,
                RichSpanStyle::new().color(nana_ui_core::PaintColor::srgb([1.0, 0.0, 0.0, 1.0])),
            );
        let mut node = TextNodeState::default();
        let (source, copied) = node.source_for("big word", Some(&rich), &style);
        assert!(copied);
        assert_eq!(
            source.spans().len(),
            1,
            "only the shaping tier reaches the source"
        );
        let span = &source.spans()[0];
        assert_eq!(span.range, 0..3);
        assert_eq!(span.style.font_size_px, 32.0);
        assert_eq!(span.style.font_weight, 700);
        assert_eq!(
            span.style.line_height,
            Some(LineHeightSpec::Relative(1.25)),
            "a larger span keeps the node's line-height ratio"
        );
        let (_, copied) = node.source_for("big word", Some(&rich), &style);
        assert!(!copied);
        node.invalidate(TextDirty::SHAPE_STYLE);
        let (_, copied) = node.source_for("big word", Some(&rich), &style);
        assert!(copied, "a node style change re-resolves the spans");
        let stale = RichText::new("other").with_span(0..2, RichSpanStyle::new().bold());
        node.invalidate(TextDirty::CONTENT);
        let (source, _) = node.source_for("big word", Some(&stale), &style);
        assert!(
            source.spans().is_empty(),
            "spans over other text are not applied"
        );
    }

    #[test]
    fn an_object_box_change_relays_out_and_a_content_change_repaints() {
        use nana_ui_core::RichObject;
        let with = |object: RichObject| RichText::builder().plain("a").object(object).build();
        let base = with(RichObject::image(1, "a.png", 20.0, 20.0));
        assert_eq!(
            classify_rich_change(
                Some(&base),
                Some(&with(RichObject::image(1, "a.png", 40.0, 20.0)))
            ),
            TextDirty::CONSTRAINT
        );
        assert_eq!(
            classify_rich_change(
                Some(&base),
                Some(&with(RichObject::image(1, "b.png", 20.0, 20.0)))
            ),
            TextDirty::PAINT
        );
        let style = ComputedStyle::default();
        let mut node = TextNodeState::default();
        let (source, _) = node.source_for(base.text(), Some(&base), &style);
        assert_eq!(source.objects().len(), 1);
        assert_eq!(source.objects()[0].metrics.width_px, 20.0);
        node.invalidate(TextDirty::CONSTRAINT);
        let resized = with(RichObject::image(1, "a.png", 40.0, 20.0));
        let (source, copied) = node.source_for(resized.text(), Some(&resized), &style);
        assert!(copied, "a box change rebuilds the source");
        assert_eq!(source.objects()[0].metrics.width_px, 40.0);
    }

    #[test]
    fn rich_changes_are_classified_by_the_tier_they_touch() {
        let red = nana_ui_core::PaintColor::srgb([1.0, 0.0, 0.0, 1.0]);
        let blue = nana_ui_core::PaintColor::srgb([0.0, 0.0, 1.0, 1.0]);
        let base = RichText::new("hello").with_span(0..5, RichSpanStyle::new().color(red));
        let recolored = RichText::new("hello").with_span(0..5, RichSpanStyle::new().color(blue));
        assert_eq!(
            classify_rich_change(Some(&base), Some(&recolored)),
            TextDirty::PAINT
        );
        let resized = base
            .clone()
            .with_span(0..2, RichSpanStyle::new().color(red).size(30.0));
        assert_eq!(
            classify_rich_change(Some(&base), Some(&resized)),
            TextDirty::SHAPE_STYLE
        );
        let effected = base
            .clone()
            .with_span(0..5, RichSpanStyle::new().color(red).effect(2));
        assert_eq!(
            classify_rich_change(Some(&base), Some(&effected)),
            TextDirty::GLYPH_PRESENTATION
        );
        // Cut differently, styled the same: nothing changed.
        let mut split = RichText::new("hello").with_span(0..2, RichSpanStyle::new().color(red));
        split.set_span(2..5, RichSpanStyle::new().color(red));
        assert_eq!(
            classify_rich_change(Some(&base), Some(&split)),
            TextDirty::NONE
        );
        assert_eq!(classify_rich_change(Some(&base), None), TextDirty::PAINT);
        let edited = RichText::new("hullo").with_span(0..5, RichSpanStyle::new().color(red));
        assert!(classify_rich_change(Some(&base), Some(&edited)).contains(TextDirty::CONTENT));
    }

    #[test]
    fn style_changes_are_classified_by_what_reads_them() {
        let base = ComputedStyle::default();
        let classify = |edit: fn(&mut ComputedStyle)| {
            let mut next = base.clone();
            edit(&mut next);
            classify_computed_style_change(&base, &next)
        };
        assert_eq!(
            classify(|s| s.color = Some([1.0, 0.0, 0.0, 1.0])),
            TextDirty::PAINT
        );
        assert_eq!(classify(|s| s.opacity = 0.5), TextDirty::OPACITY);
        assert_eq!(classify(|s| s.font_size = 20.0), TextDirty::SHAPE_STYLE);
        assert_eq!(
            classify(|s| s.font_weight = Some(700)),
            TextDirty::SHAPE_STYLE
        );
        assert_eq!(
            classify(
                |s| s.font_variations = vec![nana_ui_core::FontVariationSetting {
                    tag: *b"wdth",
                    value: 80.0,
                }]
            ),
            TextDirty::SHAPE_STYLE
        );
        assert_eq!(
            classify(|s| s.line_height = Some(LineHeightSpec::Relative(2.0))),
            TextDirty::CONSTRAINT
        );
        assert_eq!(classify(|s| s.cursor_specified = true), TextDirty::NONE);
    }

    #[test]
    fn a_named_mono_family_keeps_its_monospace_fallback() {
        assert_eq!(
            &*nana_font_family("JetBrains Mono"),
            "JetBrains Mono, monospace"
        );
        assert_eq!(&*nana_font_family("Inter"), "Inter");
        assert_eq!(&*nana_font_family("Fira Mono, serif"), "Fira Mono, serif");
    }

    impl TextWork {
        const fn union_layout(self) -> Self {
            Self(self.0 | Self::LAYOUT.0)
        }
    }
}
