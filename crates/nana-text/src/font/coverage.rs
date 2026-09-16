//! Which codepoints a face maps, cached per face under a byte budget.
//!
//! Fallback asks "does face F cover this cluster" for many faces and many
//! clusters. Re-walking a cmap for each question is the cost this cache
//! exists to remove: a face's cmap is folded once into sorted inclusive ranges
//! (a CJK face with ~30 000 mappings folds into a few hundred), and every later
//! question is a binary search.

use crate::id::FontId;
use std::collections::HashMap;

/// Sorted, disjoint, non-adjacent inclusive codepoint ranges.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CoverageSet {
    ranges: Vec<(u32, u32)>,
}

impl CoverageSet {
    pub(crate) fn from_sorted_codepoints(codepoints: &[u32]) -> Self {
        let mut ranges: Vec<(u32, u32)> = Vec::new();
        for &codepoint in codepoints {
            match ranges.last_mut() {
                Some((_, end)) if codepoint == *end + 1 => *end = codepoint,
                Some((_, end)) if codepoint <= *end => {}
                _ => ranges.push((codepoint, codepoint)),
            }
        }
        Self { ranges }
    }

    pub fn contains(&self, codepoint: u32) -> bool {
        self.ranges
            .binary_search_by(|&(start, end)| {
                if end < codepoint {
                    std::cmp::Ordering::Less
                } else if start > codepoint {
                    std::cmp::Ordering::Greater
                } else {
                    std::cmp::Ordering::Equal
                }
            })
            .is_ok()
    }

    pub fn range_count(&self) -> usize {
        self.ranges.len()
    }

    pub fn codepoint_count(&self) -> u64 {
        self.ranges
            .iter()
            .map(|&(start, end)| u64::from(end - start) + 1)
            .sum()
    }

    /// Heap bytes the ranges occupy. This, not the codepoint count, is what the
    /// cache budget charges.
    pub fn heap_bytes(&self) -> usize {
        self.ranges.len() * std::mem::size_of::<(u32, u32)>()
    }
}

/// Default budget: enough for every face of a typical desktop fallback chain
/// several times over, small next to a single glyph atlas page.
pub const DEFAULT_COVERAGE_BUDGET_BYTES: usize = 4 * 1024 * 1024;

struct Entry {
    set: std::sync::Arc<CoverageSet>,
    last_used: u64,
}

/// Outcome of one lookup, so the caller can count it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CoverageLookup {
    Hit,
    Miss,
}

/// Per-face coverage keyed by [`FontId`].
///
/// A `FontId` carries its slot generation, so a reissued slot can never read a
/// retired face's coverage. Entries of unregistered faces are dropped eagerly
/// by [`Self::forget`]; nothing is dropped merely because the
/// [`FontGeneration`](crate::FontGeneration) moved, because registering an
/// unrelated face does not change what this face covers.
pub(crate) struct CoverageCache {
    entries: HashMap<FontId, Entry>,
    budget_bytes: usize,
    used_bytes: usize,
    tick: u64,
    evictions: usize,
}

impl CoverageCache {
    pub fn new(budget_bytes: usize) -> Self {
        Self {
            entries: HashMap::new(),
            budget_bytes,
            used_bytes: 0,
            tick: 0,
            evictions: 0,
        }
    }

    pub fn get_or_insert_with(
        &mut self,
        font: FontId,
        build: impl FnOnce() -> CoverageSet,
    ) -> (std::sync::Arc<CoverageSet>, CoverageLookup) {
        self.tick += 1;
        if let Some(entry) = self.entries.get_mut(&font) {
            entry.last_used = self.tick;
            return (std::sync::Arc::clone(&entry.set), CoverageLookup::Hit);
        }
        let set = std::sync::Arc::new(build());
        let bytes = set.heap_bytes();
        // A single set larger than the whole budget is still returned, just
        // not retained, and evicts nothing: flushing every other face for a set
        // that cannot be kept anyway would only thrash the cache.
        if bytes <= self.budget_bytes {
            self.evict_until_fits(bytes);
            self.used_bytes += bytes;
            self.entries.insert(
                font,
                Entry {
                    set: std::sync::Arc::clone(&set),
                    last_used: self.tick,
                },
            );
        }
        (set, CoverageLookup::Miss)
    }

    fn evict_until_fits(&mut self, incoming: usize) {
        while self.used_bytes + incoming > self.budget_bytes {
            // Least recently used; ties cannot occur because every touch takes
            // a fresh tick.
            let Some(victim) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(font, _)| *font)
            else {
                break;
            };
            self.remove(victim);
            self.evictions += 1;
        }
    }

    fn remove(&mut self, font: FontId) {
        if let Some(entry) = self.entries.remove(&font) {
            self.used_bytes -= entry.set.heap_bytes();
        }
    }

    /// Drops a retired face's entry.
    pub fn forget(&mut self, font: FontId) {
        self.remove(font);
    }

    pub fn set_budget(&mut self, budget_bytes: usize) {
        self.budget_bytes = budget_bytes;
        self.evict_until_fits(0);
    }

    pub fn used_bytes(&self) -> usize {
        self.used_bytes
    }

    pub fn budget_bytes(&self) -> usize {
        self.budget_bytes
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn evictions(&self) -> usize {
        self.evictions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codepoints_fold_into_ranges_and_answer_membership() {
        let set = CoverageSet::from_sorted_codepoints(&[0x20, 0x41, 0x42, 0x43, 0x4E00, 0x4E00]);
        assert_eq!(set.range_count(), 3);
        assert_eq!(set.codepoint_count(), 5);
        assert!(set.contains(0x42));
        assert!(!set.contains(0x44));
        assert!(set.contains(0x4E00));
        assert!(!set.contains(0x1F600));
    }

    #[test]
    fn the_cache_evicts_least_recently_used_sets_to_stay_within_budget() {
        let one_range = std::mem::size_of::<(u32, u32)>();
        let mut cache = CoverageCache::new(2 * one_range);
        let set = || CoverageSet::from_sorted_codepoints(&[0x41]);
        let a = FontId::from_parts(0, 1);
        let b = FontId::from_parts(1, 1);
        let c = FontId::from_parts(2, 1);
        assert_eq!(cache.get_or_insert_with(a, set).1, CoverageLookup::Miss);
        assert_eq!(cache.get_or_insert_with(b, set).1, CoverageLookup::Miss);
        assert_eq!(cache.get_or_insert_with(a, set).1, CoverageLookup::Hit);
        // `b` is now the least recently used.
        assert_eq!(cache.get_or_insert_with(c, set).1, CoverageLookup::Miss);
        assert_eq!(cache.evictions(), 1);
        assert!(cache.used_bytes() <= cache.budget_bytes());
        assert_eq!(cache.get_or_insert_with(a, set).1, CoverageLookup::Hit);
        assert_eq!(cache.get_or_insert_with(b, set).1, CoverageLookup::Miss);
    }

    #[test]
    fn a_reissued_slot_does_not_read_the_retired_faces_coverage() {
        let mut cache = CoverageCache::new(DEFAULT_COVERAGE_BUDGET_BYTES);
        let retired = FontId::from_parts(4, 1);
        cache.get_or_insert_with(retired, || CoverageSet::from_sorted_codepoints(&[0x41]));
        let reissued = FontId::from_parts(4, 2);
        let (set, lookup) = cache.get_or_insert_with(reissued, CoverageSet::default);
        assert_eq!(lookup, CoverageLookup::Miss);
        assert!(!set.contains(0x41));
    }
}
