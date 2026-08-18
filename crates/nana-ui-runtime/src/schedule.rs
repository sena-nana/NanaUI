use bevy_ecs::component::Component;
use nana_ui_core::WorkCounters;

use crate::{ExtractedNode, StableNodeId};

#[derive(Component, Debug, Clone, Copy, Default)]
pub(crate) struct DirtyMask(u8);

impl DirtyMask {
    pub(crate) const STYLE: u8 = 1 << 0;
    pub(crate) const TEXT: u8 = 1 << 1;
    pub(crate) const LAYOUT: u8 = 1 << 2;
    pub(crate) const INPUT: u8 = 1 << 3;
    pub(crate) const FOCUS_IME: u8 = 1 << 4;
    /// Render extraction and paint. PAINT is not an independent bit; paint-only
    /// mutations (hover, color, opacity) set RENDER without LAYOUT.
    pub(crate) const RENDER: u8 = 1 << 5;
    pub(crate) const ACCESSIBILITY: u8 = 1 << 6;
    pub(crate) const ALL: u8 = Self::STYLE
        | Self::TEXT
        | Self::LAYOUT
        | Self::INPUT
        | Self::FOCUS_IME
        | Self::RENDER
        | Self::ACCESSIBILITY;

    pub(crate) const fn all() -> Self {
        Self(Self::ALL)
    }

    pub(crate) fn insert(&mut self, bits: u8) -> bool {
        let before = self.0;
        self.0 |= bits;
        self.0 != before
    }

    pub(crate) fn take(&mut self) -> u8 {
        std::mem::take(&mut self.0)
    }
}

/// Deterministic per-system work produced from entity dirty components.
///
/// PAINT is folded into `RENDER`. Mapping onto Issue #8 dirty bits lives on
/// [`nana_ui_core::WorkCounters`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SystemWork {
    pub generation: u64,
    pub style: Vec<StableNodeId>,
    pub text: Vec<StableNodeId>,
    pub layout: Vec<StableNodeId>,
    pub input_hit_test: Vec<StableNodeId>,
    pub focus_ime: Vec<StableNodeId>,
    pub accessibility: Vec<StableNodeId>,
    pub accessibility_removals: Vec<StableNodeId>,
    pub render_extraction: Vec<StableNodeId>,
    pub render_removals: Vec<StableNodeId>,
    /// Live retained entity count at drain time.
    pub entities_total: usize,
    /// Mounted entities that contributed dirty bits this drain.
    pub entities_changed: usize,
    /// Entities created since the previous drain.
    pub entities_spawned: usize,
    /// Entities despawned since the previous drain.
    pub entities_despawned: usize,
    /// Nodes named for render extraction. Overwritten by
    /// [`Self::record_extract`] with the number actually produced.
    pub render_nodes_extracted: usize,
    /// Theme-resolved text spans after [`Self::record_extract`]. Zero until then.
    pub extracted_text_spans: usize,
}

impl SystemWork {
    pub fn is_empty(&self) -> bool {
        self.style.is_empty()
            && self.text.is_empty()
            && self.layout.is_empty()
            && self.input_hit_test.is_empty()
            && self.focus_ime.is_empty()
            && self.accessibility.is_empty()
            && self.accessibility_removals.is_empty()
            && self.render_extraction.is_empty()
            && self.render_removals.is_empty()
    }

    /// Algorithm-level snapshot for Performance Contract assertions.
    pub fn counters(&self) -> WorkCounters {
        WorkCounters {
            entities_total: self.entities_total,
            entities_changed: self.entities_changed,
            entities_spawned: self.entities_spawned,
            entities_despawned: self.entities_despawned,
            style_processed: self.style.len(),
            text_shaped: self.text.len(),
            layout_nodes: self.layout.len(),
            hit_test_candidates: self.input_hit_test.len(),
            accessibility_nodes_updated: self.accessibility.len(),
            render_nodes_extracted: self.render_nodes_extracted,
            extracted_text_spans: self.extracted_text_spans,
        }
    }

    /// Record extract output. Draw batches and GPU upload bytes are not
    /// available from node extraction and are not fabricated here.
    pub fn record_extract(&mut self, extracted: &[ExtractedNode]) {
        self.render_nodes_extracted = extracted.len();
        self.extracted_text_spans = extracted.iter().map(|node| node.text_spans.len()).sum();
    }
}

pub(crate) fn push_work(work: &mut SystemWork, id: StableNodeId, bits: u8) {
    if bits & DirtyMask::STYLE != 0 {
        work.style.push(id);
    }
    if bits & DirtyMask::TEXT != 0 {
        work.text.push(id);
    }
    if bits & DirtyMask::LAYOUT != 0 {
        work.layout.push(id);
    }
    if bits & DirtyMask::INPUT != 0 {
        work.input_hit_test.push(id);
    }
    if bits & DirtyMask::FOCUS_IME != 0 {
        work.focus_ime.push(id);
    }
    if bits & DirtyMask::ACCESSIBILITY != 0 {
        work.accessibility.push(id);
    }
    if bits & DirtyMask::RENDER != 0 {
        work.render_extraction.push(id);
    }
}
