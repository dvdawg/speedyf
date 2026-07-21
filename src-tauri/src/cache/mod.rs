//! Byte-budgeted LRU cache. Costs are charged in *approximate decoded bitmap
//! bytes* (`width × height × 4`) rather than stored (encoded) bytes, so the
//! budget conservatively bounds both the encoded store and the decoded
//! footprint the webview may hold for mounted pages.
//!
//! Eviction prefers entries flagged stale (wrong generation / closed document
//! / superseded scale) before falling back to least-recently-used order.

use std::collections::HashMap;
use std::hash::Hash;
use std::sync::Arc;

struct Entry {
    bytes: Arc<Vec<u8>>,
    cost: u64,
    last_used: u64,
    stale: bool,
}

pub struct ByteLru<K: Eq + Hash + Clone> {
    map: HashMap<K, Entry>,
    budget: u64,
    used: u64,
    tick: u64,
    pub hits: u64,
    pub lookups: u64,
}

impl<K: Eq + Hash + Clone> ByteLru<K> {
    pub fn new(budget: u64) -> Self {
        ByteLru {
            map: HashMap::new(),
            budget,
            used: 0,
            tick: 0,
            hits: 0,
            lookups: 0,
        }
    }

    pub fn budget(&self) -> u64 {
        self.budget
    }

    pub fn used(&self) -> u64 {
        self.used
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Look up an entry, refreshing its recency on hit.
    pub fn get(&mut self, key: &K) -> Option<Arc<Vec<u8>>> {
        self.lookups += 1;
        self.tick += 1;
        let tick = self.tick;
        match self.map.get_mut(key) {
            Some(e) => {
                e.last_used = tick;
                self.hits += 1;
                Some(Arc::clone(&e.bytes))
            }
            None => None,
        }
    }

    /// Insert an entry with an explicit cost, evicting as needed. Entries
    /// larger than the whole budget are not retained.
    pub fn insert(&mut self, key: K, bytes: Arc<Vec<u8>>, cost: u64) {
        if cost > self.budget {
            return;
        }
        if let Some(old) = self.map.remove(&key) {
            self.used = self.used.saturating_sub(old.cost);
        }
        self.evict_to(self.budget.saturating_sub(cost));
        self.tick += 1;
        self.map.insert(
            key,
            Entry {
                bytes,
                cost,
                last_used: self.tick,
                stale: false,
            },
        );
        self.used += cost;
    }

    /// Change the budget at runtime (e.g. low-memory mode), evicting to fit.
    pub fn set_budget(&mut self, budget: u64) {
        self.budget = budget;
        self.evict_to(budget);
    }

    /// Flag matching entries as stale so they are evicted first.
    pub fn mark_stale(&mut self, pred: impl Fn(&K) -> bool) {
        for (k, e) in self.map.iter_mut() {
            if pred(k) {
                e.stale = true;
            }
        }
    }

    /// Drop matching entries immediately.
    pub fn remove_matching(&mut self, pred: impl Fn(&K) -> bool) {
        let victims: Vec<K> = self.map.keys().filter(|k| pred(k)).cloned().collect();
        for k in victims {
            if let Some(e) = self.map.remove(&k) {
                self.used = self.used.saturating_sub(e.cost);
            }
        }
    }

    fn evict_to(&mut self, target: u64) {
        while self.used > target && !self.map.is_empty() {
            // stale entries first (false sorts before true), then LRU
            let victim = self
                .map
                .iter()
                .min_by_key(|(_, e)| (!e.stale, e.last_used))
                .map(|(k, _)| k.clone());
            match victim {
                Some(k) => {
                    if let Some(e) = self.map.remove(&k) {
                        self.used = self.used.saturating_sub(e.cost);
                    }
                }
                None => break,
            }
        }
    }
}

/// Approximate decoded cost of an RGBA bitmap.
pub fn bitmap_cost(width: u32, height: u32) -> u64 {
    (width as u64) * (height as u64) * 4
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arc(n: usize) -> Arc<Vec<u8>> {
        Arc::new(vec![0u8; n])
    }

    #[test]
    fn bitmap_cost_is_width_height_rgba() {
        assert_eq!(bitmap_cost(100, 50), 100 * 50 * 4);
        assert_eq!(bitmap_cost(0, 50), 0);
    }

    #[test]
    fn tracks_used_bytes_and_evicts_lru_when_over_budget() {
        let mut c: ByteLru<u32> = ByteLru::new(1000);
        c.insert(1, arc(1), 400);
        c.insert(2, arc(1), 400);
        assert_eq!(c.used(), 800);
        // touch 1 so 2 becomes LRU
        assert!(c.get(&1).is_some());
        c.insert(3, arc(1), 400);
        assert_eq!(c.used(), 800);
        assert!(c.get(&2).is_none(), "LRU entry should have been evicted");
        assert!(c.get(&1).is_some());
        assert!(c.get(&3).is_some());
    }

    #[test]
    fn evicts_stale_entries_before_recently_used_ones() {
        let mut c: ByteLru<u32> = ByteLru::new(1000);
        c.insert(1, arc(1), 400);
        c.insert(2, arc(1), 400);
        // 2 is more recently used than 1, but mark 2 stale:
        assert!(c.get(&2).is_some());
        c.mark_stale(|k| *k == 2);
        c.insert(3, arc(1), 400);
        assert!(c.get(&2).is_none(), "stale entry evicted first");
        assert!(c.get(&1).is_some(), "fresh LRU entry survives");
    }

    #[test]
    fn rejects_entries_larger_than_the_whole_budget() {
        let mut c: ByteLru<u32> = ByteLru::new(100);
        c.insert(1, arc(1), 500);
        assert_eq!(c.used(), 0);
        assert_eq!(c.len(), 0);
    }

    #[test]
    fn shrinking_budget_evicts_to_fit() {
        let mut c: ByteLru<u32> = ByteLru::new(1200);
        c.insert(1, arc(1), 400);
        c.insert(2, arc(1), 400);
        c.insert(3, arc(1), 400);
        c.set_budget(500);
        assert!(c.used() <= 500);
        assert!(c.get(&3).is_some(), "most recent survives");
    }

    #[test]
    fn replacing_a_key_updates_cost_accounting() {
        let mut c: ByteLru<u32> = ByteLru::new(1000);
        c.insert(1, arc(1), 400);
        c.insert(1, arc(1), 100);
        assert_eq!(c.used(), 100);
    }

    #[test]
    fn remove_matching_drops_entries_and_cost() {
        let mut c: ByteLru<u32> = ByteLru::new(1000);
        c.insert(1, arc(1), 300);
        c.insert(2, arc(1), 300);
        c.remove_matching(|k| *k == 1);
        assert_eq!(c.used(), 300);
        assert!(c.get(&1).is_none());
    }

    #[test]
    fn counts_hits_and_lookups() {
        let mut c: ByteLru<u32> = ByteLru::new(1000);
        c.insert(1, arc(1), 100);
        let _ = c.get(&1);
        let _ = c.get(&9);
        assert_eq!(c.hits, 1);
        assert_eq!(c.lookups, 2);
    }
}
