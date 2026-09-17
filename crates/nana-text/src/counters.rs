//! Work counts for a text pass. Not `nana_ui_core::WorkCounters`, and not a
//! timing profile.
//!
//! The field convention is taken from `nana_ui_core::work::WorkCounters`:
//! a `usize` field is one the owning pass always measures, and an
//! `Option<usize>` field is one nothing has observed yet — it must stay `None`
//! rather than become a fake `0`, because a fake zero reads as "this work did
//! not happen" when it means "nobody looked".
//!
//! #89 reserved the first seven fields. #95 gave them a product producer — the
//! UiWorld text pass — and added the ones that explain what a pass that shaped
//! nothing still spent: how many candidates were skipped on revision alone,
//! and whether anything was cloned, hashed or looked up anyway.

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
    /// Considered nodes whose content, style, constraint and font revisions
    /// all matched the layout they already hold, decided before any text was
    /// read. Their per-node cost is independent of the text's length.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub text_nodes_revision_skipped: usize,
    /// Text strings copied to build a source. Zero on a pass that only skips.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub text_source_clones: usize,
    /// Text bytes fed to a content hash. A source hashes once per revision.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub text_bytes_hashed: usize,
    /// Shape cache lookups (hits plus misses).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub shape_cache_lookups: usize,
    /// Layout cache lookups (hits plus misses).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub layout_cache_lookups: usize,
    /// Layouts built rather than answered by the layout cache.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub layouts_created: usize,
    /// Of [`Self::layouts_created`], the ones laid out from shaped runs that
    /// were already cached: a constraint change, not new text.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub constraint_only_relayouts: usize,
    /// Nodes that ended the pass holding the same layout they started with.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub text_layouts_reused: usize,
    /// #96: edits that changed committed editable text.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub editable_mutations: usize,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub editable_bytes_inserted: usize,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub editable_bytes_deleted: usize,
    /// Selection changes that left a collapsed caret, with no text change.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub caret_only_updates: usize,
    /// Selection changes that left a non-empty selection, with no text change.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub selection_only_updates: usize,
    /// Preedit starts, changes and ends.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub composition_updates: usize,
    /// Editor paragraphs laid out again after an edit or composition change
    /// whose shaping missed the cache.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub paragraphs_reshaped_from_edit: usize,
    /// Editor paragraphs laid out again after an edit or composition change.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub paragraphs_relayout_from_edit: usize,
    /// Points resolved to a text position against retained editor geometry.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub hit_test_queries: usize,
    /// Carets placed against retained editor geometry.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub caret_geometry_queries: usize,
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
        self.text_nodes_revision_skipped += other.text_nodes_revision_skipped;
        self.text_source_clones += other.text_source_clones;
        self.text_bytes_hashed += other.text_bytes_hashed;
        self.shape_cache_lookups += other.shape_cache_lookups;
        self.layout_cache_lookups += other.layout_cache_lookups;
        self.layouts_created += other.layouts_created;
        self.constraint_only_relayouts += other.constraint_only_relayouts;
        self.text_layouts_reused += other.text_layouts_reused;
        self.editable_mutations += other.editable_mutations;
        self.editable_bytes_inserted += other.editable_bytes_inserted;
        self.editable_bytes_deleted += other.editable_bytes_deleted;
        self.caret_only_updates += other.caret_only_updates;
        self.selection_only_updates += other.selection_only_updates;
        self.composition_updates += other.composition_updates;
        self.paragraphs_reshaped_from_edit += other.paragraphs_reshaped_from_edit;
        self.paragraphs_relayout_from_edit += other.paragraphs_relayout_from_edit;
        self.hit_test_queries += other.hit_test_queries;
        self.caret_geometry_queries += other.caret_geometry_queries;
    }

    /// True when no pass has touched any field.
    pub fn is_unobserved(&self) -> bool {
        *self == Self::default()
    }
}

/// The #95 fields are left out of a serialized record while zero, so a
/// recorded golden from a pass that never produced them keeps its shape.
fn is_zero(value: &usize) -> bool {
    *value == 0
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
