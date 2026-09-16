//! Bounded LRU of shape results.
//!
//! Bounded twice — entry count and retained bytes — because a few long
//! paragraphs and many short labels exhaust different limits. Entries from an
//! older font generation are purged the first time a newer generation is seen,
//! rather than lingering until LRU pressure reaches them.

use super::ShapedText;
use super::key::{FontEpoch, ShapeKey};
use crate::shape::{ShapedGlyph, ShapedRun};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// Limits for a [`Shaper`](super::Shaper)'s cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShapeCacheBudget {
    pub max_entries: usize,
    pub max_bytes: usize,
}

impl Default for ShapeCacheBudget {
    fn default() -> Self {
        Self {
            max_entries: 4096,
            max_bytes: 16 * 1024 * 1024,
        }
    }
}

struct Entry {
    value: Arc<ShapedText>,
    tick: u64,
    bytes: usize,
}

pub(crate) struct ShapeCache {
    entries: HashMap<ShapeKey, Entry>,
    /// Access order: tick -> key. Ticks are unique, so this is exact LRU.
    order: BTreeMap<u64, ShapeKey>,
    tick: u64,
    bytes: usize,
    budget: ShapeCacheBudget,
    epoch: Option<FontEpoch>,
}

/// Bytes a shape result occupies, excluding what the key already charges.
pub(crate) fn shaped_bytes(value: &ShapedText) -> usize {
    std::mem::size_of::<ShapedText>()
        + value.runs.capacity() * std::mem::size_of::<ShapedRun>()
        + value
            .runs
            .iter()
            .map(|run| run.glyphs.capacity() * std::mem::size_of::<ShapedGlyph>())
            .sum::<usize>()
        + value.paragraphs.capacity() * std::mem::size_of::<super::ShapedParagraph>()
}

impl ShapeCache {
    pub fn new(budget: ShapeCacheBudget) -> Self {
        Self {
            entries: HashMap::new(),
            order: BTreeMap::new(),
            tick: 0,
            bytes: 0,
            budget,
            epoch: None,
        }
    }

    pub fn get(&mut self, key: &ShapeKey) -> Option<Arc<ShapedText>> {
        let entry = self.entries.get_mut(key)?;
        self.tick += 1;
        let key = self
            .order
            .remove(&entry.tick)
            .expect("every entry has an order slot");
        entry.tick = self.tick;
        self.order.insert(self.tick, key);
        Some(Arc::clone(&entry.value))
    }

    /// Stores a result and returns how many entries were evicted to fit it.
    /// A result larger than the whole byte budget is not stored and evicts
    /// nothing.
    pub fn insert(&mut self, key: ShapeKey, value: Arc<ShapedText>) -> usize {
        let bytes = key.retained_bytes() + shaped_bytes(&value);
        if bytes > self.budget.max_bytes || self.budget.max_entries == 0 {
            return 0;
        }
        let mut evicted = 0;
        while self.entries.len() + 1 > self.budget.max_entries
            || self.bytes + bytes > self.budget.max_bytes
        {
            if !self.evict_oldest() {
                break;
            }
            evicted += 1;
        }
        self.tick += 1;
        self.bytes += bytes;
        self.order.insert(self.tick, key.clone());
        if let Some(previous) = self.entries.insert(
            key,
            Entry {
                value,
                tick: self.tick,
                bytes,
            },
        ) {
            self.order.remove(&previous.tick);
            self.bytes -= previous.bytes;
        }
        evicted
    }

    fn evict_oldest(&mut self) -> bool {
        let Some((_, key)) = self.order.pop_first() else {
            return false;
        };
        if let Some(entry) = self.entries.remove(&key) {
            self.bytes -= entry.bytes;
        }
        true
    }

    /// Drops every entry from another font system or generation; returns how
    /// many.
    pub fn purge_other_epochs(&mut self, current: FontEpoch) -> usize {
        if self.epoch == Some(current) {
            return 0;
        }
        self.epoch = Some(current);
        let stale: Vec<u64> = self
            .order
            .iter()
            .filter(|(_, key)| key.epoch() != current)
            .map(|(tick, _)| *tick)
            .collect();
        for tick in &stale {
            if let Some(key) = self.order.remove(tick)
                && let Some(entry) = self.entries.remove(&key)
            {
                self.bytes -= entry.bytes;
            }
        }
        stale.len()
    }

    pub fn set_budget(&mut self, budget: ShapeCacheBudget) -> usize {
        self.budget = budget;
        let mut evicted = 0;
        while (self.entries.len() > budget.max_entries || self.bytes > budget.max_bytes)
            && self.evict_oldest()
        {
            evicted += 1;
        }
        evicted
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn bytes(&self) -> usize {
        self.bytes
    }
}
