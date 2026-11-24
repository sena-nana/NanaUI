//! Paragraph color span resolution, effect layering, decorations, and solid quads.

use nana_text::TextLayout;
use nana_ui_runtime::TextVerticalAlignment;
use nana_ui_scene::{SceneGlyphPresentation, SceneRichPaint, SceneTextEffects, SceneTextSpan};
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;

use super::super::clip::{self, LogicalRect};
use super::super::color::{linear_from_srgb8, to_rgba8};
use super::atlas;
use super::entry::{EntrySegment, EntryStore};
use super::glyph::{
    GlyphRenderMode, GlyphRole, GlyphSynthesis, NanaGlyphBuffer, NanaGlyphRun, PlacedGlyph,
};
use super::ordinals::GraphemeOrdinals;
use super::pipeline::{self, CONTENT_MASK, GlyphInstance};
use super::raster;

#[derive(Default)]
pub(super) struct ShapeHasher(u64);

impl Hasher for ShapeHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        let mut state = if self.0 == 0 {
            0xcbf2_9ce4_8422_2325
        } else {
            self.0
        };
        for byte in bytes {
            state ^= u64::from(*byte);
            state = state.wrapping_mul(0x1000_0000_01b3);
        }
        self.0 = state;
    }
}

/// Open or extend the segment an entry's glyph at `offset` belongs to.
///
/// A segment has to name a page of each kind because one bind group does, but
/// only the kinds it actually samples are constrained. A segment still holding
/// a placeholder has not sampled that kind yet, so the first glyph of it adopts
/// a page instead of splitting — which is what keeps one emoji in a line of
/// text free.
pub(super) fn push_entry_segment(
    segments: &mut Vec<EntrySegment>,
    placeholders: [u32; 2],
    mask_page: u32,
    color_page: u32,
    offset: u32,
    material: u64,
) {
    if let Some(open) = segments.last_mut() {
        let mask_ok = open.mask_page == mask_page
            || open.mask_page == placeholders[0]
            || mask_page == placeholders[0];
        let color_ok = open.color_page == color_page
            || open.color_page == placeholders[1]
            || color_page == placeholders[1];
        if mask_ok && color_ok && open.material == material {
            if mask_page != placeholders[0] {
                open.mask_page = mask_page;
            }
            if color_page != placeholders[1] {
                open.color_page = color_page;
            }
            open.count += 1;
            return;
        }
    }
    segments.push(EntrySegment {
        material,
        mask_page,
        color_page,
        first: offset,
        count: 1,
    });
}

/// What each byte of the content paints in.
///
/// Rich text is a color per byte range over one layout. Since #99 the layout
/// does not know about color at all, so this is what puts the two back
/// together — at resolve time, per glyph cluster, which is why a recolor is no
/// longer a reason to lay anything out again.
///
/// Node opacity is deliberately not folded in: it rides on the run row, so a
/// fade neither relayouts rich text nor rewrites a glyph.
#[derive(Clone, Copy)]
pub(super) struct ResolvedSpanColor {
    pub(super) srgb: [f32; 4],
    pub(super) paint_color: Option<nana_ui_core::PaintColor>,
}

pub(super) struct SpanColors {
    /// Well-formed spans, in start order. Empty means solid `default`.
    pub(super) spans: Vec<(Range<usize>, ResolvedSpanColor)>,
    pub(super) default: [f32; 4],
    pub(super) default_paint_color: Option<nana_ui_core::PaintColor>,
    /// Identity of the whole mapping, for [`TextGpuEntry::colors`]. Zero for
    /// solid text, which is the case that may be recoloured without rebuilding
    /// anything.
    pub(super) fingerprint: u64,
}

#[cfg(test)]
mod span_color_tests {
    use super::*;

    #[test]
    fn authoring_span_keeps_extended_linear_sc_rgb_channels() {
        let paint = nana_ui_core::PaintColor::LinearScRgb {
            channels: [1.25, -0.125, 0.25],
            alpha: 0.7,
        };
        let spans = [SceneTextSpan {
            start: 0,
            end: 1,
            color: paint,
        }];
        let colors = SpanColors::new("x", &spans, [0.0; 4], None);
        let resolved = colors.color_at(0);
        assert_eq!(resolved.paint_color, Some(paint));
        let (channels, alpha) = resolved.paint_color.unwrap().to_linear_sc_rgb();
        assert_eq!(channels, [1.25, -0.125, 0.25]);
        assert_eq!(alpha, 0.7);
    }
}

impl SpanColors {
    pub(super) fn new(
        content: &str,
        spans: &[SceneTextSpan],
        default: [f32; 4],
        default_paint_color: Option<nana_ui_core::PaintColor>,
    ) -> Self {
        let mut ordered: Vec<&SceneTextSpan> = spans.iter().collect();
        // Overlap is resolved by start order below, so the order has to be the
        // one the rule assumes rather than the one the scene happened to write.
        ordered.sort_by_key(|span| (span.start, span.end));
        let mut kept: Vec<(Range<usize>, ResolvedSpanColor)> = Vec::new();
        for span in ordered {
            // A span the scene built against different bytes would recolor
            // whatever now sits at those offsets, so a range that is not a
            // char boundary of *this* content is dropped rather than snapped.
            if span.start >= span.end
                || span.end > content.len()
                || !content.is_char_boundary(span.start)
                || !content.is_char_boundary(span.end)
            {
                continue;
            }
            // Overlaps would make "which span wins" depend on search order.
            // Later spans lose, which is the order `Scene` writes them in.
            if kept.last().is_some_and(|(last, _)| last.end > span.start) {
                continue;
            }
            kept.push((
                span.start..span.end,
                ResolvedSpanColor {
                    srgb: span.color.to_srgb(),
                    paint_color: (!matches!(span.color, nana_ui_core::PaintColor::Srgb { .. }))
                        .then_some(span.color),
                },
            ));
        }
        // The default is in the fingerprint because it decides which glyphs
        // carry their own colour: a span that happens to paint the default
        // inherits the run row, and would keep inheriting a *different* one
        // after a recolour if this did not notice.
        let fingerprint = if kept.is_empty() {
            0
        } else {
            let mut hasher = ShapeHasher::default();
            default.map(f32::to_bits).hash(&mut hasher);
            kept.len().hash(&mut hasher);
            for (range, color) in &kept {
                range.start.hash(&mut hasher);
                range.end.hash(&mut hasher);
                color.srgb.map(f32::to_bits).hash(&mut hasher);
                hash_paint_color(color.paint_color, &mut hasher);
            }
            hash_paint_color(default_paint_color, &mut hasher);
            // Zero means "solid"; a fingerprint that lands there would claim it.
            hasher.finish() | 1
        };
        Self {
            spans: kept,
            default,
            default_paint_color,
            fingerprint,
        }
    }

    /// The color at a byte offset. Binary search rather than a cursor: an RTL
    /// run walks its clusters backwards, so offsets do not arrive in order.
    pub(super) fn color_at(&self, byte: usize) -> ResolvedSpanColor {
        if self.spans.is_empty() {
            return ResolvedSpanColor {
                srgb: self.default,
                paint_color: self.default_paint_color,
            };
        }
        let index = self.spans.partition_point(|(range, _)| range.start <= byte);
        match index.checked_sub(1).and_then(|i| self.spans.get(i)) {
            Some((range, color)) if range.end > byte => *color,
            _ => ResolvedSpanColor {
                srgb: self.default,
                paint_color: self.default_paint_color,
            },
        }
    }
}

pub(super) fn hash_paint_color(
    paint: Option<nana_ui_core::PaintColor>,
    hasher: &mut impl std::hash::Hasher,
) {
    use std::hash::Hash;
    match paint {
        None => 0u8.hash(hasher),
        Some(paint) => {
            1u8.hash(hasher);
            std::mem::discriminant(&paint).hash(hasher);
            let (channels, alpha) = paint.to_linear_sc_rgb();
            channels.map(f32::to_bits).hash(hasher);
            alpha.to_bits().hash(hasher);
        }
    }
}

/// The effects `prepare` hands `build_entry`: the scene's, or the legacy
/// single shadow of a primitive that carries none.
#[derive(Clone, Copy)]
pub(super) struct EffectSource<'a> {
    pub(super) rich: Option<&'a SceneRichPaint>,
    pub(super) legacy_shadow: Option<nana_ui_core::TextShadowSpec>,
}

/// Widest logical CSS blur radius a shadow is drawn with. Past it the kernel
/// costs more than the softness is worth.
pub(super) const MAX_SHADOW_BLUR: f32 = 24.0;

/// An underline or a strikeout, in the entry's raster px.
#[derive(Clone, Copy, Debug)]
pub(super) struct SolidQuad {
    pub(super) origin: [i32; 2],
    pub(super) size: [u32; 2],
    pub(super) color: [f32; 4],
    paint_color: Option<nana_ui_core::PaintColor>,
    pub(super) line_through: bool,
    /// The byte of the first glyph it runs under.
    pub(super) cluster: u32,
}

/// Writes the underlines (or, with `line_through`, the strikeouts) of
/// `solids` into entry `id` from slot `placed` on.
#[allow(clippy::too_many_arguments)]
pub(super) fn place_solid_quads(
    entries: &mut EntryStore,
    id: u32,
    solids: &[SolidQuad],
    line_through: bool,
    inherited: u32,
    placeholders: [u32; 2],
    placed: &mut u32,
    segments: &mut Vec<EntrySegment>,
    ordinals: &GraphemeOrdinals,
) {
    for quad in solids
        .iter()
        .filter(|quad| quad.line_through == line_through)
    {
        let color = pipeline::pack_srgb(quad.color);
        // A line in the paragraph's own colour inherits the run row, so a
        // recolour moves it with the glyphs and rebuilds nothing.
        let own = if quad.paint_color.is_some() {
            pipeline::INSTANCE_OWN_COLOR | pipeline::INSTANCE_LINEAR_COLOR
        } else if color == inherited {
            0
        } else {
            pipeline::INSTANCE_OWN_COLOR
        };
        let linear_color = quad
            .paint_color
            .map(super::super::color::pack_paint_color)
            .unwrap_or([0.0; 4]);
        push_entry_segment(
            segments,
            placeholders,
            placeholders[0],
            placeholders[1],
            *placed,
            0,
        );
        entries.push_glyph(
            id,
            *placed,
            atlas::GlyphAtlasEntryId::STALE,
            GlyphInstance::new(
                quad.origin,
                quad.size,
                [0, 0],
                color,
                CONTENT_MASK | own,
                linear_color,
            )
            .with_role(pipeline::ROLE_SOLID)
            // A line reveals with the first glyph it runs under.
            .with_ordinal(ordinals.of(quad.cluster as usize)),
        );
        *placed += 1;
    }
}

/// Appends `presentation`'s header, effect table and per-grapheme pairs to
/// `fx`, returning the header's word offset. The layout `glyph_look` in
/// `text_atlas.wgsl` reads.
pub(super) fn pack_glyph_fx(fx: &mut Vec<u32>, presentation: &SceneGlyphPresentation) -> u32 {
    const HEADER: usize = 32;
    let at = fx.len();
    let reveal = presentation.reveal.as_ref();
    let clusters = presentation
        .cluster_effects
        .len()
        .max(reveal.map_or(0, |reveal| reveal.at_s.len()));
    let mut flags = 0u32;
    if reveal.is_some() {
        flags |= 1;
    }
    if !presentation.cluster_effects.is_empty() {
        flags |= 2;
    }
    let (start_secs, start_fraction) = reveal.map_or((0, 0.0), |reveal| {
        nana_ui_core::motion::glyph::split_seconds(reveal.start)
    });
    let intro = reveal.map(|reveal| reveal.intro).unwrap_or_default();
    let transitions = &presentation.transitions;
    let easing = match intro.easing {
        nana_ui_core::GlyphEasing::Linear => 0u32,
        nana_ui_core::GlyphEasing::EaseOut => 1,
        nana_ui_core::GlyphEasing::BackOut => 2,
    };
    let effects_at = at + HEADER;
    let chain_count = presentation.chain_count();
    let effect_words: usize = (0..chain_count)
        .map(|i| {
            presentation
                .chain(i)
                .iter()
                .filter(|effect| effect.is_effective())
                .count()
                * 4
        })
        .sum();
    let clusters_at = effects_at + chain_count * 2 + effect_words;
    fx.extend_from_slice(&[
        flags,
        clusters as u32,
        start_secs,
        start_fraction.to_bits(),
        reveal.and_then(|reveal| reveal.limit).unwrap_or(u32::MAX),
        u32::from(intro.fade) | u32::from(intro.pop) << 1 | easing << 8,
        intro.rise_px.to_bits(),
        intro.duration_s.max(0.0).to_bits(),
        effects_at as u32,
        clusters_at as u32,
        chain_count as u32,
        // Which way "up" is: 1 when the line is vertical and its line-over
        // side is the page's right (`glyph_over`).
        u32::from(presentation.writing_mode.is_vertical()),
    ]);
    fx.resize(at + HEADER, 0);
    let enter = transitions.entrance.as_ref();
    let exit = transitions.exit.as_ref();
    let (exit_secs, exit_fraction) = nana_ui_core::motion::glyph::split_seconds(
        exit.map_or(std::time::Duration::ZERO, |e| e.start),
    );
    let enter_start = enter
        .map_or(std::time::Duration::ZERO, |m| m.start)
        .max(reveal.map_or(std::time::Duration::ZERO, |r| r.start));
    let (enter_secs, enter_fraction) = nana_ui_core::motion::glyph::split_seconds(enter_start);
    let freeze = transitions
        .freeze_reveal
        .map(nana_ui_core::motion::glyph::split_seconds);
    let viewport = transitions
        .viewport
        .unwrap_or(nana_ui_core::LogicalRect::new(0., 0., 1., 1.));
    fx[at + 12..at + 32].copy_from_slice(&[
        u32::from(enter.is_some()),
        u32::from(exit.is_some()),
        enter.map_or(0, |t| {
            u32::from(t.space == nana_ui_core::PresentationMaskSpace::Target)
        }),
        exit.map_or(0, |t| {
            u32::from(t.space == nana_ui_core::PresentationMaskSpace::Target)
        }),
        enter.map_or(0., |t| t.duration.as_secs_f32()).to_bits(),
        exit_secs,
        exit_fraction.to_bits(),
        exit.map_or(0., |t| t.duration.as_secs_f32()).to_bits(),
        u32::from(freeze.is_some()),
        freeze.map_or(0, |f| f.0),
        freeze.map_or(0., |f| f.1).to_bits(),
        viewport.x.to_bits(),
        viewport.y.to_bits(),
        viewport.width.to_bits(),
        viewport.height.to_bits(),
        enter_secs,
        enter_fraction.to_bits(),
        0,
        0,
        0,
    ]);
    let mut next = effects_at + chain_count * 2;
    for i in 0..chain_count {
        let count = presentation
            .chain(i)
            .iter()
            .filter(|effect| effect.is_effective())
            .count();
        fx.extend_from_slice(&[next as u32, count as u32]);
        next += count * 4;
    }
    for effect in (0..chain_count)
        .flat_map(|i| presentation.chain(i))
        .filter(|effect| effect.is_effective())
    {
        let kind = match effect.kind {
            nana_ui_core::GlyphEffectKind::Shake => 0u32,
            nana_ui_core::GlyphEffectKind::Wave => 1,
            nana_ui_core::GlyphEffectKind::Jump => 2,
            nana_ui_core::GlyphEffectKind::Rainbow => 3,
            nana_ui_core::GlyphEffectKind::Pulse => 4,
            nana_ui_core::GlyphEffectKind::Flicker => 5,
        };
        fx.extend_from_slice(&[
            kind,
            effect.amplitude.to_bits(),
            effect.frequency_hz.to_bits(),
            effect.stagger.to_bits(),
        ]);
    }
    for ordinal in 0..clusters {
        let effect = presentation
            .cluster_effects
            .get(ordinal)
            .map_or(u32::MAX, |index| {
                if *index == u16::MAX {
                    u32::MAX
                } else {
                    u32::from(*index)
                }
            });
        let at = reveal.map_or(0.0, |reveal| {
            reveal.at_s.get(ordinal).copied().unwrap_or(f32::INFINITY)
        });
        fx.extend_from_slice(&[effect, at.to_bits()]);
    }
    at as u32
}

/// A legacy `text_shadow` as the effects every other shadow is drawn from.
pub(super) fn legacy_effects(shadow: nana_ui_core::TextShadowSpec) -> Option<Arc<SceneRichPaint>> {
    SceneRichPaint::new(
        SceneTextEffects {
            shadows: Arc::from([nana_ui_core::RichTextShadow {
                offset: [shadow.offset_x, shadow.offset_y],
                blur_px: shadow.blur_radius.max(0.0),
                spread_px: 0.0,
                color: shadow.color,
            }]),
            ..SceneTextEffects::default()
        },
        Vec::new(),
    )
}

pub(super) fn legacy_shadow_fingerprint(shadow: nana_ui_core::TextShadowSpec) -> u64 {
    let mut hasher = ShapeHasher::default();
    for value in [shadow.offset_x, shadow.offset_y, shadow.blur_radius] {
        value.to_bits().hash(&mut hasher);
    }
    hash_paint_color(Some(shadow.color), &mut hasher);
    hasher.finish() | 1
}

/// One entry stamp for its colours and its effects. Zero only when both are:
/// the solid case a recolour does not rebuild.
pub(super) fn combine_fingerprints(colors: u64, effects: u64) -> u64 {
    if effects == 0 {
        return colors;
    }
    let mut hasher = ShapeHasher::default();
    colors.hash(&mut hasher);
    effects.hash(&mut hasher);
    hasher.finish() | 1
}

/// `fills` in paint order with their effects, into `out`: every shadow layer
/// (the last CSS layer lowest), outlines that sit under the fill, the fills,
/// outlines over them. Returns the index of the first fill run, which is
/// where underlines go.
///
/// A glyph that is stroked or shadowed is filled from the outline scaler its
/// stroke and shadow come from, so the three line up to the pixel.
pub(super) fn layer_effects(
    fills: &NanaGlyphBuffer,
    effects: &SceneRichPaint,
    scale: f32,
    out: &mut NanaGlyphBuffer,
) -> usize {
    out.clear();
    let push = |out: &mut NanaGlyphBuffer,
                run: &NanaGlyphRun,
                synthesis: GlyphSynthesis,
                mode: GlyphRenderMode,
                color: nana_ui_core::PaintColor,
                role: GlyphRole,
                glyph: PlacedGlyph| {
        out.push(
            run.font,
            run.font_generation,
            run.variation,
            run.size_bits,
            synthesis,
            mode,
            color.to_srgb(),
            Some(color),
            role,
            glyph,
        );
    };
    // Only an outline has to line up with its fill to the pixel; a shadow is
    // offset and soft anyway, and keeps the platform rasterizer's fill.
    let outlined = |fx: &SceneTextEffects| fx.stroke.is_some();
    for layer in (0..effects.max_shadows()).rev() {
        for run in &fills.runs {
            for glyph in fills.glyphs_of(run) {
                let fx = effects.effects_at(glyph.cluster as usize);
                let Some(shadow) = fx.shadows.get(layer) else {
                    continue;
                };
                let blur = shadow.blur_px.clamp(0.0, MAX_SHADOW_BLUR) * scale;
                let spread = shadow.spread_px.max(0.0) * scale;
                let mode = if blur < 0.5 && spread <= 0.0 {
                    GlyphRenderMode::Mask
                } else {
                    GlyphRenderMode::Blur {
                        radius_q: GlyphRenderMode::quarters(blur),
                        spread_q: GlyphRenderMode::quarters(spread),
                    }
                };
                // A sharp shadow is the fill's own bitmap, shifted.
                let synthesis = if mode.is_derived() || outlined(fx) {
                    run.synthesis.with(GlyphSynthesis::OUTLINE_RASTER)
                } else {
                    run.synthesis
                };
                push(
                    out,
                    run,
                    synthesis,
                    mode,
                    shadow.color,
                    GlyphRole::Shadow,
                    PlacedGlyph {
                        x: glyph.x + shadow.offset[0] * scale,
                        y: glyph.y + shadow.offset[1] * scale,
                        ..*glyph
                    },
                );
            }
        }
    }
    let strokes = |out: &mut NanaGlyphBuffer, placement: nana_ui_core::TextStrokePlacement| {
        for run in &fills.runs {
            for glyph in fills.glyphs_of(run) {
                let fx = effects.effects_at(glyph.cluster as usize);
                let Some(stroke) = fx.stroke.filter(|stroke| stroke.placement == placement) else {
                    continue;
                };
                let width = stroke.width_px.max(0.0) * scale;
                if width < 0.25 {
                    continue;
                }
                push(
                    out,
                    run,
                    run.synthesis.with(GlyphSynthesis::OUTLINE_RASTER),
                    GlyphRenderMode::Stroke {
                        width_q: GlyphRenderMode::quarters(width),
                        join: raster::stroke_join(stroke.join),
                    },
                    stroke.color,
                    GlyphRole::Stroke,
                    *glyph,
                );
            }
        }
    };
    strokes(out, nana_ui_core::TextStrokePlacement::Under);
    let fill_start = out.runs.len();
    for run in &fills.runs {
        for glyph in fills.glyphs_of(run) {
            let fx = effects.effects_at(glyph.cluster as usize);
            let synthesis = if outlined(fx) {
                run.synthesis.with(GlyphSynthesis::OUTLINE_RASTER)
            } else {
                run.synthesis
            };
            out.push(
                run.font,
                run.font_generation,
                run.variation,
                run.size_bits,
                synthesis,
                run.render_mode,
                run.color,
                run.paint_color,
                GlyphRole::Fill,
                *glyph,
            );
        }
    }
    strokes(out, nana_ui_core::TextStrokePlacement::Over);
    fill_start
}

/// Underlines and strikeouts along each line's runs, from the faces' own
/// `post` / `OS/2` metrics, merged across neighbouring glyphs that decorate
/// alike. Positions are in the entry's raster px, on whole pixels.
pub(super) fn decoration_quads(
    layout: &TextLayout,
    effects: &SceneRichPaint,
    colors: &SpanColors,
    scale: f32,
    origin: [f32; 2],
    out: &mut Vec<SolidQuad>,
) {
    #[derive(Clone, Copy, PartialEq)]
    struct Key {
        decoration: nana_ui_core::TextDecorationLine,
        color: [u32; 4],
        paint_color: Option<nana_ui_core::PaintColor>,
    }
    struct Open {
        key: Key,
        color: [f32; 4],
        left: f32,
        right: f32,
        cluster: u32,
    }
    for line in &layout.lines {
        let baseline = (line.metrics.baseline_y_px * scale).round() + origin[1].floor();
        for run in layout.line_runs(line) {
            // An inline object's placeholder is not text a line runs under.
            if run.is_object() {
                continue;
            }
            let metrics = run.metrics;
            let size = run.font_size_px;
            let (underline_offset, underline_thickness, strike_offset, strike_thickness) =
                if metrics.has_decorations() {
                    (
                        metrics.underline_offset_px,
                        metrics.underline_thickness_px,
                        metrics.strikeout_offset_px,
                        metrics.strikeout_thickness_px,
                    )
                } else {
                    (size * 0.1, size / 14.0, size * 0.3, size / 14.0)
                };
            let flush = |open: Option<Open>, out: &mut Vec<SolidQuad>| {
                let Some(open) = open else {
                    return;
                };
                let left = (open.left * scale + origin[0]).round();
                let right = (open.right * scale + origin[0]).round();
                let width = (right - left).max(1.0) as u32;
                let mut emit = |top: f32, thickness: f32, line_through: bool| {
                    out.push(SolidQuad {
                        origin: [left as i32, top as i32],
                        size: [width, (thickness * scale).round().max(1.0) as u32],
                        color: open.color,
                        paint_color: open.key.paint_color,
                        line_through,
                        cluster: open.cluster,
                    });
                };
                if open.key.decoration.underline {
                    emit(
                        baseline + (underline_offset * scale).round(),
                        underline_thickness,
                        false,
                    );
                }
                if open.key.decoration.line_through {
                    emit(
                        baseline - (strike_offset * scale).round(),
                        strike_thickness,
                        true,
                    );
                }
            };
            let mut open: Option<Open> = None;
            for (left, glyph) in run.glyph_cells() {
                let fx = effects.effects_at(glyph.cluster as usize);
                if !fx.decoration.is_active() {
                    flush(open.take(), out);
                    continue;
                }
                let (color, paint_color) = match fx.decoration_color {
                    Some(color) => (color.to_srgb(), Some(color)),
                    None => {
                        let fill = colors.color_at(glyph.cluster as usize);
                        (fill.srgb, fill.paint_color)
                    }
                };
                let key = Key {
                    decoration: fx.decoration,
                    color: color.map(f32::to_bits),
                    paint_color,
                };
                let right = left + glyph.advance_px;
                match open.as_mut() {
                    Some(current) if current.key == key && (current.right - left).abs() < 0.01 => {
                        current.right = right;
                    }
                    _ => {
                        flush(open.take(), out);
                        open = Some(Open {
                            key,
                            color,
                            left,
                            right,
                            cluster: glyph.cluster,
                        });
                    }
                }
            }
            flush(open.take(), out);
        }
    }
}

/// The page size of a paragraph: the widest line and the summed line boxes,
/// crossed over for vertical text.
pub(super) fn measure(layout: &TextLayout) -> (f32, f32) {
    layout.physical_size()
}

pub(super) fn text_box_origin(
    bounds: LogicalRect,
    vertical: TextVerticalAlignment,
    laid_out_height: f32,
) -> [f32; 2] {
    nana_ui_runtime::scene_contract::text_layout_origin(
        nana_ui_runtime::LayoutBox {
            x: bounds.x,
            y: bounds.y,
            width: bounds.width,
            height: bounds.height,
        },
        vertical,
        laid_out_height,
    )
}

/// The run row's color, quantized exactly as an instance's four bytes would
/// be.
///
/// A glyph inherits the row whenever its own packed color matches the
/// paragraph's, so the two must agree to the bit — otherwise a rich span that
/// happens to paint the default color would shift by a least significant bit
/// when it stopped carrying its own.
pub(super) fn run_color(color: [f32; 4]) -> [f32; 4] {
    let [r, g, b, a] = to_rgba8(color);
    [
        linear_from_srgb8(r),
        linear_from_srgb8(g),
        linear_from_srgb8(b),
        f32::from(a) / 255.0,
    ]
}

/// The axis-aligned box `ink` covers once its node's homography is applied.
pub(super) fn transformed_ink(ink: LogicalRect, affine: [f32; 6], persp: [f32; 2]) -> LogicalRect {
    clip::transformed_aabb_projective(ink, affine, persp)
}
