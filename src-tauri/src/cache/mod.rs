//! Byte-budgeted LRU cache. Costs are the encoded bytes actually retained by
//! Rust. The frontend's page virtualization separately bounds how many images
//! can be decoded/mounted by the webview.
//!
//! Eviction prefers entries flagged stale (wrong generation / closed document
//! / superseded scale) before falling back to least-recently-used order.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::hash::Hash;
use std::sync::Arc;

struct Entry {
    bytes: Arc<Vec<u8>>,
    cost: u64,
    last_used: u64,
    stale: bool,
}

pub struct ByteLru<K: Eq + Hash + Clone + Ord> {
    map: HashMap<K, Entry>,
    order: BinaryHeap<Reverse<(u8, u64, K)>>,
    budget: u64,
    used: u64,
    tick: u64,
    pub hits: u64,
    pub lookups: u64,
}

impl<K: Eq + Hash + Clone + Ord> ByteLru<K> {
    pub fn new(budget: u64) -> Self {
        ByteLru {
            map: HashMap::new(),
            order: BinaryHeap::new(),
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
    ///
    /// A hit also clears the stale flag: staleness marks entries *believed*
    /// unwanted, and a live request is proof otherwise. Without this, every
    /// page still on screen when a generation bump swept the document would
    /// stay a preferred eviction victim for the rest of its life.
    pub fn get(&mut self, key: &K) -> Option<Arc<Vec<u8>>> {
        self.lookups += 1;
        self.tick += 1;
        let tick = self.tick;
        match self.map.get_mut(key) {
            Some(e) => {
                e.last_used = tick;
                e.stale = false;
                self.hits += 1;
                self.order.push(Reverse((1, tick, key.clone())));
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
            key.clone(),
            Entry {
                bytes,
                cost,
                last_used: self.tick,
                stale: false,
            },
        );
        self.order.push(Reverse((1, self.tick, key)));
        self.used += cost;
        self.compact_order_if_needed();
    }

    /// Change the budget at runtime (e.g. low-memory mode), evicting to fit.
    pub fn set_budget(&mut self, budget: u64) {
        self.budget = budget;
        self.evict_to(budget);
    }

    /// Flag matching entries as stale so they are evicted first. Entries that
    /// are already stale are skipped: re-marking them would push duplicate
    /// heap records on every call, and repeated marking with no intervening
    /// insert (which is what compacts `order`) would grow it without bound.
    pub fn mark_stale(&mut self, pred: impl Fn(&K) -> bool) {
        let mut newly_stale = Vec::new();
        for (k, e) in self.map.iter_mut() {
            if !e.stale && pred(k) {
                e.stale = true;
                newly_stale.push((k.clone(), e.last_used));
            }
        }
        self.order.extend(
            newly_stale
                .into_iter()
                .map(|(key, tick)| Reverse((0, tick, key))),
        );
        self.compact_order_if_needed();
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
            let Some(Reverse((freshness, tick, key))) = self.order.pop() else {
                break;
            };
            let current = self
                .map
                .get(&key)
                .map(|entry| (u8::from(!entry.stale), entry.last_used));
            if current != Some((freshness, tick)) {
                continue;
            }
            if let Some(entry) = self.map.remove(&key) {
                self.used = self.used.saturating_sub(entry.cost);
            }
        }
        self.compact_order_if_needed();
    }

    fn compact_order_if_needed(&mut self) {
        if self.order.len() <= self.map.len().saturating_mul(8).saturating_add(64) {
            return;
        }
        self.order = self
            .map
            .iter()
            .map(|(key, entry)| Reverse((u8::from(!entry.stale), entry.last_used, key.clone())))
            .collect();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arc(n: usize) -> Arc<Vec<u8>> {
        Arc::new(vec![0u8; n])
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
    fn a_hit_clears_staleness_so_visible_entries_stop_being_victims() {
        let mut c: ByteLru<u32> = ByteLru::new(1000);
        c.insert(1, arc(1), 400);
        c.insert(2, arc(1), 400);
        c.mark_stale(|k| *k == 1);
        // 1 is stale but demonstrably still wanted; 2 has not been touched.
        assert!(c.get(&1).is_some());
        c.insert(3, arc(1), 400);
        assert!(c.get(&1).is_some(), "re-requested entry survives");
        assert!(c.get(&2).is_none(), "untouched LRU entry evicted instead");
    }

    #[test]
    fn repeated_marking_does_not_grow_the_order_heap() {
        // mark_stale used to push a record per matching entry per call, with
        // no compaction of its own — repeated marking with no intervening
        // insert grew the heap without bound.
        let mut c: ByteLru<u32> = ByteLru::new(10_000);
        for k in 0..20 {
            c.insert(k, arc(1), 100);
        }
        let baseline = c.order.len();
        for _ in 0..500 {
            c.mark_stale(|_| true);
        }
        assert!(
            c.order.len() <= baseline + c.map.len(),
            "order heap grew to {} from {baseline}",
            c.order.len()
        );
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
