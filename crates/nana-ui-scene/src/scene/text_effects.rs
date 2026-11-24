//! Text paint data: spans, CSS text effects, rich paint runs, glyph
//! presentation and OpenType settings.

use super::*;

/// One paint-ready span inside a [`ScenePrimitiveKind::Text`] primitive.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneTextSpan {
    pub start: usize,
    pub end: usize,
    /// In the space it was authored in, converted once at GPU upload.
    pub color: nana_ui_core::PaintColor,
}

/// What a stretch of a [`ScenePrimitiveKind::Text`] draws besides its fill:
/// decoration lines, an outline, shadow layers. The fill colour itself rides
/// in [`ScenePrimitiveKind::Text::spans`], like any other coloured run.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SceneTextEffects {
    pub material: Option<nana_ui_core::MaterialRef>,
    pub decoration: nana_ui_core::TextDecorationLine,
    /// `None` draws the lines in the glyphs' own fill.
    pub decoration_color: Option<nana_ui_core::PaintColor>,
    pub stroke: Option<nana_ui_core::RichTextStroke>,
    /// CSS order: the first is drawn on top. At most
    /// [`nana_ui_core::MAX_TEXT_SHADOWS`].
    pub shadows: Arc<[nana_ui_core::RichTextShadow]>,
    /// Emphasis mark colour; `None` draws the marks in the glyphs' own fill
    /// (`currentColor`). The marks themselves are in the paragraph's layout.
    pub emphasis_color: Option<nana_ui_core::PaintColor>,
}

impl SceneTextEffects {
    pub fn is_empty(&self) -> bool {
        self.material.is_none()
            && self.emphasis_color.is_none()
            && !self.decoration.is_active()
            && self.stroke.is_none()
            && self.shadows.is_empty()
    }

    /// Logical px these effects can reach past the glyphs' own ink.
    pub fn reach(&self) -> f32 {
        let shadows = self
            .shadows
            .iter()
            .map(|shadow| {
                shadow.offset[0].abs().max(shadow.offset[1].abs())
                    + shadow.blur_px.max(0.0)
                    + shadow.spread_px.max(0.0)
            })
            .fold(0.0f32, f32::max);
        let stroke = self
            .stroke
            .map_or(0.0, |stroke| stroke.width_px.max(0.0) * 0.5);
        shadows.max(stroke) + stroke
    }

    /// `over` laid on these effects: each one `over` sets wins.
    pub fn overlay(&self, over: &nana_ui_core::RichPaintStyle) -> Self {
        Self {
            material: over.material.clone().or_else(|| self.material.clone()),
            decoration: over.decoration.unwrap_or(self.decoration),
            decoration_color: over.decoration_color.or(self.decoration_color),
            stroke: over.stroke.or(self.stroke),
            shadows: over
                .shadows
                .clone()
                .unwrap_or_else(|| Arc::clone(&self.shadows)),
            emphasis_color: over.emphasis_color.or(self.emphasis_color),
        }
    }
}

/// What a Text primitive presents per glyph, by grapheme ordinal: the
/// effect table, which effect each grapheme plays, and the reveal.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneGlyphPresentation {
    pub transitions: nana_ui_core::TextTransitions,
    pub effects: Arc<[nana_ui_core::GlyphEffect]>,
    pub effect_chains: Arc<[nana_ui_core::GlyphEffectChain]>,
    /// Effect index per grapheme of the content; `u16::MAX` for none. Empty
    /// when no grapheme plays an effect.
    pub cluster_effects: Arc<[u16]>,
    pub reveal: Option<nana_ui_core::RevealSchedule>,
    /// The writing mode of the text it presents, the node's own: which page
    /// direction a wave, a jump and a rise take
    /// ([`nana_ui_core::evaluate_glyph_chain_in`]).
    pub writing_mode: WritingModeSpec,
}

impl SceneGlyphPresentation {
    /// The presentation of `content`, set in `writing_mode`, under
    /// `presentation`, its graphemes' effect indices read from `rich`'s spans.
    pub fn new(
        content: &str,
        rich: Option<&nana_ui_core::RichText>,
        presentation: &nana_ui_core::GlyphPresentation,
        writing_mode: WritingModeSpec,
    ) -> Option<Arc<Self>> {
        use unicode_segmentation::UnicodeSegmentation;
        let mut any = false;
        let cluster_effects: Vec<u16> = match rich.filter(|rich| rich.text() == content) {
            Some(rich)
                if !presentation.effects.is_empty() || !presentation.effect_chains.is_empty() =>
            {
                content
                    .grapheme_indices(true)
                    .map(
                        |(offset, _)| match rich.style_at(offset).and_then(|style| style.effect) {
                            Some(effect)
                                if presentation
                                    .effect_chain(usize::from(effect))
                                    .iter()
                                    .any(nana_ui_core::GlyphEffect::is_effective) =>
                            {
                                any = true;
                                effect
                            }
                            _ => u16::MAX,
                        },
                    )
                    .collect()
            }
            _ => Vec::new(),
        };
        if !any && presentation.reveal.is_none() && presentation.transitions == Default::default() {
            return None;
        }
        Some(Arc::new(Self {
            effects: Arc::clone(&presentation.effects),
            effect_chains: Arc::clone(&presentation.effect_chains),
            cluster_effects: if any {
                cluster_effects.into()
            } else {
                Arc::from([])
            },
            transitions: presentation.transitions.clone(),
            reveal: presentation.reveal.clone(),
            writing_mode,
        }))
    }

    /// Grapheme `ordinal`'s look at `now`: what the text shader draws it as.
    pub fn sample(
        &self,
        reveal: Option<&nana_ui_core::RevealSchedule>,
        ordinal: u32,
        now: std::time::Duration,
    ) -> nana_ui_core::GlyphSample {
        nana_ui_core::evaluate_glyph_chain_in(
            self.effects_of(ordinal as usize),
            reveal,
            ordinal,
            now,
            self.writing_mode,
        )
    }

    /// Until when it changes what is drawn; `None` while an effect loops.
    pub fn live_until(&self) -> Option<std::time::Duration> {
        if self.transitions.exit.is_some() {
            return Some(self.transitions.end());
        }
        if self
            .cluster_effects
            .iter()
            .filter(|index| **index != u16::MAX)
            .any(|index| {
                self.chain(usize::from(*index))
                    .iter()
                    .any(nana_ui_core::GlyphEffect::is_animated)
            })
        {
            return None;
        }
        Some(
            self.reveal
                .as_ref()
                .map_or(std::time::Duration::ZERO, |reveal| {
                    self.transitions.reveal_end(reveal)
                }),
        )
    }

    pub fn effects_of(&self, ordinal: usize) -> &[nana_ui_core::GlyphEffect] {
        let Some(index) = self
            .cluster_effects
            .get(ordinal)
            .filter(|index| **index != u16::MAX)
        else {
            return &[];
        };
        self.chain(usize::from(*index))
    }

    pub fn chain(&self, index: usize) -> &[nana_ui_core::GlyphEffect] {
        if self.effect_chains.is_empty() {
            self.effects
                .get(index)
                .map(std::slice::from_ref)
                .unwrap_or(&[])
        } else {
            self.effect_chains
                .get(index)
                .map(|chain| chain.0.as_ref())
                .unwrap_or(&[])
        }
    }

    pub fn chain_count(&self) -> usize {
        if self.effect_chains.is_empty() {
            self.effects.len()
        } else {
            self.effect_chains.len()
        }
    }

    /// The effect grapheme `ordinal` plays.
    pub fn effect_of(&self, ordinal: usize) -> Option<&nana_ui_core::GlyphEffect> {
        self.effects_of(ordinal).first()
    }
}

/// One byte range of a Text primitive with effects of its own.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneRichRun {
    pub range: std::ops::Range<usize>,
    pub effects: SceneTextEffects,
}

/// Everything a Text primitive draws besides its fill, resolved: the node's
/// own effects (CSS `text-decoration`, `text-shadow`, `-webkit-text-stroke`)
/// and the ranges a rich text overrides them over.
///
/// The painter turns this into instances of the same paragraph — shadows,
/// then strokes, then fills, then lines — so a paint change is a rebuild of
/// those instances and never a relayout.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneRichPaint {
    pub base: SceneTextEffects,
    /// Sorted, non-overlapping.
    pub runs: Vec<SceneRichRun>,
    /// A hash of all of the above. Equal effects have equal revisions, so the
    /// painter can tell a repaint that changed nothing from one that did
    /// without comparing them. Never zero.
    pub revision: u64,
    /// Logical px the effects can reach past the glyphs' own ink.
    pub reach: f32,
}

impl SceneRichPaint {
    /// `None` when nothing has an effect.
    pub fn new(base: SceneTextEffects, runs: Vec<SceneRichRun>) -> Option<Arc<Self>> {
        let runs: Vec<SceneRichRun> = runs
            .into_iter()
            .filter(|run| run.range.start < run.range.end && run.effects != base)
            .collect();
        if base.is_empty() && runs.iter().all(|run| run.effects.is_empty()) {
            return None;
        }
        let reach = runs
            .iter()
            .map(|run| run.effects.reach())
            .fold(base.reach(), f32::max);
        let mut hasher = std::hash::DefaultHasher::new();
        hash_effects(&base, &mut hasher);
        for run in &runs {
            std::hash::Hash::hash(&run.range, &mut hasher);
            hash_effects(&run.effects, &mut hasher);
        }
        let revision = std::hash::Hasher::finish(&hasher) | 1;
        Some(Arc::new(Self {
            base,
            runs,
            revision,
            reach,
        }))
    }

    /// The effects governing byte `offset`.
    pub fn effects_at(&self, offset: usize) -> &SceneTextEffects {
        let index = self.runs.partition_point(|run| run.range.end <= offset);
        self.runs
            .get(index)
            .filter(|run| run.range.start <= offset)
            .map_or(&self.base, |run| &run.effects)
    }

    pub(super) fn all(&self) -> impl Iterator<Item = &SceneTextEffects> {
        std::iter::once(&self.base).chain(self.runs.iter().map(|run| &run.effects))
    }

    /// The most shadow layers any range draws.
    pub fn max_shadows(&self) -> usize {
        self.all()
            .map(|effects| effects.shadows.len())
            .max()
            .unwrap_or(0)
    }

    pub fn has_stroke(&self) -> bool {
        self.all().any(|effects| effects.stroke.is_some())
    }

    pub fn has_decoration(&self) -> bool {
        self.all().any(|effects| effects.decoration.is_active())
    }

    /// A node's effects from its resolved CSS, with `rich`'s paint tier over
    /// them.
    pub fn from_style(
        style: &nana_ui_core::LayoutStyle,
        rich: Option<&nana_ui_core::RichText>,
    ) -> Option<Arc<Self>> {
        Self::from_base(css_text_effects(style), rich)
    }

    /// `base` with `rich`'s paint tier over it.
    pub fn from_base(
        base: SceneTextEffects,
        rich: Option<&nana_ui_core::RichText>,
    ) -> Option<Arc<Self>> {
        let runs = rich
            .map(|rich| {
                rich.spans()
                    .iter()
                    .filter(|(_, span)| {
                        let paint = &span.paint;
                        paint.material.is_some()
                            || paint.decoration.is_some()
                            || paint.decoration_color.is_some()
                            || paint.stroke.is_some()
                            || paint.shadows.is_some()
                            || paint.emphasis_color.is_some()
                    })
                    .map(|(range, span)| SceneRichRun {
                        range,
                        effects: base.overlay(&span.paint),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self::new(base, runs)
    }
}

fn hash_paint(color: &nana_ui_core::PaintColor, hasher: &mut impl std::hash::Hasher) {
    let (channels, alpha) = color.to_linear_sc_rgb();
    for value in [channels[0], channels[1], channels[2], alpha] {
        hasher.write_u32(value.to_bits());
    }
}

fn hash_effects(effects: &SceneTextEffects, hasher: &mut impl std::hash::Hasher) {
    hasher.write_u64(effects.material.as_ref().map_or(0, |m| m.key()));
    hasher.write_u8(
        u8::from(effects.decoration.underline) | u8::from(effects.decoration.line_through) << 1,
    );
    if let Some(color) = &effects.decoration_color {
        hash_paint(color, hasher);
    }
    hasher.write_u8(0xfe);
    if let Some(stroke) = &effects.stroke {
        hasher.write_u32(stroke.width_px.to_bits());
        hash_paint(&stroke.color, hasher);
        hasher.write_u8(stroke.join as u8);
        hasher.write_u8(stroke.placement as u8);
    }
    hasher.write_u8(0xfd);
    for shadow in effects.shadows.iter() {
        for value in [
            shadow.offset[0],
            shadow.offset[1],
            shadow.blur_px,
            shadow.spread_px,
        ] {
            hasher.write_u32(value.to_bits());
        }
        hash_paint(&shadow.color, hasher);
    }
    hasher.write_usize(effects.shadows.len());
    if let Some(color) = &effects.emphasis_color {
        hash_paint(color, hasher);
    }
    hasher.write_u8(0xfc);
}

/// A node's text effects as its CSS states them: `text-decoration`, every
/// `text-shadow` layer, `-webkit-text-stroke` under `paint-order`.
pub fn css_text_effects(style: &nana_ui_core::LayoutStyle) -> SceneTextEffects {
    let shadows: Arc<[nana_ui_core::RichTextShadow]> = style
        .paint
        .text_shadows
        .iter()
        .take(nana_ui_core::MAX_TEXT_SHADOWS)
        .map(|shadow| nana_ui_core::RichTextShadow {
            offset: [shadow.offset_x, shadow.offset_y],
            blur_px: shadow.blur_radius.max(0.0),
            spread_px: 0.0,
            color: shadow.color,
        })
        .collect();
    let fill = style.paint_colors.color;
    let stroke = style
        .paint
        .text_stroke
        .filter(|stroke| stroke.width > 0.0 && stroke.width.is_finite())
        .and_then(|stroke| {
            let color = stroke.color.or(fill)?;
            Some(nana_ui_core::RichTextStroke {
                width_px: stroke.width,
                color,
                join: nana_ui_core::TextStrokeJoin::Miter,
                placement: if style.paint.paint_order_stroke_first {
                    nana_ui_core::TextStrokePlacement::Under
                } else {
                    nana_ui_core::TextStrokePlacement::Over
                },
            })
        });
    let text_ext = &*style.text_ext;
    SceneTextEffects {
        material: style.paint.text_material.clone(),
        decoration: text_ext.text_decoration.unwrap_or_default(),
        decoration_color: None,
        stroke,
        shadows,
        emphasis_color: text_ext
            .text_emphasis_color
            .and_then(nana_ui_core::TextEmphasisColor::paint),
    }
}

/// OpenType and wrap extras on a [`ScenePrimitiveKind::Text`] run.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SceneTextOpenType {
    pub features: Vec<FontFeatureSetting>,
    pub variations: Vec<FontVariationSetting>,
    pub kerning: FontKerningSpec,
    pub word_break: WordBreakSpec,
    pub line_break: LineBreakSpec,
    /// CSS `direction` after inherit. Drives the same RLI/PDI wrap as shaping.
    pub direction: DirSpec,
    /// Whether an authored newline is a line break rather than a space.
    ///
    /// CSS `white-space` decides it, and a multiline editor's value keeps its
    /// breaks whatever `white-space` says. The renderer cannot derive it from
    /// anything else here, and getting it wrong measures one line and paints
    /// two.
    pub preserve_lines: bool,
    /// CSS `writing-mode` after inherit. Vertical text — an editor's value
    /// included — is laid out and painted in columns (#59).
    pub writing_mode: WritingModeSpec,
    /// CSS `text-orientation` after inherit (#59).
    pub text_orientation: nana_ui_core::TextOrientationSpec,
    /// The language the Runtime shaped this text in (BCP 47). A renderer that
    /// lays the text out again shapes in the same one, or its `locl` forms
    /// and fallback faces would differ from what was measured.
    pub language: Option<nana_ui_runtime::LanguageTag>,
    /// The CJK line decision the Runtime laid this text out with (#211):
    /// punctuation trim, autospace, justification and pretty wrapping. A
    /// renderer laying the text out again hands the engine the same.
    pub typography: nana_ui_core::UsedTextTypography,
}

impl SceneTextOpenType {
    pub fn from_computed(style: &nana_ui_runtime::ComputedStyle) -> Self {
        Self {
            features: style.font_features.clone(),
            variations: style.font_variations.clone(),
            kerning: style.font_kerning,
            word_break: style.word_break,
            line_break: style.line_break,
            // The used direction: `text-orientation: upright` reads `ltr`.
            direction: style.writing_context().direction,
            writing_mode: style.writing_mode,
            text_orientation: style.text_orientation,
            language: style.language.clone(),
            typography: style.used_text_typography(),
            // Not on `ComputedStyle`: `white-space` is a box-layout property,
            // so the caller that has the layout style sets it.
            preserve_lines: false,
        }
    }
}
