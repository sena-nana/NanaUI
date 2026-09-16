//! Work counts for a text pass. Not `nana_ui_core::WorkCounters`, and not a
//! timing profile.
//!
//! The field convention is taken from `nana_ui_core::work::WorkCounters`:
//! a `usize` field is one the owning pass always measures, and an
//! `Option<usize>` field is one nothing has observed yet — it must stay `None`
//! rather than become a fake `0`, because a fake zero reads as "this work did
//! not happen" when it means "nobody looked".
//!
//! These are the interfaces #89 reserves. They have no product producer yet;
//! folding them into `WorkCounters` belongs at the UiWorld seam, when a real
//! pass exists to fill them.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TextWorkCounters {
    /// Text nodes the pass looked at, including ones it skipped as unchanged.
    #[serde(default)]
    pub text_nodes_considered: usize,
    /// Of those, the ones that actually reached the shaper.
    #[serde(default)]
    pub text_nodes_shaped: usize,
    /// `None` until a shape cache is consulted.
    #[serde(default)]
    pub shape_cache_hits: Option<usize>,
    #[serde(default)]
    pub shape_cache_misses: Option<usize>,
    /// `None` until a layout cache is consulted.
    #[serde(default)]
    pub layout_cache_hits: Option<usize>,
    #[serde(default)]
    pub layout_cache_misses: Option<usize>,
    /// Glyphs an engine resolved to a glyph id this pass. `None` until an
    /// engine that resolves glyphs records it.
    #[serde(default)]
    pub glyphs_resolved: Option<usize>,
}

impl TextWorkCounters {
    /// A pass that ran knows both numbers, so neither is optional.
    pub fn record_text_pass(&mut self, considered: usize, shaped: usize) {
        self.text_nodes_considered += considered;
        self.text_nodes_shaped += shaped;
    }

    /// Recording `(0, 0)` still moves the fields to `Some(0)`: the cache was
    /// consulted and answered nothing, which is different from never consulted.
    pub fn record_shape_cache(&mut self, hits: usize, misses: usize) {
        add_optional(&mut self.shape_cache_hits, hits);
        add_optional(&mut self.shape_cache_misses, misses);
    }

    pub fn record_layout_cache(&mut self, hits: usize, misses: usize) {
        add_optional(&mut self.layout_cache_hits, hits);
        add_optional(&mut self.layout_cache_misses, misses);
    }

    pub fn record_glyphs_resolved(&mut self, glyphs: usize) {
        add_optional(&mut self.glyphs_resolved, glyphs);
    }

    /// Folds another pass in. `None + None` stays `None`; anything observed on
    /// either side makes the result observed.
    pub fn accumulate(&mut self, other: Self) {
        self.text_nodes_considered += other.text_nodes_considered;
        self.text_nodes_shaped += other.text_nodes_shaped;
        fold_optional(&mut self.shape_cache_hits, other.shape_cache_hits);
        fold_optional(&mut self.shape_cache_misses, other.shape_cache_misses);
        fold_optional(&mut self.layout_cache_hits, other.layout_cache_hits);
        fold_optional(&mut self.layout_cache_misses, other.layout_cache_misses);
        fold_optional(&mut self.glyphs_resolved, other.glyphs_resolved);
    }

    /// True when no pass has touched any field.
    pub fn is_unobserved(&self) -> bool {
        *self == Self::default()
    }
}

fn add_optional(slot: &mut Option<usize>, count: usize) {
    *slot = Some(slot.unwrap_or(0) + count);
}

fn fold_optional(slot: &mut Option<usize>, other: Option<usize>) {
    match (*slot, other) {
        (None, None) => {}
        (a, b) => *slot = Some(a.unwrap_or(0) + b.unwrap_or(0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_text_counters_are_zero_and_cache_fields_are_explicitly_unobserved() {
        let counters = TextWorkCounters::default();
        assert_eq!(counters.text_nodes_considered, 0);
        assert_eq!(counters.text_nodes_shaped, 0);
        assert_eq!(counters.shape_cache_hits, None);
        assert_eq!(counters.shape_cache_misses, None);
        assert_eq!(counters.layout_cache_hits, None);
        assert_eq!(counters.layout_cache_misses, None);
        assert_eq!(counters.glyphs_resolved, None);
        assert!(counters.is_unobserved());
    }

    #[test]
    fn cache_fields_stay_none_until_a_pass_consults_them() {
        let mut counters = TextWorkCounters::default();
        counters.record_text_pass(4, 1);
        assert_eq!(counters.text_nodes_considered, 4);
        assert_eq!(counters.text_nodes_shaped, 1);
        assert_eq!(
            counters.shape_cache_hits, None,
            "shaping a node must not invent a cache observation"
        );

        counters.record_shape_cache(0, 0);
        assert_eq!(
            counters.shape_cache_hits,
            Some(0),
            "a consulted cache reports Some(0), not None"
        );
        assert_eq!(counters.shape_cache_misses, Some(0));
        assert_eq!(counters.layout_cache_hits, None);
    }

    #[test]
    fn accumulate_folds_counts_and_none_plus_none_stays_none() {
        let mut left = TextWorkCounters::default();
        left.record_text_pass(2, 2);
        left.record_glyphs_resolved(7);

        let mut right = TextWorkCounters::default();
        right.record_text_pass(3, 0);

        left.accumulate(right);
        assert_eq!(left.text_nodes_considered, 5);
        assert_eq!(left.text_nodes_shaped, 2);
        assert_eq!(left.glyphs_resolved, Some(7));
        assert_eq!(
            left.shape_cache_hits, None,
            "neither side consulted a shape cache, so it stays unobserved"
        );
    }
}
