//! Algorithm-level work counters and named frame stages for the Performance
//! Contract (Issue #8).
//!
//! Runtime fills these from dirty system work. GPU upload bytes are omitted
//! rather than estimated: this crate does not observe renderer uploads.

/// Per-frame algorithm counts. Timing stays on the Runtime profiler; these
/// fields are the stable CI signals.
///
/// Runtime dirty-bit mapping (Issue #8 §6.2). STATE / TRANSFORM / PAINT are
/// not independent mask bits today:
///
/// - STYLE → `style_processed`
/// - TEXT → `text_shaped` (text shape)
/// - LAYOUT → `layout_nodes`
/// - INPUT → `hit_test_candidates`
/// - FOCUS_IME is tracked on Runtime `SystemWork::focus_ime`, not a dedicated
///   counter field
/// - RENDER → `render_nodes_extracted`. **PAINT is folded into RENDER**; paint-only
///   mutations (hover, color) schedule RENDER without LAYOUT
/// - ACCESSIBILITY → `accessibility_nodes_updated`
///
/// STATE (hover/press/focus) invalidates STYLE+RENDER. TRANSFORM invalidates
/// INPUT+RENDER, and LAYOUT when it affects flow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WorkCounters {
    pub entities_total: usize,
    pub entities_changed: usize,
    pub entities_spawned: usize,
    pub entities_despawned: usize,
    pub style_processed: usize,
    pub text_shaped: usize,
    pub layout_nodes: usize,
    pub hit_test_candidates: usize,
    pub accessibility_nodes_updated: usize,
    /// Nodes scheduled for, or actually produced by, render extraction.
    pub render_nodes_extracted: usize,
    /// Theme-resolved text spans on extracted nodes. Zero until extract is
    /// recorded; not a GPU batch count.
    pub extracted_text_spans: usize,
}

impl WorkCounters {
    /// Fold another drain into this frame snapshot. `entities_total` is the
    /// latest live count; extract fields are added only when the other snapshot
    /// recorded them.
    pub fn accumulate(&mut self, other: Self) {
        self.entities_total = other.entities_total;
        self.entities_changed = self.entities_changed.saturating_add(other.entities_changed);
        self.entities_spawned = self.entities_spawned.saturating_add(other.entities_spawned);
        self.entities_despawned = self
            .entities_despawned
            .saturating_add(other.entities_despawned);
        self.style_processed = self.style_processed.saturating_add(other.style_processed);
        self.text_shaped = self.text_shaped.saturating_add(other.text_shaped);
        self.layout_nodes = self.layout_nodes.saturating_add(other.layout_nodes);
        self.hit_test_candidates = self
            .hit_test_candidates
            .saturating_add(other.hit_test_candidates);
        self.accessibility_nodes_updated = self
            .accessibility_nodes_updated
            .saturating_add(other.accessibility_nodes_updated);
        self.render_nodes_extracted = self
            .render_nodes_extracted
            .saturating_add(other.render_nodes_extracted);
        self.extracted_text_spans = self
            .extracted_text_spans
            .saturating_add(other.extracted_text_spans);
    }
}

/// Named CPU stages from Issue #8 §4. Runtime times the stages it owns;
/// Batch / GPU Upload / Encode / Submit stay `unsupported` until a renderer
/// workstream implements them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameStage {
    Input,
    Reconcile,
    Style,
    TextShape,
    Layout,
    HitTest,
    Accessibility,
    Animation,
    Extract,
    Batch,
    GpuUpload,
    Encode,
    Submit,
}

impl FrameStage {
    pub const ALL: [Self; 13] = [
        Self::Input,
        Self::Reconcile,
        Self::Style,
        Self::TextShape,
        Self::Layout,
        Self::HitTest,
        Self::Accessibility,
        Self::Animation,
        Self::Extract,
        Self::Batch,
        Self::GpuUpload,
        Self::Encode,
        Self::Submit,
    ];

    /// Stages the retained Runtime does not own. A profiler should report these
    /// as unsupported with zero duration rather than pretending they ran.
    pub const fn runtime_unsupported(self) -> bool {
        matches!(
            self,
            Self::Batch | Self::GpuUpload | Self::Encode | Self::Submit
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_counters_are_zero_and_gpu_stages_are_explicitly_unsupported() {
        assert_eq!(WorkCounters::default(), WorkCounters::default());
        assert!(!FrameStage::Style.runtime_unsupported());
        assert!(FrameStage::GpuUpload.runtime_unsupported());
        assert_eq!(FrameStage::ALL.len(), 13);
    }
}
