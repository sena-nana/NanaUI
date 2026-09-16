//! Bounded LRU of layout results, keyed by [`LayoutKey`].
//!
//! Bounded twice, entries and retained bytes, for the same reason the shape
//! cache is: many short labels and a few long paragraphs exhaust different
//! limits.
//!
//! It also answers one question the counters need and nothing else can: whether
//! a miss is a *new* text or the same shaped text at new constraints. That is
//! the number a resize storm is judged by.

use super::ir::{LineBox, TextLayout};
use super::key::LayoutKey;
use crate::shape::{ShapedGlyph, ShapedRun};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// Limits for a [`Layouter`](super::Layouter)'s cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LayoutCacheBudget {
    pub max_entries: usize,
    pub max_bytes: usize,
}

impl Default for LayoutCacheBudget {
    fn default() -> Self {
        Self {
            max_entries: 4096,
            max_bytes: 8 * 1024 * 1024,
        }
    }
}

struct Entry {
    value: Arc<TextLayout>,
    tick: u64,
    bytes: usize,
    shaped: usize,
}

pub(super) struct LayoutCache {
    entries: HashMap<LayoutKey, Entry>,
    order: BTreeMap<u64, LayoutKey>,
    /// Shaped identity -> how many live entries were laid out from it.
    shaped: HashMap<usize, usize>,
    tick: u64,
    bytes: usize,
    budget: LayoutCacheBudget,
}

/// Bytes a layout occupies, excluding what the key already charges.
fn layout_bytes(value: &TextLayout) -> usize {
    std::mem::size_of::<TextLayout>()
        + value.runs.capacity() * std::mem::size_of::<ShapedRun>()
        + value
            .runs
            .iter()
            .map(|run| run.glyphs.capacity() * std::mem::size_of::<ShapedGlyph>())
            .sum::<usize>()
        + value.lines.capacity() * std::mem::size_of::<LineBox>()
}

impl LayoutCache {
    pub fn new(budget: LayoutCacheBudget) -> Self {
        Self {
            entries: HashMap::new(),
            order: BTreeMap::new(),
            shaped: HashMap::new(),
            tick: 0,
            bytes: 0,
            budget,
        }
    }

    pub fn get(&mut self, key: &LayoutKey) -> Option<Arc<TextLayout>> {
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

    /// True when some other layout of the same shaped runs is still cached, so
    /// a miss on `key` is a constraint change rather than new text.
    pub fn knows_shaped(&self, key: &LayoutKey) -> bool {
        self.shaped.contains_key(&key.shaped_identity())
    }

    /// Stores a layout and returns how many entries were evicted to fit it. A
    /// layout larger than the whole byte budget is returned to the caller but
    /// not stored, and evicts nothing.
    pub fn insert(&mut self, key: LayoutKey, value: Arc<TextLayout>) -> usize {
        let bytes = key.retained_bytes() + layout_bytes(&value);
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
        let shaped = key.shaped_identity();
        *self.shaped.entry(shaped).or_insert(0) += 1;
        self.order.insert(self.tick, key.clone());
        if let Some(previous) = self.entries.insert(
            key,
            Entry {
                value,
                tick: self.tick,
                bytes,
                shaped,
            },
        ) {
            self.order.remove(&previous.tick);
            self.bytes -= previous.bytes;
            self.forget_shaped(previous.shaped);
        }
        evicted
    }

    fn evict_oldest(&mut self) -> bool {
        let Some((_, key)) = self.order.pop_first() else {
            return false;
        };
        if let Some(entry) = self.entries.remove(&key) {
            self.bytes -= entry.bytes;
            self.forget_shaped(entry.shaped);
        }
        true
    }

    fn forget_shaped(&mut self, shaped: usize) {
        if let Some(count) = self.shaped.get_mut(&shaped) {
            *count -= 1;
            if *count == 0 {
                self.shaped.remove(&shaped);
            }
        }
    }

    pub fn set_budget(&mut self, budget: LayoutCacheBudget) -> usize {
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
