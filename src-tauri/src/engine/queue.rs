//! Blocking priority queue + per-document generation registry.
//!
//! Jobs are ordered by (priority, submission sequence). Every job carries the
//! generation current at submission time; the worker skips (and counts)
//! obsolete render jobs whose generation has since been bumped. Non-render
//! work such as search and save is never cancelled by a zoom change.

use super::types::{DocId, Priority};
use parking_lot::{Condvar, Mutex};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

#[derive(Clone, Copy, Debug)]
pub struct JobMeta {
    pub doc: DocId,
    pub gen: u64,
}

struct Item<T> {
    key: Reverse<(u8, u64)>,
    meta: JobMeta,
    work: T,
}

impl<T> PartialEq for Item<T> {
    fn eq(&self, other: &Self) -> bool {
        self.key == other.key
    }
}
impl<T> Eq for Item<T> {}
impl<T> PartialOrd for Item<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<T> Ord for Item<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.key.cmp(&other.key)
    }
}

struct Inner<T> {
    heap: BinaryHeap<Item<T>>,
    seq: u64,
    closed: bool,
}

pub struct PrioQueue<T> {
    inner: Mutex<Inner<T>>,
    cv: Condvar,
}

impl<T> Default for PrioQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> PrioQueue<T> {
    pub fn new() -> Self {
        PrioQueue {
            inner: Mutex::new(Inner {
                heap: BinaryHeap::new(),
                seq: 0,
                closed: false,
            }),
            cv: Condvar::new(),
        }
    }

    pub fn push(&self, prio: Priority, meta: JobMeta, work: T) {
        {
            let mut inner = self.inner.lock();
            if inner.closed {
                return;
            }
            inner.seq += 1;
            let seq = inner.seq;
            inner.heap.push(Item {
                key: Reverse((prio as u8, seq)),
                meta,
                work,
            });
        }
        self.cv.notify_one();
    }

    /// Blocks until a job is available; returns None once closed and drained.
    pub fn pop_blocking(&self) -> Option<(JobMeta, T)> {
        let mut inner = self.inner.lock();
        loop {
            if let Some(item) = inner.heap.pop() {
                return Some((item.meta, item.work));
            }
            if inner.closed {
                return None;
            }
            self.cv.wait(&mut inner);
        }
    }

    pub fn len(&self) -> usize {
        self.inner.lock().heap.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.lock().heap.is_empty()
    }

    pub fn close(&self) {
        self.inner.lock().closed = true;
        self.cv.notify_all();
    }
}

#[derive(Default)]
pub struct GenerationMap {
    map: Mutex<HashMap<DocId, u64>>,
}

impl GenerationMap {
    pub fn current(&self, doc: DocId) -> u64 {
        *self.map.lock().get(&doc).unwrap_or(&0)
    }

    pub fn bump(&self, doc: DocId) -> u64 {
        let mut map = self.map.lock();
        let e = map.entry(doc).or_insert(0);
        *e += 1;
        *e
    }

    pub fn is_stale(&self, meta: &JobMeta) -> bool {
        meta.gen < self.current(meta.doc)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn meta(doc: DocId, generation: u64) -> JobMeta {
        JobMeta {
            doc,
            gen: generation,
        }
    }

    #[test]
    fn pops_by_priority_then_fifo_within_priority() {
        let q: PrioQueue<&'static str> = PrioQueue::new();
        q.push(Priority::Prefetch, meta(1, 0), "prefetch");
        q.push(Priority::VisiblePage, meta(1, 0), "page-a");
        q.push(Priority::VisiblePage, meta(1, 0), "page-b");
        q.push(Priority::VisibleThumb, meta(1, 0), "thumb");
        q.close();
        let order: Vec<&str> = std::iter::from_fn(|| q.pop_blocking().map(|(_, w)| w)).collect();
        assert_eq!(order, vec!["page-a", "page-b", "thumb", "prefetch"]);
    }

    #[test]
    fn close_unblocks_and_drains() {
        let q: Arc<PrioQueue<u32>> = Arc::new(PrioQueue::new());
        let q2 = Arc::clone(&q);
        let handle = std::thread::spawn(move || q2.pop_blocking());
        std::thread::sleep(std::time::Duration::from_millis(50));
        q.push(Priority::VisiblePage, meta(1, 0), 42);
        let got = handle.join().unwrap();
        assert_eq!(got.map(|(_, w)| w), Some(42));
        q.close();
        assert!(q.pop_blocking().is_none());
    }

    #[test]
    fn generation_bump_marks_older_jobs_stale() {
        let gens = GenerationMap::default();
        let g0 = gens.current(7);
        let m = meta(7, g0);
        assert!(!gens.is_stale(&m));
        let g1 = gens.bump(7);
        assert!(g1 > g0);
        assert!(gens.is_stale(&m), "job submitted at g0 is stale after bump");
        let fresh = meta(7, g1);
        assert!(!gens.is_stale(&fresh));
    }

    #[test]
    fn generations_are_per_document() {
        let gens = GenerationMap::default();
        let a = meta(1, gens.current(1));
        gens.bump(2);
        assert!(
            !gens.is_stale(&a),
            "bumping doc 2 must not stale doc 1 jobs"
        );
    }
}
