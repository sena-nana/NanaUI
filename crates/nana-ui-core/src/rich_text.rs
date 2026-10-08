//! Rich text as application data: one string and the styled byte ranges
//! over it.
//!
//! [`RichText`] is a *value*. The application owns it — builds it, keeps it,
//! hands a clone to a node — and the framework never edits it behind the
//! application's back. A clone is two reference counts, so passing the same
//! document to a display and a history row costs nothing.
//!
//! A span's style is **sparse** and split into the three tiers the text
//! pipeline prices differently:
//!
//! | tier | fields | a change costs |
//! | --- | --- | --- |
//! | [`RichShapeStyle`] | family, size, weight, italic, letter spacing, features | shaping + layout |
//! | [`RichPaintStyle`] | colour, decoration, stroke, shadows | scene paint only |
//! | presentation | [`RichSpanStyle::effect`] | compositor only |
//!
//! Every field is an `Option`: `None` inherits the node's computed style, so
//! one theme change still restyles a whole dialogue that only overrides the
//! words it emphasises.
//!
//! [`AttributedRanges`] is the range algebra underneath: sorted,
//! non-overlapping, normalized byte ranges carrying one attribute each. It is
//! pure range code — it knows nothing about text engines or styles — so an
//! editor's rich session can keep its own attribute type over the same
//! operations.

use std::ops::Range;
use std::sync::Arc;

use crate::{FontFeatureSetting, PaintColor, TextDecorationLine};

/// Sorted, non-overlapping, non-empty byte ranges, one attribute each.
///
/// Kept normalized after every operation: adjacent ranges never carry equal
/// attributes (they are merged), and no range is empty. Two values that
/// attribute the same bytes the same way are therefore `==`, which is what
/// lets a caller compare two documents' styling by comparing the ranges.
///
/// Offsets are bytes. The algebra does not know the text, so it cannot snap
/// to character boundaries; [`RichText`] does that before it gets here.
#[derive(Debug, Clone, PartialEq)]
pub struct AttributedRanges<A> {
    runs: Vec<(Range<usize>, A)>,
}

impl<A> Default for AttributedRanges<A> {
    fn default() -> Self {
        Self { runs: Vec::new() }
    }
}

impl<A: Clone + PartialEq> AttributedRanges<A> {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every attributed range, in order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (Range<usize>, &A)> + '_ {
        self.runs
            .iter()
            .map(|(range, attribute)| (range.clone(), attribute))
    }

    pub fn len(&self) -> usize {
        self.runs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    /// The attribute covering byte `offset`, if any.
    pub fn at(&self, offset: usize) -> Option<&A> {
        let index = self.runs.partition_point(|(range, _)| range.end <= offset);
        self.runs
            .get(index)
            .filter(|(range, _)| range.start <= offset)
            .map(|(_, attribute)| attribute)
    }

    /// Attributes `range` with `attribute`, replacing whatever covered it.
    pub fn set(&mut self, range: Range<usize>, attribute: A) {
        self.update(range, |_| Some(attribute.clone()));
    }

    /// Removes every attribute inside `range`.
    pub fn clear_range(&mut self, range: Range<usize>) {
        self.update(range, |_| None);
    }

    /// Rewrites `range` piece by piece: `f` sees the attribute each piece had
    /// (`None` for a gap) and answers the one it gets (`None` clears it).
    ///
    /// This is the operation a toolbar is made of — "make the selection
    /// bold" keeps every other field each piece already had.
    pub fn update(&mut self, range: Range<usize>, mut f: impl FnMut(Option<&A>) -> Option<A>) {
        if range.start >= range.end {
            return;
        }
        let mut out: Vec<(Range<usize>, A)> = Vec::with_capacity(self.runs.len() + 2);
        let mut cursor = range.start;
        for (run, attribute) in self.runs.drain(..) {
            if run.end <= range.start || run.start >= range.end {
                // Wholly outside: but a gap inside `range` before this run
                // must be offered to `f` first.
                if run.start >= range.end && cursor < range.end {
                    if let Some(new) = f(None) {
                        out.push((cursor..range.end, new));
                    }
                    cursor = range.end;
                }
                out.push((run, attribute));
                continue;
            }
            if run.start < range.start {
                out.push((run.start..range.start, attribute.clone()));
            }
            let inside = run.start.max(range.start)..run.end.min(range.end);
            if cursor < inside.start
                && let Some(new) = f(None)
            {
                out.push((cursor..inside.start, new));
            }
            if let Some(new) = f(Some(&attribute)) {
                out.push((inside.clone(), new));
            }
            cursor = inside.end;
            if run.end > range.end {
                out.push((range.end..run.end, attribute));
            }
        }
        if cursor < range.end
            && let Some(new) = f(None)
        {
            out.push((cursor..range.end, new));
        }
        out.sort_by_key(|(range, _)| range.start);
        self.runs = out;
        self.normalize();
    }

    /// An edit of the underlying text: `removed` bytes go and `inserted`
    /// bytes take their place at `removed.start`.
    ///
    /// Ranges after the edit shift; ranges cut by it shrink. The inserted
    /// bytes take the attribute of the range they were typed *into* — the
    /// one covering the byte before `removed.start` — the way typing at the
    /// end of a bold word stays bold. At offset 0 they take the one after.
    pub fn splice(&mut self, removed: Range<usize>, inserted: usize) {
        let removed = removed.start..removed.end.max(removed.start);
        let inherited = if removed.start > 0 {
            self.at(removed.start - 1).cloned()
        } else {
            self.at(removed.end).cloned()
        };
        let removed_len = removed.end - removed.start;
        let shift = |offset: usize| -> usize {
            if offset <= removed.start {
                offset
            } else if offset >= removed.end {
                // The inserted bytes are opened as a hole below.
                offset - removed_len
            } else {
                removed.start
            }
        };
        let mut out: Vec<(Range<usize>, A)> = Vec::with_capacity(self.runs.len() + 1);
        for (run, attribute) in self.runs.drain(..) {
            let start = shift(run.start);
            let end = shift(run.end);
            if start < end {
                out.push((start..end, attribute));
            }
        }
        self.runs = out;
        if inserted > 0 {
            // Open a hole for the inserted bytes, then fill it.
            let at = removed.start;
            let mut opened: Vec<(Range<usize>, A)> = Vec::with_capacity(self.runs.len() + 1);
            for (run, attribute) in self.runs.drain(..) {
                if run.end <= at {
                    opened.push((run, attribute));
                } else if run.start >= at {
                    opened.push((run.start + inserted..run.end + inserted, attribute));
                } else {
                    opened.push((run.start..at, attribute.clone()));
                    opened.push((at + inserted..run.end + inserted, attribute));
                }
            }
            if let Some(attribute) = inherited {
                opened.push((at..at + inserted, attribute));
                opened.sort_by_key(|(range, _)| range.start);
            }
            self.runs = opened;
        }
        self.normalize();
    }

    /// The attributes inside `range`, rebased so `range.start` is 0.
    pub fn slice(&self, range: Range<usize>) -> Self {
        let mut runs = Vec::new();
        for (run, attribute) in &self.runs {
            let start = run.start.max(range.start);
            let end = run.end.min(range.end);
            if start < end {
                runs.push((start - range.start..end - range.start, attribute.clone()));
            }
        }
        Self { runs }
    }

    /// A projection of every attribute through `f`, normalized: pieces `f`
    /// maps to `None` drop, and neighbours it maps to equal values merge.
    ///
    /// Comparing two projections is how a change is classified by what it
    /// touches — two documents whose shape projections are equal shape
    /// alike, whatever their colours do.
    pub fn map<B: Clone + PartialEq>(
        &self,
        mut f: impl FnMut(&A) -> Option<B>,
    ) -> AttributedRanges<B> {
        let mut out = AttributedRanges {
            runs: self
                .runs
                .iter()
                .filter_map(|(range, attribute)| f(attribute).map(|mapped| (range.clone(), mapped)))
                .collect(),
        };
        out.normalize();
        out
    }

    /// Drops ranges past `len` and cuts the one that crosses it.
    pub fn truncate(&mut self, len: usize) {
        self.runs.retain_mut(|(range, _)| {
            range.end = range.end.min(len);
            range.start < range.end
        });
    }

    fn normalize(&mut self) {
        self.runs.retain(|(range, _)| range.start < range.end);
        let mut merged: Vec<(Range<usize>, A)> = Vec::with_capacity(self.runs.len());
        for (range, attribute) in self.runs.drain(..) {
            if let Some((last, previous)) = merged.last_mut()
                && last.end == range.start
                && *previous == attribute
            {
                last.end = range.end;
                continue;
            }
            merged.push((range, attribute));
        }
        self.runs = merged;
    }
}

/// How the corners of a text stroke are joined.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextStrokeJoin {
    /// Rounded corners: what an outline around lettering usually wants.
    #[default]
    Round,
    Miter,
    Bevel,
}

/// Whether a text stroke paints under the fill or over it.
///
/// `Under` keeps the glyph's own shape whole and grows an outline around it
/// (CSS `paint-order: stroke`); `Over` centres the stroke on the outline and
/// eats into the glyph, the CSS default for `-webkit-text-stroke`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TextStrokePlacement {
    #[default]
    Under,
    Over,
}

/// An outline around each glyph.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RichTextStroke {
    /// Full stroke width in logical px, centred on the glyph outline.
    pub width_px: f32,
    pub color: PaintColor,
    pub join: TextStrokeJoin,
    pub placement: TextStrokePlacement,
}

impl RichTextStroke {
    pub fn new(width_px: f32, color: PaintColor) -> Self {
        Self {
            width_px,
            color,
            join: TextStrokeJoin::Round,
            placement: TextStrokePlacement::Under,
        }
    }

    pub fn join(mut self, join: TextStrokeJoin) -> Self {
        self.join = join;
        self
    }

    pub fn placement(mut self, placement: TextStrokePlacement) -> Self {
        self.placement = placement;
        self
    }
}

/// Most shadow layers one span draws; more are dropped from the end.
pub const MAX_TEXT_SHADOWS: usize = 4;

/// One shadow layer under the glyphs: a blurred, optionally spread copy of
/// their coverage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RichTextShadow {
    /// Logical px, positive right and down.
    pub offset: [f32; 2],
    /// CSS blur radius (twice the Gaussian's standard deviation), logical px.
    pub blur_px: f32,
    /// Grows the coverage by this many logical px before blurring.
    pub spread_px: f32,
    pub color: PaintColor,
}

impl RichTextShadow {
    pub fn new(offset: [f32; 2], blur_px: f32, color: PaintColor) -> Self {
        Self {
            offset,
            blur_px,
            spread_px: 0.0,
            color,
        }
    }

    pub fn spread(mut self, spread_px: f32) -> Self {
        self.spread_px = spread_px;
        self
    }
}

/// The shaping tier of a span: everything that changes a glyph's advance.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RichShapeStyle {
    /// CSS `font-family` list.
    pub family: Option<Arc<str>>,
    pub size_px: Option<f32>,
    pub weight: Option<u16>,
    pub italic: Option<bool>,
    pub letter_spacing_px: Option<f32>,
    /// OpenType features, replacing the node's list.
    pub features: Option<Arc<[FontFeatureSetting]>>,
}

impl RichShapeStyle {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// The paint tier of a span: everything drawn from glyphs already placed.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RichPaintStyle {
    /// Glyph fill.
    pub color: Option<PaintColor>,
    pub decoration: Option<TextDecorationLine>,
    /// Underline and line-through colour; the fill colour when unset.
    pub decoration_color: Option<PaintColor>,
    pub stroke: Option<RichTextStroke>,
    /// Drawn first to last under the glyphs, the first one on top (CSS
    /// order). At most [`MAX_TEXT_SHADOWS`].
    pub shadows: Option<Arc<[RichTextShadow]>>,
}

impl RichPaintStyle {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// A span's sparse style. `None` everywhere inherits the node's style.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RichSpanStyle {
    pub shape: RichShapeStyle,
    pub paint: RichPaintStyle,
    /// Index of a glyph effect the presentation layer plays over the span.
    /// Read by the compositor only: changing it shapes, lays out and paints
    /// nothing.
    pub effect: Option<u16>,
}

impl RichSpanStyle {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.shape.is_empty() && self.paint.is_empty() && self.effect.is_none()
    }

    pub fn family(mut self, family: impl Into<Arc<str>>) -> Self {
        self.shape.family = Some(family.into());
        self
    }

    pub fn size(mut self, size_px: f32) -> Self {
        self.shape.size_px = Some(size_px);
        self
    }

    pub fn weight(mut self, weight: u16) -> Self {
        self.shape.weight = Some(weight);
        self
    }

    pub fn bold(self) -> Self {
        self.weight(700)
    }

    pub fn italic(mut self, italic: bool) -> Self {
        self.shape.italic = Some(italic);
        self
    }

    pub fn letter_spacing(mut self, px: f32) -> Self {
        self.shape.letter_spacing_px = Some(px);
        self
    }

    pub fn features(mut self, features: impl Into<Arc<[FontFeatureSetting]>>) -> Self {
        self.shape.features = Some(features.into());
        self
    }

    pub fn color(mut self, color: PaintColor) -> Self {
        self.paint.color = Some(color);
        self
    }

    pub fn underline(mut self) -> Self {
        let mut decoration = self.paint.decoration.unwrap_or_default();
        decoration.underline = true;
        self.paint.decoration = Some(decoration);
        self
    }

    pub fn line_through(mut self) -> Self {
        let mut decoration = self.paint.decoration.unwrap_or_default();
        decoration.line_through = true;
        self.paint.decoration = Some(decoration);
        self
    }

    pub fn decoration_color(mut self, color: PaintColor) -> Self {
        self.paint.decoration_color = Some(color);
        self
    }

    pub fn stroke(mut self, stroke: RichTextStroke) -> Self {
        self.paint.stroke = Some(stroke);
        self
    }

    /// Adds a shadow layer under the ones already set (CSS order: the first
    /// is drawn on top). Past [`MAX_TEXT_SHADOWS`] it is ignored.
    pub fn shadow(mut self, shadow: RichTextShadow) -> Self {
        let mut layers: Vec<RichTextShadow> =
            self.paint.shadows.as_deref().unwrap_or_default().to_vec();
        if layers.len() < MAX_TEXT_SHADOWS {
            layers.push(shadow);
        }
        self.paint.shadows = Some(layers.into());
        self
    }

    pub fn effect(mut self, effect: u16) -> Self {
        self.effect = Some(effect);
        self
    }

    /// `over` laid on top of `self`: each field `over` sets wins.
    pub fn overlay(&self, over: &Self) -> Self {
        fn pick<T: Clone>(base: &Option<T>, over: &Option<T>) -> Option<T> {
            over.clone().or_else(|| base.clone())
        }
        Self {
            shape: RichShapeStyle {
                family: pick(&self.shape.family, &over.shape.family),
                size_px: pick(&self.shape.size_px, &over.shape.size_px),
                weight: pick(&self.shape.weight, &over.shape.weight),
                italic: pick(&self.shape.italic, &over.shape.italic),
                letter_spacing_px: pick(
                    &self.shape.letter_spacing_px,
                    &over.shape.letter_spacing_px,
                ),
                features: pick(&self.shape.features, &over.shape.features),
            },
            paint: RichPaintStyle {
                color: pick(&self.paint.color, &over.paint.color),
                decoration: pick(&self.paint.decoration, &over.paint.decoration),
                decoration_color: pick(&self.paint.decoration_color, &over.paint.decoration_color),
                stroke: pick(&self.paint.stroke, &over.paint.stroke),
                shadows: pick(&self.paint.shadows, &over.paint.shadows),
            },
            effect: over.effect.or(self.effect),
        }
    }
}

/// The character an inline object stands in the text as (U+FFFC OBJECT
/// REPLACEMENT CHARACTER). One character, so a caret steps over an object in
/// one move and a selection takes it whole.
pub const OBJECT_REPLACEMENT: char = '\u{FFFC}';

/// What an inline object shows.
#[derive(Debug, Clone, PartialEq)]
pub enum RichObjectContent {
    /// An image by URL, with the source rules CSS `url()` has (`file://`,
    /// `nana://res/…`, a policy-gated `http(s)` host). Drawn contained in the
    /// object's box.
    Image { source: Arc<str> },
    /// A host texture slot the application fills — an animated sticker it
    /// decodes itself, a live preview. Drawn by the host texture renderer.
    HostTexture { slot: Arc<str> },
    /// An editor marker: a chip the editor draws over the text and a display
    /// never shows. It always takes no room, so a line breaks the same way in
    /// the editor and on the display.
    Chip { label: Arc<str>, kind: u16 },
    /// An editor marker shown as a labelled tag: a small pill with `label`
    /// in it. Unlike a [`Self::Chip`] it takes room on the line (the room
    /// its label needs, shaped by the text engine) and it is drawn wherever
    /// the text is, so a display that must break lines like the editor
    /// leaves tags out of its text.
    Tag { label: Arc<str>, kind: u16 },
}

/// An object inline in the text: a sticker, an emote, an editor chip.
///
/// It stands on the baseline: `height_px - descent_px` above it, `descent_px`
/// below. Changing its size relays the paragraph out without shaping it
/// again; changing only what it shows repaints it.
#[derive(Debug, Clone, PartialEq)]
pub struct RichObject {
    /// The application's name for it, handed back by the editor's events and
    /// the scene.
    pub id: u64,
    pub width_px: f32,
    pub height_px: f32,
    pub descent_px: f32,
    pub content: RichObjectContent,
}

impl RichObject {
    /// An image `width_px` × `height_px`, standing on the baseline.
    pub fn image(id: u64, source: impl Into<Arc<str>>, width_px: f32, height_px: f32) -> Self {
        Self {
            id,
            width_px,
            height_px,
            descent_px: 0.0,
            content: RichObjectContent::Image {
                source: source.into(),
            },
        }
    }

    /// A host texture slot `width_px` × `height_px`.
    pub fn texture(id: u64, slot: impl Into<Arc<str>>, width_px: f32, height_px: f32) -> Self {
        Self {
            id,
            width_px,
            height_px,
            descent_px: 0.0,
            content: RichObjectContent::HostTexture { slot: slot.into() },
        }
    }

    /// An editor-only marker chip. It takes no room on the line.
    pub fn chip(id: u64, label: impl Into<Arc<str>>, kind: u16) -> Self {
        Self {
            id,
            width_px: 0.0,
            height_px: 0.0,
            descent_px: 0.0,
            content: RichObjectContent::Chip {
                label: label.into(),
                kind,
            },
        }
    }

    /// An editor marker shown as a labelled tag (see [`RichObjectContent::Tag`]).
    /// Its size is its label's, so it has none of its own.
    pub fn tag(id: u64, label: impl Into<Arc<str>>, kind: u16) -> Self {
        Self {
            id,
            width_px: 0.0,
            height_px: 0.0,
            descent_px: 0.0,
            content: RichObjectContent::Tag {
                label: label.into(),
                kind,
            },
        }
    }

    /// The label a tag shows.
    pub fn tag_label(&self) -> Option<&Arc<str>> {
        match &self.content {
            RichObjectContent::Tag { label, .. } => Some(label),
            _ => None,
        }
    }

    /// How far the object reaches below the baseline.
    pub fn descent(mut self, descent_px: f32) -> Self {
        self.descent_px = descent_px;
        self
    }

    /// Whether only an editor shows it.
    pub fn editor_only(&self) -> bool {
        matches!(self.content, RichObjectContent::Chip { .. })
    }

    /// The box it takes on its line: width, ascent, descent. A chip takes
    /// none.
    pub fn line_box(&self) -> [f32; 3] {
        if self.editor_only() || self.tag_label().is_some() {
            return [0.0; 3];
        }
        let finite = |value: f32| {
            if value.is_finite() {
                value.max(0.0)
            } else {
                0.0
            }
        };
        let descent = finite(self.descent_px);
        [
            finite(self.width_px),
            (finite(self.height_px) - descent).max(0.0),
            descent,
        ]
    }
}

/// One string and its styled ranges, owned by the application.
///
/// Ranges are bytes into [`Self::text`]. Every way in snaps them down to
/// character boundaries and clips them to the text, so a value always
/// addresses whole characters. Inline objects sit at the
/// [`OBJECT_REPLACEMENT`] characters of the text; each is looked up by the
/// byte offset of its character. Ruby annotations name a base range and the
/// small text set above it (horizontal text only).
#[derive(Debug, Clone, Default)]
pub struct RichText {
    text: Arc<str>,
    spans: Arc<AttributedRanges<RichSpanStyle>>,
    /// Sorted by offset; each names an [`OBJECT_REPLACEMENT`] of `text`.
    objects: Arc<Vec<(usize, RichObject)>>,
    /// Sorted, non-overlapping, never empty.
    rubies: Arc<Vec<(Range<usize>, Arc<str>)>>,
}

impl PartialEq for RichText {
    fn eq(&self, other: &Self) -> bool {
        (Arc::ptr_eq(&self.text, &other.text) || self.text == other.text)
            && (Arc::ptr_eq(&self.spans, &other.spans) || self.spans == other.spans)
            && (Arc::ptr_eq(&self.objects, &other.objects) || self.objects == other.objects)
            && (Arc::ptr_eq(&self.rubies, &other.rubies) || self.rubies == other.rubies)
    }
}

impl RichText {
    /// Plain text, no spans: the node's style everywhere.
    pub fn new(text: impl Into<Arc<str>>) -> Self {
        Self {
            text: text.into(),
            spans: Arc::default(),
            objects: Arc::default(),
            rubies: Arc::default(),
        }
    }

    /// The ruby annotations: each base range and the text set above it,
    /// sorted by base.
    pub fn rubies(&self) -> &[(Range<usize>, Arc<str>)] {
        &self.rubies
    }

    /// The annotation whose base covers byte `offset`.
    pub fn ruby_at(&self, offset: usize) -> Option<(Range<usize>, &Arc<str>)> {
        self.rubies
            .iter()
            .find(|(base, _)| base.contains(&offset))
            .map(|(base, text)| (base.clone(), text))
    }

    /// Sets `annotation` above `base` (snapped to character boundaries),
    /// replacing every annotation the base overlaps. An empty base or
    /// annotation only clears.
    pub fn set_ruby(&mut self, base: Range<usize>, annotation: impl Into<Arc<str>>) {
        let base = self.snap(base);
        let annotation = annotation.into();
        self.clear_ruby(base.clone());
        if base.is_empty() || annotation.is_empty() {
            return;
        }
        let rubies = Arc::make_mut(&mut self.rubies);
        let index = rubies.partition_point(|(existing, _)| existing.start < base.start);
        rubies.insert(index, (base, annotation));
    }

    /// Removes every annotation whose base overlaps `range` (or, for an
    /// empty range, contains it).
    pub fn clear_ruby(&mut self, range: Range<usize>) {
        if self.rubies.is_empty() {
            return;
        }
        let hits = |base: &Range<usize>| {
            if range.is_empty() {
                base.start <= range.start && range.start < base.end
            } else {
                base.start < range.end && range.start < base.end
            }
        };
        if self.rubies.iter().any(|(base, _)| hits(base)) {
            Arc::make_mut(&mut self.rubies).retain(|(base, _)| !hits(base));
        }
    }

    /// The inline objects, sorted by the offset of their character.
    pub fn objects(&self) -> &[(usize, RichObject)] {
        &self.objects
    }

    /// The object whose character sits at byte `offset`.
    pub fn object_at(&self, offset: usize) -> Option<&RichObject> {
        self.objects
            .binary_search_by_key(&offset, |(at, _)| *at)
            .ok()
            .map(|index| &self.objects[index].1)
    }

    /// Inserts `object` at byte `offset` (snapped to a character boundary):
    /// its character goes into the text there, styled like the character
    /// before it.
    pub fn insert_object(&mut self, offset: usize, object: RichObject) {
        let at = self.snap(offset..offset).start;
        self.replace_range(at..at, OBJECT_REPLACEMENT.encode_utf8(&mut [0; 4]));
        let objects = Arc::make_mut(&mut self.objects);
        let index = objects.partition_point(|(existing, _)| *existing < at);
        objects.insert(index, (at, object));
    }

    /// Replaces the object at byte `offset` in place, keeping its character.
    /// Returns `false` when no object sits there.
    pub fn set_object(&mut self, offset: usize, object: RichObject) -> bool {
        let Ok(index) = self.objects.binary_search_by_key(&offset, |(at, _)| *at) else {
            return false;
        };
        Arc::make_mut(&mut self.objects)[index].1 = object;
        true
    }

    pub fn builder() -> RichTextBuilder {
        RichTextBuilder::default()
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// The text as the shared allocation it is held in, for a holder that
    /// keeps it without copying.
    pub fn shared_text(&self) -> &Arc<str> {
        &self.text
    }

    pub fn spans(&self) -> &AttributedRanges<RichSpanStyle> {
        &self.spans
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// The span style at byte `offset`, if a span covers it.
    pub fn style_at(&self, offset: usize) -> Option<&RichSpanStyle> {
        self.spans.at(offset)
    }

    /// Replaces the style of `range` with `style`.
    pub fn with_span(mut self, range: Range<usize>, style: RichSpanStyle) -> Self {
        self.set_span(range, style);
        self
    }

    /// Replaces the style of `range` with `style`.
    pub fn set_span(&mut self, range: Range<usize>, style: RichSpanStyle) {
        let range = self.snap(range);
        Arc::make_mut(&mut self.spans)
            .update(range, |_| (!style.is_empty()).then(|| style.clone()));
    }

    /// Lays `style` over whatever `range` already carries, field by field.
    pub fn apply_span(&mut self, range: Range<usize>, style: &RichSpanStyle) {
        let range = self.snap(range);
        Arc::make_mut(&mut self.spans).update(range, |current| {
            let merged = current.cloned().unwrap_or_default().overlay(style);
            (!merged.is_empty()).then_some(merged)
        });
    }

    /// Replaces `range` of the text with `replacement`, shifting the spans
    /// and objects. Inserted text takes the style of the character before it;
    /// objects inside `range` go with their characters.
    pub fn replace_range(&mut self, range: Range<usize>, replacement: &str) {
        let range = self.snap(range);
        let mut text = String::with_capacity(self.text.len() - range.len() + replacement.len());
        text.push_str(&self.text[..range.start]);
        text.push_str(replacement);
        text.push_str(&self.text[range.end..]);
        self.text = text.into();
        Arc::make_mut(&mut self.spans).splice(range.clone(), replacement.len());
        if !self.objects.is_empty() {
            let objects = Arc::make_mut(&mut self.objects);
            objects.retain(|(at, _)| *at < range.start || *at >= range.end);
            for (at, _) in objects.iter_mut() {
                if *at >= range.end {
                    *at = *at + replacement.len() - range.len();
                }
            }
        }
        if !self.rubies.is_empty() {
            // An edit touching the inside of a base drops its annotation:
            // the reading no longer matches the text.
            let rubies = Arc::make_mut(&mut self.rubies);
            rubies.retain(|(base, _)| base.end <= range.start || base.start >= range.end);
            for (base, _) in rubies.iter_mut() {
                if base.start >= range.end {
                    base.start = base.start + replacement.len() - range.len();
                    base.end = base.end + replacement.len() - range.len();
                }
            }
        }
    }

    /// The piece of this document over `range`: its text, the spans over it
    /// and the objects in it, rebased so the piece starts at 0. What a copy
    /// takes.
    pub fn slice(&self, range: Range<usize>) -> RichText {
        let range = self.snap(range);
        RichText {
            text: Arc::from(&self.text[range.clone()]),
            spans: Arc::new(self.spans.slice(range.clone())),
            objects: Arc::new(
                self.objects
                    .iter()
                    .filter(|(at, _)| range.contains(at))
                    .map(|(at, object)| (at - range.start, object.clone()))
                    .collect(),
            ),
            rubies: Arc::new(
                self.rubies
                    .iter()
                    .filter(|(base, _)| range.start <= base.start && base.end <= range.end)
                    .map(|(base, text)| {
                        (
                            base.start - range.start..base.end - range.start,
                            text.clone(),
                        )
                    })
                    .collect(),
            ),
        }
    }

    /// Replaces `range` with `piece`, its spans and objects included. What a
    /// paste of a copied piece does.
    pub fn replace_with(&mut self, range: Range<usize>, piece: &RichText) {
        let range = self.snap(range);
        let at = range.start;
        self.replace_range(range, piece.text());
        if !piece.spans.is_empty() || !self.spans.is_empty() {
            let inserted = at..at + piece.text.len();
            let spans = Arc::make_mut(&mut self.spans);
            spans.clear_range(inserted);
            for (span, style) in piece.spans.iter() {
                spans.set(span.start + at..span.end + at, style.clone());
            }
        }
        if !piece.objects.is_empty() {
            let objects = Arc::make_mut(&mut self.objects);
            for (offset, object) in piece.objects.iter() {
                let offset = offset + at;
                let index = objects.partition_point(|(existing, _)| *existing < offset);
                objects.insert(index, (offset, object.clone()));
            }
        }
        if !piece.rubies.is_empty() {
            let rubies = Arc::make_mut(&mut self.rubies);
            for (base, text) in piece.rubies.iter() {
                let base = base.start + at..base.end + at;
                let index = rubies.partition_point(|(existing, _)| existing.start < base.start);
                rubies.insert(index, (base, text.clone()));
            }
        }
    }

    /// Byte range clipped to the text and moved onto character boundaries.
    fn snap(&self, range: Range<usize>) -> Range<usize> {
        let floor = |mut offset: usize| {
            offset = offset.min(self.text.len());
            while !self.text.is_char_boundary(offset) {
                offset -= 1;
            }
            offset
        };
        let start = floor(range.start);
        let end = floor(range.end).max(start);
        start..end
    }
}

impl From<&str> for RichText {
    fn from(text: &str) -> Self {
        Self::new(text)
    }
}

impl From<String> for RichText {
    fn from(text: String) -> Self {
        Self::new(text)
    }
}

/// Builds a [`RichText`] one styled piece at a time.
#[derive(Debug, Default)]
pub struct RichTextBuilder {
    text: String,
    spans: AttributedRanges<RichSpanStyle>,
    objects: Vec<(usize, RichObject)>,
    rubies: Vec<(Range<usize>, Arc<str>)>,
}

impl RichTextBuilder {
    /// Appends `base` in the node's own style, with `annotation` set above
    /// it.
    pub fn ruby(&mut self, base: &str, annotation: impl Into<Arc<str>>) -> &mut Self {
        self.styled_ruby(base, annotation, RichSpanStyle::new())
    }

    /// Appends `base` styled with `style`, with `annotation` set above it in
    /// the base's style at half its size.
    pub fn styled_ruby(
        &mut self,
        base: &str,
        annotation: impl Into<Arc<str>>,
        style: RichSpanStyle,
    ) -> &mut Self {
        let start = self.text.len();
        self.push(base, style);
        let annotation = annotation.into();
        if !base.is_empty() && !annotation.is_empty() {
            self.rubies.push((start..self.text.len(), annotation));
        }
        self
    }

    /// Appends `text` in the node's own style.
    pub fn plain(&mut self, text: &str) -> &mut Self {
        self.text.push_str(text);
        self
    }

    /// Appends `text` styled with `style`.
    pub fn push(&mut self, text: &str, style: RichSpanStyle) -> &mut Self {
        let start = self.text.len();
        self.text.push_str(text);
        if !style.is_empty() {
            self.spans.set(start..self.text.len(), style);
        }
        self
    }

    /// Appends an inline object, in the node's own style.
    pub fn object(&mut self, object: RichObject) -> &mut Self {
        let at = self.text.len();
        self.text.push(OBJECT_REPLACEMENT);
        self.objects.push((at, object));
        self
    }

    /// Appends an inline object whose character carries `style` (an effect
    /// or a decoration that should reach it).
    pub fn styled_object(&mut self, object: RichObject, style: RichSpanStyle) -> &mut Self {
        let at = self.text.len();
        self.object(object);
        if !style.is_empty() {
            self.spans.set(at..self.text.len(), style);
        }
        self
    }

    pub fn build(&mut self) -> RichText {
        RichText {
            text: std::mem::take(&mut self.text).into(),
            spans: Arc::new(std::mem::take(&mut self.spans)),
            objects: Arc::new(std::mem::take(&mut self.objects)),
            rubies: Arc::new(std::mem::take(&mut self.rubies)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranges(values: &[(Range<usize>, u8)]) -> AttributedRanges<u8> {
        let mut out = AttributedRanges::new();
        for (range, value) in values {
            out.set(range.clone(), *value);
        }
        out
    }

    fn dump(ranges: &AttributedRanges<u8>) -> Vec<(Range<usize>, u8)> {
        ranges
            .iter()
            .map(|(range, value)| (range, *value))
            .collect()
    }

    #[test]
    fn setting_over_ranges_splits_and_merges() {
        let mut attributed = ranges(&[(0..10, 1)]);
        attributed.set(3..5, 2);
        assert_eq!(dump(&attributed), vec![(0..3, 1), (3..5, 2), (5..10, 1)]);
        attributed.set(3..5, 1);
        assert_eq!(
            dump(&attributed),
            vec![(0..10, 1)],
            "equal neighbours merge"
        );
        attributed.set(12..14, 1);
        assert_eq!(dump(&attributed), vec![(0..10, 1), (12..14, 1)]);
        attributed.set(9..13, 1);
        assert_eq!(dump(&attributed), vec![(0..14, 1)]);
        attributed.clear_range(2..4);
        assert_eq!(dump(&attributed), vec![(0..2, 1), (4..14, 1)]);
        assert_eq!(attributed.at(3), None);
        assert_eq!(attributed.at(4), Some(&1));
    }

    #[test]
    fn update_offers_gaps_and_pieces_alike() {
        let mut attributed = ranges(&[(2..4, 1), (6..8, 2)]);
        attributed.update(0..10, |current| Some(current.copied().unwrap_or(0) + 10));
        assert_eq!(
            dump(&attributed),
            vec![(0..2, 10), (2..4, 11), (4..6, 10), (6..8, 12), (8..10, 10)]
        );
    }

    #[test]
    fn typing_inherits_the_style_before_the_caret_and_edits_shift_the_rest() {
        let mut attributed = ranges(&[(0..5, 1), (5..9, 2)]);
        attributed.splice(5..5, 3);
        assert_eq!(dump(&attributed), vec![(0..8, 1), (8..12, 2)]);
        attributed.splice(6..9, 0);
        assert_eq!(dump(&attributed), vec![(0..6, 1), (6..9, 2)]);
        attributed.splice(0..0, 2);
        assert_eq!(dump(&attributed), vec![(0..8, 1), (8..11, 2)]);
        attributed.splice(0..11, 0);
        assert!(attributed.is_empty());
    }

    #[test]
    fn projections_compare_only_what_they_keep() {
        let a = ranges(&[(0..3, 1), (3..6, 2)]);
        let b = ranges(&[(0..3, 3), (3..6, 4)]);
        let parity = |value: &u8| Some(*value % 2);
        assert_eq!(a.map(parity), b.map(parity));
        assert_eq!(dump(&a.map(|_| Some(0u8))), vec![(0..6, 0)]);
    }

    #[test]
    fn a_rich_text_snaps_spans_to_characters_and_clips_them() {
        let rich = RichText::new("中文ab").with_span(1..100, RichSpanStyle::new().bold());
        let (range, style) = rich.spans().iter().next().expect("one span");
        assert_eq!(
            range,
            0..rich.text().len(),
            "snapped down to the character start"
        );
        assert_eq!(style.shape.weight, Some(700));
    }

    #[test]
    fn the_builder_records_each_piece_and_skips_plain_ones() {
        let rich = RichText::builder()
            .plain("Hello ")
            .push(
                "world",
                RichSpanStyle::new().color(PaintColor::srgb([1.0, 0.0, 0.0, 1.0])),
            )
            .build();
        assert_eq!(rich.text(), "Hello world");
        assert_eq!(rich.spans().len(), 1);
        assert!(rich.style_at(0).is_none());
        assert!(rich.style_at(6).is_some());
    }

    #[test]
    fn applying_a_span_keeps_the_fields_it_does_not_set() {
        let mut rich = RichText::new("abcdef").with_span(0..6, RichSpanStyle::new().size(30.0));
        rich.apply_span(2..4, &RichSpanStyle::new().bold());
        let middle = rich.style_at(2).expect("styled");
        assert_eq!(middle.shape.size_px, Some(30.0));
        assert_eq!(middle.shape.weight, Some(700));
        assert_eq!(rich.style_at(0).unwrap().shape.weight, None);
    }

    #[test]
    fn shadows_stop_at_the_layer_cap() {
        let shadow = RichTextShadow::new([1.0, 1.0], 2.0, PaintColor::srgb([0.0; 4]));
        let mut style = RichSpanStyle::new();
        for _ in 0..6 {
            style = style.shadow(shadow);
        }
        assert_eq!(
            style.paint.shadows.as_deref().unwrap().len(),
            MAX_TEXT_SHADOWS
        );
    }

    #[test]
    fn rubies_ride_on_their_bases_and_go_with_copies() {
        let mut rich = RichText::builder()
            .plain("我是")
            .ruby("漢字", "かんじ")
            .plain("です")
            .build();
        assert_eq!(rich.rubies()[0].0, 6..12);
        assert_eq!(&**rich.ruby_at(9).unwrap().1, "かんじ");
        rich.replace_range(0..0, "啊");
        assert_eq!(rich.rubies()[0].0, 9..15, "an edit before a base shifts it");
        let piece = rich.slice(3..15);
        assert_eq!(piece.rubies()[0].0, 6..12, "a copy keeps a whole base");
        assert!(
            rich.slice(12..15).rubies().is_empty(),
            "but not part of one"
        );
        let mut target = RichText::new("x");
        target.replace_with(1..1, &piece);
        assert_eq!(target.rubies()[0].0, 7..13);
        rich.set_ruby(9..12, "かん");
        rich.set_ruby(9..15, "かんじ");
        assert_eq!(
            rich.rubies().len(),
            1,
            "a new annotation replaces the one it overlaps"
        );
        assert_eq!(rich.rubies()[0].0, 9..15);
        rich.replace_range(12..12, "x");
        assert_eq!(rich.rubies().len(), 0, "an edit inside a base drops it");
        rich.set_ruby(0..3, "a");
        rich.clear_ruby(1..1);
        assert!(rich.rubies().is_empty());
    }

    #[test]
    fn objects_ride_on_their_characters_through_edits() {
        let sticker = RichObject::image(1, "file:///cat.png", 32.0, 32.0);
        let mut rich = RichText::builder()
            .plain("hi ")
            .object(sticker.clone())
            .plain(" there")
            .build();
        assert_eq!(rich.object_at(3), Some(&sticker));
        rich.replace_range(0..2, "hello");
        assert_eq!(
            rich.objects()[0].0,
            6,
            "objects shift with the text before them"
        );
        rich.insert_object(0, RichObject::chip(2, "wait", 0));
        assert_eq!(rich.objects().len(), 2);
        assert_eq!(rich.objects()[0].0, 0);
        assert_eq!(rich.objects()[1].0, 6 + OBJECT_REPLACEMENT.len_utf8());
        let chip = &rich.objects()[0].1;
        assert!(chip.editor_only());
        assert_eq!(chip.line_box(), [0.0; 3], "a chip takes no room");
        let at = rich.objects()[1].0;
        rich.replace_range(at..at + OBJECT_REPLACEMENT.len_utf8(), "");
        assert_eq!(
            rich.objects().len(),
            1,
            "deleting the character deletes the object"
        );
        assert_eq!(
            RichObject::image(3, "a.png", 20.0, 30.0)
                .descent(6.0)
                .line_box(),
            [20.0, 24.0, 6.0]
        );
    }

    #[test]
    fn a_slice_pasted_back_is_the_piece_it_was() {
        let bold = RichSpanStyle::new().bold();
        let doc = RichText::builder()
            .plain("a ")
            .push("bold", bold.clone())
            .object(RichObject::image(4, "x.png", 8.0, 8.0))
            .plain(" z")
            .build();
        let piece = doc.slice(2..doc.text().len() - 2);
        assert_eq!(piece.text(), "bold\u{FFFC}");
        assert_eq!(piece.style_at(0), Some(&bold));
        assert_eq!(piece.objects()[0].0, 4);
        let mut target = RichText::new("[]");
        target.replace_with(1..1, &piece);
        assert_eq!(target.text(), "[bold\u{FFFC}]");
        assert_eq!(target.style_at(1), Some(&bold));
        assert_eq!(target.style_at(0), None);
        assert_eq!(target.objects()[0].0, 5);
    }

    #[test]
    fn replacing_text_moves_the_spans_with_it() {
        let mut rich = RichText::new("say hi").with_span(4..6, RichSpanStyle::new().bold());
        rich.replace_range(0..3, "whisper");
        assert_eq!(rich.text(), "whisper hi");
        let (range, _) = rich.spans().iter().next().unwrap();
        assert_eq!(range, 8..10);
    }
}
