//! The shaping pipeline and its cache.

use super::bidi;
use super::cache::{ShapeCache, ShapeCacheBudget};
use super::key::{FontEpoch, ShapeKey};
use super::opentype::{FaceShapers, RawGlyph, ShapeInput};
use super::{ShapeCounters, ShapeRequest, ShapedText};
use crate::font::unicode;
use crate::font::{
    FontFeatures, FontInstance, FontQuery, FontSelection, FontSystem, FontVariations, LanguageTag,
};
use crate::id::{FontGeneration, FontId, ShapeRunId};
use crate::shape::{GlyphFlags, RunDirection, ScriptTag, ShapedGlyph, ShapedRun};
use crate::source::TextSpan;
use crate::style::TextStyle;
use nana_ui_core::{DirSpec, FontKerningSpec};
use std::ops::Range;
use std::sync::Arc;

/// Faces tried for one missing cluster range before it is left as `.notdef`.
pub const MAX_FALLBACK_CANDIDATES_PER_RANGE: usize = 8;
/// Retries per shaped item. Together with the per-range limit this keeps a
/// string no face covers from costing text length × face count.
pub const MAX_FALLBACK_RETRIES_PER_ITEM: usize = 32;

/// Shapes [`ShapeRequest`]s against a [`FontSystem`] and caches the results.
pub struct Shaper {
    cache: ShapeCache,
    faces: FaceShapers,
    counters: ShapeCounters,
    /// The font system and generation `faces` and the cache were built from.
    epoch: Option<FontEpoch>,
    next_run: u32,
}

impl Default for Shaper {
    fn default() -> Self {
        Self::new(ShapeCacheBudget::default())
    }
}

/// A style segment: a byte range of the text and the style that applies.
struct StyleSegment<'a> {
    range: Range<usize>,
    style: &'a TextStyle,
    query: FontQuery,
    selection: Arc<FontSelection>,
}

/// A maximal range shaped in one HarfRust call.
struct Item {
    range: Range<usize>,
    segment: usize,
    font: Option<FontId>,
    level: u8,
    script: Option<ScriptTag>,
}

/// A piece of an item after fallback: its range, the face it shapes with, the
/// faces already ruled out for it, and its shaping once final.
struct Piece {
    range: Range<usize>,
    font: FontId,
    tried: Vec<FontId>,
    shaped: Option<(Vec<RawGlyph>, Option<FontInstance>)>,
}

fn is_paragraph_separator(cluster: &str) -> bool {
    matches!(cluster, "\n" | "\r\n" | "\r" | "\u{2029}" | "\u{85}")
}

impl Shaper {
    pub fn new(budget: ShapeCacheBudget) -> Self {
        Self {
            cache: ShapeCache::new(budget),
            faces: FaceShapers::default(),
            counters: ShapeCounters::default(),
            epoch: None,
            next_run: 0,
        }
    }

    pub fn counters(&self) -> ShapeCounters {
        ShapeCounters {
            shape_cache_bytes: self.cache.bytes(),
            shape_cache_entries: self.cache.len(),
            ..self.counters
        }
    }

    pub fn reset_counters(&mut self) {
        self.counters = ShapeCounters::default();
    }

    pub fn set_budget(&mut self, budget: ShapeCacheBudget) {
        self.counters.shape_cache_evictions += self.cache.set_budget(budget);
    }

    /// Shapes `request`, from the cache when an equal request was shaped
    /// against the same font system at its current generation. One shaper may
    /// serve several systems in turn; switching drops the other system's
    /// entries and per-face data rather than mixing their `FontId`s.
    pub fn shape(&mut self, fonts: &mut FontSystem, request: &ShapeRequest<'_>) -> Arc<ShapedText> {
        self.counters.shape_requests += 1;
        let generation = fonts.generation();
        let epoch = (fonts.instance_id(), generation);
        if self.epoch != Some(epoch) {
            self.epoch = Some(epoch);
            self.faces.clear();
            self.counters.shape_cache_evictions += self.cache.purge_other_epochs(epoch);
        }

        let (text_hash, hashed_now) = request.source.content_hash();
        if hashed_now {
            self.counters.text_bytes_hashed += request.source.text().len();
        }
        let scale = request.scale.px_per_logical;
        let rtl = request.direction == DirSpec::Rtl;
        let key = ShapeKey::new(
            request.source,
            text_hash,
            request.style,
            request.source.spans(),
            rtl,
            request.language,
            scale,
            epoch,
        );
        if let Some(hit) = self.cache.get(&key) {
            self.counters.shape_cache_hits += 1;
            return hit;
        }
        self.counters.shape_cache_misses += 1;

        let shaped = Arc::new(self.shape_uncached(fonts, request, rtl, scale, generation));
        self.counters.shape_runs_created += shaped.runs.len();
        self.counters.shape_glyphs_created += shaped.glyph_count();
        self.counters.shape_cache_evictions += self.cache.insert(key, Arc::clone(&shaped));
        shaped
    }

    fn shape_uncached(
        &mut self,
        fonts: &mut FontSystem,
        request: &ShapeRequest<'_>,
        rtl: bool,
        scale: f32,
        generation: FontGeneration,
    ) -> ShapedText {
        let text = request.source.text();
        let mut levels = bidi::resolve(text, rtl);
        let paragraphs = std::mem::take(&mut levels.paragraphs);
        if text.is_empty() {
            return ShapedText {
                runs: Vec::new(),
                paragraphs,
                font_generation: generation,
            };
        }

        let clusters = unicode::clusters(text);
        let boundaries: Vec<usize> = clusters.iter().map(|cluster| cluster.range.start).collect();
        let segments = style_segments(
            fonts,
            text,
            &boundaries,
            request.style,
            request.source.spans(),
            request.language,
        );

        // Scripts: each cluster's own, Common/Inherited taking the preceding
        // specific script, leading ones the first following.
        let mut scripts: Vec<Option<ScriptTag>> = Vec::with_capacity(clusters.len());
        let mut previous = None;
        for cluster in &clusters {
            if cluster.script.is_some() {
                previous = cluster.script;
            }
            scripts.push(cluster.script.or(previous));
        }
        let first_script = clusters.iter().find_map(|cluster| cluster.script);
        for script in scripts.iter_mut() {
            if script.is_some() {
                break;
            }
            *script = first_script;
        }

        // Fonts: the #90 coverage choice for every cluster, per style segment.
        // A cluster nothing covers renders the primary's `.notdef`; when the
        // family list resolved to no face at all, the first registered face
        // stands in, so the text still yields `MISSING` glyphs instead of
        // silently vanishing. Only an empty font system leaves it unshaped.
        let last_resort = fonts.faces().first().copied();
        let mut cluster_fonts: Vec<Option<FontId>> = vec![None; clusters.len()];
        for segment in &segments {
            let assignments = fonts.resolve_text(
                &segment.selection,
                &text[segment.range.clone()],
                request.language,
            );
            let mut cursor = boundaries.partition_point(|start| *start < segment.range.start);
            for assignment in assignments {
                let end = segment.range.start + assignment.range.end;
                while cursor < clusters.len() && boundaries[cursor] < end {
                    cluster_fonts[cursor] = assignment
                        .font
                        .or(segment.selection.primary)
                        .or(last_resort);
                    cursor += 1;
                }
            }
        }

        // Items: maximal cluster sequences sharing segment, face, level and
        // script, broken at paragraph separators (which shape to nothing).
        let mut items: Vec<Item> = Vec::new();
        let mut segment_index = 0;
        let mut last_level = None;
        let mut last_script = None;
        for (index, cluster) in clusters.iter().enumerate() {
            while segments[segment_index].range.end <= cluster.range.start {
                segment_index += 1;
            }
            let level = levels.levels[cluster.range.start];
            if last_level != Some(level) {
                self.counters.bidi_runs += 1;
                last_level = Some(level);
            }
            if last_script != Some(scripts[index]) {
                self.counters.script_runs += 1;
                last_script = Some(scripts[index]);
            }
            // Separators shape to nothing; the byte gap they leave also stops
            // the neighbouring items from merging across them.
            if is_paragraph_separator(&text[cluster.range.clone()]) {
                continue;
            }
            let font = cluster_fonts[index];
            match items.last_mut() {
                Some(item)
                    if item.segment == segment_index
                        && item.font == font
                        && item.level == level
                        && item.script == scripts[index]
                        && item.range.end == cluster.range.start =>
                {
                    item.range.end = cluster.range.end;
                }
                _ => items.push(Item {
                    range: cluster.range.clone(),
                    segment: segment_index,
                    font,
                    level,
                    script: scripts[index],
                }),
            }
        }

        let mut runs = Vec::new();
        for item in items {
            let Some(font) = item.font else {
                // Only a font system with no faces at all gets here.
                self.counters.text_bytes_unshaped += item.range.len();
                continue;
            };
            let segment = &segments[item.segment];
            let pieces =
                self.fallback_pieces(fonts, text, &item, font, segment, request.language, scale);
            for piece in pieces {
                if let Some(run) = self.build_run(fonts, &item, piece, segment, scale) {
                    runs.push(run);
                }
            }
        }

        ShapedText {
            runs,
            paragraphs,
            font_generation: generation,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn shape_raw(
        &mut self,
        fonts: &FontSystem,
        text: &str,
        range: Range<usize>,
        font: FontId,
        item: &Item,
        segment: &StyleSegment<'_>,
        language: Option<&LanguageTag>,
        scale: f32,
    ) -> (Vec<RawGlyph>, Option<FontInstance>) {
        let variations = FontVariations::from_settings(&segment.style.variations);
        let instance = fonts.instance(font, &segment.query, &variations);
        let coords = instance
            .as_ref()
            .map_or(&[][..], |instance| instance.coords());
        let features = FontFeatures::from_settings(&segment.style.features);
        let input = ShapeInput {
            text,
            range,
            rtl: RunDirection::from_bidi_level(item.level).is_rtl(),
            script: item.script,
            language: language.map(LanguageTag::as_str),
            features: features.as_slice(),
            disable_kerning: segment.style.kerning == FontKerningSpec::None,
            coords,
            size_px: segment.style.font_size_px * scale,
        };
        let glyphs = self
            .faces
            .shape(font, || fonts.face_data(font), &input)
            .unwrap_or_default();
        (glyphs, instance)
    }

    /// Splits an item into pieces whose faces actually shape their clusters,
    /// retrying `.notdef` cluster ranges with the #90 candidate list.
    ///
    /// A piece is shaped; its `.notdef` cluster ranges are cut out and handed
    /// to the next candidate that covers them, and the loop repeats until every
    /// piece either shapes cleanly or has run out of candidates (then it keeps
    /// its `.notdef` glyphs, flagged `MISSING`). Split-off neighbours are
    /// reshaped, because their context changed.
    #[allow(clippy::too_many_arguments)]
    fn fallback_pieces(
        &mut self,
        fonts: &mut FontSystem,
        text: &str,
        item: &Item,
        font: FontId,
        segment: &StyleSegment<'_>,
        language: Option<&LanguageTag>,
        scale: f32,
    ) -> Vec<Piece> {
        let mut pieces = vec![Piece {
            range: item.range.clone(),
            font,
            tried: vec![font],
            shaped: None,
        }];
        let mut retries = 0;
        while pieces.iter().any(|piece| piece.shaped.is_none()) {
            let mut next = Vec::with_capacity(pieces.len());
            for piece in pieces {
                if piece.shaped.is_some() {
                    next.push(piece);
                    continue;
                }
                let shaped = self.shape_raw(
                    fonts,
                    text,
                    piece.range.clone(),
                    piece.font,
                    item,
                    segment,
                    language,
                    scale,
                );
                let missing = missing_ranges(&shaped.0, &piece.range);
                let mut replacements = Vec::with_capacity(missing.len());
                for gap in &missing {
                    let candidate = if retries < MAX_FALLBACK_RETRIES_PER_ITEM {
                        self.next_candidate(fonts, text, gap, &piece.tried, item, segment, language)
                    } else {
                        None
                    };
                    if candidate.is_some() {
                        retries += 1;
                        self.counters.fallback_retries += 1;
                    }
                    replacements.push(candidate);
                }
                if replacements.iter().all(Option::is_none) {
                    // Clean, or nothing can do better: keep this shaping,
                    // `.notdef` and all.
                    next.push(Piece {
                        shaped: Some(shaped),
                        ..piece
                    });
                    continue;
                }
                let mut cursor = piece.range.start;
                for (gap, candidate) in missing.into_iter().zip(replacements) {
                    if cursor < gap.start {
                        next.push(Piece {
                            range: cursor..gap.start,
                            font: piece.font,
                            tried: piece.tried.clone(),
                            shaped: None,
                        });
                    }
                    cursor = gap.end;
                    // A gap with no candidate stays on this face; the next pass
                    // finds no candidate again and keeps its `.notdef`.
                    let (font, tried) = match candidate {
                        Some(candidate) => {
                            let mut tried = piece.tried.clone();
                            tried.push(candidate);
                            (candidate, tried)
                        }
                        None => (piece.font, piece.tried.clone()),
                    };
                    next.push(Piece {
                        range: gap,
                        font,
                        tried,
                        shaped: None,
                    });
                }
                if cursor < piece.range.end {
                    next.push(Piece {
                        range: cursor..piece.range.end,
                        font: piece.font,
                        tried: piece.tried.clone(),
                        shaped: None,
                    });
                }
            }
            pieces = merge_pieces(next);
        }
        pieces
    }

    #[allow(clippy::too_many_arguments)]
    fn next_candidate(
        &mut self,
        fonts: &mut FontSystem,
        text: &str,
        gap: &Range<usize>,
        tried: &[FontId],
        item: &Item,
        segment: &StyleSegment<'_>,
        language: Option<&LanguageTag>,
    ) -> Option<FontId> {
        let candidates = fonts.cluster_candidates(
            &segment.selection,
            &text[gap.clone()],
            item.script,
            language,
        );
        for (candidate, _) in candidates
            .into_iter()
            .filter(|(candidate, _)| !tried.contains(candidate))
            .take(MAX_FALLBACK_CANDIDATES_PER_RANGE)
        {
            self.counters.fallback_fonts_examined += 1;
            if fonts.covers_text(candidate, &text[gap.clone()]) {
                return Some(candidate);
            }
        }
        None
    }

    fn build_run(
        &mut self,
        fonts: &FontSystem,
        item: &Item,
        piece: Piece,
        segment: &StyleSegment<'_>,
        scale: f32,
    ) -> Option<ShapedRun> {
        let (raw, instance) = piece.shaped?;
        if raw.is_empty() {
            return None;
        }
        let size_px = segment.style.font_size_px * scale;
        let letter_spacing = segment.style.letter_spacing_px * scale;

        let mut starts: Vec<u32> = raw.iter().map(|glyph| glyph.cluster).collect();
        starts.sort_unstable();
        starts.dedup();
        let cluster_end = |cluster: u32| -> u32 {
            let next = starts.partition_point(|start| *start <= cluster);
            starts.get(next).copied().unwrap_or(piece.range.end as u32)
        };
        let fallback = Some(piece.font) != segment.selection.primary;

        let glyphs: Vec<ShapedGlyph> = raw
            .iter()
            .enumerate()
            .map(|(index, glyph)| {
                let mut flags = GlyphFlags::NONE;
                flags.set(GlyphFlags::MISSING, glyph.glyph_id == 0);
                flags.set(GlyphFlags::FALLBACK_FONT, fallback);
                // Letter spacing trails each cluster: the glyph that ends it
                // in visual order takes the extra advance.
                let ends_cluster = raw
                    .get(index + 1)
                    .is_none_or(|next| next.cluster != glyph.cluster);
                let spacing = if ends_cluster { letter_spacing } else { 0.0 };
                ShapedGlyph {
                    glyph_id: glyph.glyph_id,
                    cluster: glyph.cluster,
                    cluster_end: cluster_end(glyph.cluster),
                    advance_px: glyph.x_advance + spacing,
                    advance_y_px: glyph.y_advance,
                    offset_x_px: glyph.x_offset,
                    offset_y_px: glyph.y_offset,
                    flags,
                }
            })
            .collect();

        let metrics = instance
            .as_ref()
            .map(|instance| fonts.run_metrics(instance, size_px))
            .unwrap_or_default();
        let index = self.next_run;
        self.next_run = self.next_run.wrapping_add(1);
        Some(ShapedRun {
            id: ShapeRunId::from_parts(index, 1),
            source: piece.range.clone(),
            direction: RunDirection::from_bidi_level(item.level),
            bidi_level: item.level,
            script: item.script.unwrap_or_default(),
            font: piece.font,
            font_size_px: size_px,
            advance_px: glyphs.iter().map(|glyph| glyph.advance_px).sum(),
            origin_x_px: 0.0,
            glyphs,
            metrics,
        })
    }
}

/// Cluster ranges containing a `.notdef` glyph, merged where adjacent, in
/// logical order.
fn missing_ranges(glyphs: &[RawGlyph], range: &Range<usize>) -> Vec<Range<usize>> {
    let mut starts: Vec<u32> = glyphs.iter().map(|glyph| glyph.cluster).collect();
    starts.sort_unstable();
    starts.dedup();
    let mut missing: Vec<Range<usize>> = Vec::new();
    for (index, start) in starts.iter().enumerate() {
        let is_missing = glyphs
            .iter()
            .any(|glyph| glyph.cluster == *start && glyph.glyph_id == 0);
        if !is_missing {
            continue;
        }
        let begin = *start as usize;
        let end = starts
            .get(index + 1)
            .map_or(range.end, |next| *next as usize);
        match missing.last_mut() {
            Some(last) if last.end == begin => last.end = end,
            _ => missing.push(begin..end),
        }
    }
    missing
}

/// Joins adjacent pieces on the same face so they shape as one.
fn merge_pieces(pieces: Vec<Piece>) -> Vec<Piece> {
    let mut merged: Vec<Piece> = Vec::with_capacity(pieces.len());
    for piece in pieces {
        match merged.last_mut() {
            Some(last) if last.font == piece.font && last.range.end == piece.range.start => {
                // The joined range has to be shaped as a whole.
                last.range.end = piece.range.end;
                last.shaped = None;
            }
            _ => merged.push(piece),
        }
    }
    merged
}

/// Style segments tiling the text: spans override the base style, composition
/// spans override plain ones, and every boundary is moved back to the start of
/// the grapheme cluster it falls in so no cluster is split.
fn style_segments<'a>(
    fonts: &mut FontSystem,
    text: &str,
    boundaries: &[usize],
    base: &'a TextStyle,
    spans: &'a [TextSpan],
    language: Option<&LanguageTag>,
) -> Vec<StyleSegment<'a>> {
    let snap = |byte: usize| -> usize {
        let byte = byte.min(text.len());
        if byte == text.len() {
            return byte;
        }
        let index = boundaries.partition_point(|start| *start <= byte);
        boundaries[index.saturating_sub(1)]
    };
    let mut cuts: Vec<usize> = vec![0, text.len()];
    for span in spans {
        cuts.push(snap(span.range.start));
        cuts.push(snap(span.range.end));
    }
    cuts.sort_unstable();
    cuts.dedup();

    let style_at = |start: usize| -> &'a TextStyle {
        let covering =
            |span: &&TextSpan| snap(span.range.start) <= start && start < snap(span.range.end);
        spans
            .iter()
            .rfind(|span| covering(span) && span.composition.is_some())
            .or_else(|| spans.iter().rfind(covering))
            .map_or(base, |span| &span.style)
    };

    let mut segments: Vec<StyleSegment<'a>> = Vec::new();
    for window in cuts.windows(2) {
        let range = window[0]..window[1];
        if range.is_empty() {
            continue;
        }
        let style = style_at(range.start);
        match segments.last_mut() {
            Some(last) if std::ptr::eq(last.style, style) => last.range.end = range.end,
            _ => {
                let query = FontQuery::from_style(style, language.cloned());
                let selection = fonts.select(&query);
                segments.push(StyleSegment {
                    range,
                    style,
                    query,
                    selection,
                });
            }
        }
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    fn glyph(glyph_id: u32, cluster: u32) -> RawGlyph {
        RawGlyph {
            glyph_id,
            cluster,
            x_advance: 1.0,
            y_advance: 0.0,
            x_offset: 0.0,
            y_offset: 0.0,
        }
    }

    #[test]
    fn missing_ranges_merge_adjacent_notdef_clusters_in_either_glyph_order() {
        let ltr = [
            glyph(5, 0),
            glyph(0, 1),
            glyph(0, 2),
            glyph(7, 3),
            glyph(0, 5),
        ];
        assert_eq!(missing_ranges(&ltr, &(0..6)), vec![1..3, 5..6]);
        let rtl: Vec<RawGlyph> = ltr.iter().rev().copied().collect();
        assert_eq!(missing_ranges(&rtl, &(0..6)), vec![1..3, 5..6]);
    }
}
