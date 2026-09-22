//! A map with a ceiling that drops its least recently used entries.
//!
//! For tables keyed by something an animation varies every frame — a font
//! axis coordinate, a weight, a size — where every value would otherwise stay
//! until the font set changed. Unlike clearing the whole table at the
//! ceiling, the entries the current frames still ask for survive.

use std::{collections::HashMap, hash::Hash};

#[derive(Debug, Clone)]
pub(crate) struct BoundedCache<K, V> {
    entries: HashMap<K, (V, u64)>,
    cap: usize,
    tick: u64,
}

impl<K: Eq + Hash + Clone, V> BoundedCache<K, V> {
    pub(crate) fn new(cap: usize) -> Self {
        Self {
            entries: HashMap::new(),
            cap: cap.max(4),
            tick: 0,
        }
    }

    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
    }

    /// The entry for `key`, marked as just used.
    pub(crate) fn get(&mut self, key: &K) -> Option<&V> {
        self.tick += 1;
        let tick = self.tick;
        self.entries.get_mut(key).map(|(value, used)| {
            *used = tick;
            &*value
        })
    }

    /// Inserts `value`. Past the ceiling, the least recently used quarter
    /// goes first: one scan per many inserts rather than one per insert.
    pub(crate) fn insert(&mut self, key: K, value: V) {
        if self.entries.len() >= self.cap && !self.entries.contains_key(&key) {
            let mut used: Vec<(u64, K)> = self
                .entries
                .iter()
                .map(|(key, (_, used))| (*used, key.clone()))
                .collect();
            used.sort_unstable_by_key(|(used, _)| *used);
            for (_, key) in used.into_iter().take(self.cap / 4) {
                self.entries.remove(&key);
            }
        }
        self.tick += 1;
        self.entries.insert(key, (value, self.tick));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_coldest_entries_go_and_a_warm_one_stays() {
        let mut cache = BoundedCache::new(8);
        for key in 0..8 {
            cache.insert(key, key * 10);
        }
        assert_eq!(cache.get(&0), Some(&0));
        cache.insert(8, 80);
        assert!(cache.len() <= 8);
        assert_eq!(cache.get(&0), Some(&0), "used last, so kept");
        assert_eq!(cache.get(&1), None, "the coldest went");
        assert_eq!(cache.get(&8), Some(&80));
    }
}
