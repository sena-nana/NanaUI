//! A host [`TextShaper`] whose only measurement authority is a `nana-text`
//! engine (Issue #95).
//!
//! Plain text nodes resolve through [`TextShaper::text_engine`] and retain the
//! layout they were measured from. Everything else a pass measures — editor
//! text, EmptyState and modal intrinsic text, component probes — goes through
//! [`TextShaper::shape`], which lays out through the same engine, so no node's
//! metrics come from a second engine.
//!
//! Editors (Issue #96) are answered from a retained
//! [`EditorGeometry`](nana_text::EditorGeometry) per node: caret positions,
//! selection highlights and pointer hits read its per-paragraph layouts
//! without shaping or laying anything out, and an edit lays out only the
//! paragraphs whose bytes changed.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;

use nana_text::{
    Affinity, EditorGeometry, SharedTextEngine, TextConstraints as NanaTextConstraints,
    TextEngine as _, TextSource, TextStyle as NanaTextStyle, TextWorkCounters,
};

use crate::text_node::{nana_text_constraints, nana_text_style, text_kind, text_metrics_of_layout};
use crate::{
    ComputedStyle, LayoutBox, StableNodeId, TextContent, TextHit, TextHorizontalAlignment,
    TextMetrics, TextShapeConstraints, TextShaper,
};

/// Editors whose geometry is kept. Each entry holds layouts of one editor's
/// text, so this bounds memory by editors recently probed, not by text.
const EDITOR_GEOMETRY_CAPACITY: usize = 32;

#[derive(Clone)]
struct EditorEntry {
    id: StableNodeId,
    style: NanaTextStyle,
    constraints: NanaTextConstraints,
    geometry: EditorGeometry,
}

#[derive(Clone)]
pub struct NanaTextEngineShaper {
    engine: SharedTextEngine,
    /// Engine work done through [`TextShaper::shape`], handed to the pass
    /// through [`TextShaper::take_text_work`].
    work: TextWorkCounters,
    /// Retained editor geometry, least recently used first.
    editors: Vec<EditorEntry>,
}

impl NanaTextEngineShaper {
    pub fn new(engine: SharedTextEngine) -> Self {
        Self {
            engine,
            work: TextWorkCounters::default(),
            editors: Vec::new(),
        }
    }

    pub fn engine(&self) -> &SharedTextEngine {
        &self.engine
    }

    /// The node's editor geometry for these inputs, synced to `text` unless
    /// `synced` says this batch already did. Creates it when `create`.
    ///
    /// One entry per node: a node probed under new style or constraints — a
    /// resize — lays its paragraphs out again in the entry it has, rather
    /// than growing a second entry that would push other editors out.
    fn editor_geometry(
        &mut self,
        id: StableNodeId,
        text: &str,
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
        synced: bool,
        create: bool,
    ) -> Option<&EditorGeometry> {
        let nana_style = nana_text_style(style);
        let nana_constraints =
            nana_text_constraints(style, &constraints, TextHorizontalAlignment::Start);
        let mut entry = match self.editors.iter().rposition(|entry| entry.id == id) {
            Some(index) => {
                let entry = self.editors.remove(index);
                let same = entry.constraints == nana_constraints && entry.style == nana_style;
                if !same && !create {
                    // Measuring under constraints the editor is not probed
                    // with: leave its geometry alone.
                    self.editors.push(entry);
                    return None;
                }
                entry
            }
            None if create => {
                if self.editors.len() >= EDITOR_GEOMETRY_CAPACITY {
                    self.editors.remove(0);
                }
                EditorEntry {
                    id,
                    style: nana_style.clone(),
                    constraints: nana_constraints,
                    geometry: EditorGeometry::new(),
                }
            }
            None => return None,
        };
        let changed = entry.constraints != nana_constraints || entry.style != nana_style;
        if !synced || changed || entry.geometry.text_len() != text.len() {
            entry.style = nana_style;
            entry.constraints = nana_constraints;
            let mut engine = nana_text::lock_text_engine(&self.engine);
            let sync = entry.geometry.sync(
                &mut engine,
                text,
                None,
                &entry.style,
                &entry.constraints,
                &mut self.work,
            );
            self.work.text_source_clones += sync.paragraphs_laid_out;
        }
        self.editors.push(entry);
        self.editors.last().map(|entry| &entry.geometry)
    }

    fn position(
        &mut self,
        id: StableNodeId,
        text: &TextContent,
        offset: usize,
        affinity: Affinity,
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
        synced: bool,
    ) -> (f32, f32, f32) {
        if !is_caret_boundary(&text.value, offset) {
            return (0.0, 0.0, 0.0);
        }
        self.editor_geometry(id, &text.value, style, constraints, synced, true)
            .and_then(|geometry| geometry.caret_rect(offset, affinity))
            .map_or((0.0, 0.0, 0.0), |caret| {
                (caret.x_px, caret.y_px, caret.height_px)
            })
    }

    fn highlights(
        &mut self,
        id: StableNodeId,
        text: &TextContent,
        (start, end): (usize, usize),
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
        synced: bool,
    ) -> Vec<LayoutBox> {
        if start >= end
            || !is_caret_boundary(&text.value, start)
            || !is_caret_boundary(&text.value, end)
        {
            return Vec::new();
        }
        self.editor_geometry(id, &text.value, style, constraints, synced, true)
            .map(|geometry| {
                geometry
                    .selection_rects(start..end)
                    .into_iter()
                    .map(|rect| LayoutBox {
                        x: rect.x,
                        y: rect.y,
                        width: rect.width,
                        height: rect.height,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The caret a point hits, with the affinity the geometry resolved: the
    /// end of a line that wrapped without hanging whitespace is the next
    /// line's start, and only `Upstream` keeps the caret on the line that was
    /// clicked.
    fn hit_at_point(
        &mut self,
        id: StableNodeId,
        text: &TextContent,
        x: f32,
        y: f32,
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
        synced: bool,
    ) -> Option<TextHit> {
        if text.value.is_empty() {
            return Some(TextHit::default());
        }
        let geometry = self.editor_geometry(id, &text.value, style, constraints, synced, true)?;
        let hit = geometry.hit_test(x, y);
        Some(TextHit {
            offset: hit.offset,
            affinity: hit.affinity,
        })
    }

    fn measure(
        &mut self,
        id: StableNodeId,
        text: &TextContent,
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
    ) -> TextMetrics {
        // An editor measures from the geometry its probes read, so an edit
        // lays out its own paragraph rather than the whole text.
        if let Some(geometry) =
            self.editor_geometry(id, &text.value, style, constraints, false, false)
        {
            return metrics_of_geometry(geometry);
        }
        let source = TextSource::new(text.value.as_str());
        let layout = nana_text::lock_text_engine(&self.engine).layout(
            text_kind(&constraints),
            &source,
            &nana_text_style(style),
            &nana_text_constraints(style, &constraints, TextHorizontalAlignment::Start),
            &mut self.work,
        );
        self.work.text_source_clones += 1;
        text_metrics_of_layout(&layout)
    }
}

/// A byte offset a caret can stand at.
fn is_caret_boundary(text: &str, offset: usize) -> bool {
    nana_text::editable::navigation::is_grapheme_boundary(text, offset)
}

fn metrics_of_geometry(geometry: &EditorGeometry) -> TextMetrics {
    let mut metrics = TextMetrics::default();
    for (index, (_, _, layout)) in geometry.paragraph_layouts().enumerate() {
        let paragraph = text_metrics_of_layout(layout);
        metrics.width = metrics.width.max(paragraph.width);
        metrics.height += paragraph.height;
        if index == 0 {
            metrics.ascent = paragraph.ascent;
        }
    }
    metrics
}

impl std::fmt::Debug for NanaTextEngineShaper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NanaTextEngineShaper")
            .field("editors", &self.editors.len())
            .finish_non_exhaustive()
    }
}

impl TextShaper for NanaTextEngineShaper {
    fn shape(
        &mut self,
        id: StableNodeId,
        text: &TextContent,
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
    ) -> TextMetrics {
        self.measure(id, text, style, constraints)
    }

    /// Probes of one text snapshot sync each editor's geometry once, not once
    /// per probe.
    fn with_text_probes<R>(
        &mut self,
        text: &TextContent,
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
        consume: impl FnOnce(&mut dyn TextShaper) -> R,
    ) -> R {
        let mut prepared = PreparedEngineShaper {
            host: self,
            text,
            style,
            constraints,
            synced: Vec::new(),
        };
        consume(&mut prepared)
    }

    /// One unwrapped line, measured on its own: callers ask this of strings
    /// that are not the node's text (completion labels), so it must not touch
    /// the node's editor geometry.
    fn horizontal_offset(
        &mut self,
        _id: StableNodeId,
        text: &TextContent,
        byte_offset: usize,
        style: &ComputedStyle,
    ) -> f32 {
        if !is_caret_boundary(&text.value, byte_offset) {
            return 0.0;
        }
        let constraints = TextShapeConstraints::default();
        let source = TextSource::new(text.value.as_str());
        let layout = nana_text::lock_text_engine(&self.engine).layout(
            text_kind(&constraints),
            &source,
            &nana_text_style(style),
            &nana_text_constraints(style, &constraints, TextHorizontalAlignment::Start),
            &mut self.work,
        );
        self.work.text_source_clones += 1;
        layout
            .caret_geometry(nana_text::CaretPosition::new(
                byte_offset,
                Affinity::Downstream,
                0,
            ))
            .map_or(0.0, |caret| caret.x_px)
    }

    fn text_position(
        &mut self,
        id: StableNodeId,
        text: &TextContent,
        byte_offset: usize,
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
    ) -> (f32, f32, f32) {
        self.position(
            id,
            text,
            byte_offset,
            Affinity::Downstream,
            style,
            constraints,
            false,
        )
    }

    fn text_caret_position(
        &mut self,
        id: StableNodeId,
        text: &TextContent,
        byte_offset: usize,
        affinity: Affinity,
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
    ) -> (f32, f32, f32) {
        self.position(id, text, byte_offset, affinity, style, constraints, false)
    }

    fn text_highlights(
        &mut self,
        id: StableNodeId,
        text: &TextContent,
        selection: (usize, usize),
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
    ) -> Vec<LayoutBox> {
        self.highlights(id, text, selection, style, constraints, false)
    }

    fn text_hit_at_point(
        &mut self,
        id: StableNodeId,
        text: &TextContent,
        x: f32,
        y: f32,
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
    ) -> Option<TextHit> {
        self.hit_at_point(id, text, x, y, style, constraints, false)
    }

    /// The whole engine epoch, folded: a language change or another engine is
    /// as much a new measurement as a font registration.
    fn font_generation(&self) -> u64 {
        let epoch = nana_text::lock_text_engine(&self.engine).epoch();
        let mut hasher = DefaultHasher::new();
        epoch.hash(&mut hasher);
        hasher.finish()
    }

    fn take_text_work(&mut self) -> nana_text::TextWorkCounters {
        let mut work = std::mem::take(&mut self.work);
        for entry in &self.editors {
            entry.geometry.record_queries(&mut work);
        }
        // The pass counts the nodes it measures; these are the engine's own
        // cache and layout numbers behind them.
        work.text_nodes_considered = 0;
        work.text_nodes_shaped = 0;
        work
    }

    fn text_engine(&self) -> Option<SharedTextEngine> {
        Some(Arc::clone(&self.engine))
    }
}

/// One snapshot's probes: the first probe of each editor syncs its geometry,
/// the rest read it.
struct PreparedEngineShaper<'a> {
    host: &'a mut NanaTextEngineShaper,
    text: &'a TextContent,
    style: &'a ComputedStyle,
    constraints: TextShapeConstraints,
    /// Editors whose geometry this batch synced to `text`.
    synced: Vec<StableNodeId>,
}

impl PreparedEngineShaper<'_> {
    /// Whether the geometry for this probe is already synced to its text: a
    /// probe of the batch's snapshot syncs the editor's geometry the first
    /// time and reads it after.
    fn synced(
        &mut self,
        id: StableNodeId,
        text: &TextContent,
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
    ) -> bool {
        let batched =
            std::ptr::eq(text, self.text) && constraints == self.constraints && style == self.style;
        if !batched {
            return false;
        }
        if !self.synced.contains(&id) {
            self.host
                .editor_geometry(id, &text.value, style, constraints, false, true);
            self.synced.push(id);
        }
        true
    }
}

impl TextShaper for PreparedEngineShaper<'_> {
    fn shape(
        &mut self,
        id: StableNodeId,
        text: &TextContent,
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
    ) -> TextMetrics {
        self.host.measure(id, text, style, constraints)
    }

    fn font_generation(&self) -> u64 {
        self.host.font_generation()
    }

    fn text_engine(&self) -> Option<SharedTextEngine> {
        self.host.text_engine()
    }

    fn take_text_work(&mut self) -> nana_text::TextWorkCounters {
        self.host.take_text_work()
    }

    fn horizontal_offset(
        &mut self,
        id: StableNodeId,
        text: &TextContent,
        byte_offset: usize,
        style: &ComputedStyle,
    ) -> f32 {
        self.host.horizontal_offset(id, text, byte_offset, style)
    }

    fn text_position(
        &mut self,
        id: StableNodeId,
        text: &TextContent,
        byte_offset: usize,
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
    ) -> (f32, f32, f32) {
        let synced = self.synced(id, text, style, constraints);
        self.host.position(
            id,
            text,
            byte_offset,
            Affinity::Downstream,
            style,
            constraints,
            synced,
        )
    }

    fn text_caret_position(
        &mut self,
        id: StableNodeId,
        text: &TextContent,
        byte_offset: usize,
        affinity: Affinity,
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
    ) -> (f32, f32, f32) {
        let synced = self.synced(id, text, style, constraints);
        self.host
            .position(id, text, byte_offset, affinity, style, constraints, synced)
    }

    fn text_highlights(
        &mut self,
        id: StableNodeId,
        text: &TextContent,
        selection: (usize, usize),
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
    ) -> Vec<LayoutBox> {
        let synced = self.synced(id, text, style, constraints);
        self.host
            .highlights(id, text, selection, style, constraints, synced)
    }

    fn text_hit_at_point(
        &mut self,
        id: StableNodeId,
        text: &TextContent,
        x: f32,
        y: f32,
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
    ) -> Option<TextHit> {
        let synced = self.synced(id, text, style, constraints);
        self.host
            .hit_at_point(id, text, x, y, style, constraints, synced)
    }
}
