//! Per-process working set: resident-page tracking with a clock (second-chance) trimmer.

use crate::Error;
use alloc::collections::BTreeMap;
use alloc::vec::Vec;

/// Resident pages of one process, bounded by `[min, max]`.
pub struct WorkingSet {
    min: usize,
    max: usize,
    /// page address -> referenced bit
    pages: BTreeMap<u64, bool>,
    hand: u64,
}

impl WorkingSet {
    /// Creates a working set with the given soft minimum and hard maximum (in pages).
    pub fn new(min: usize, max: usize) -> Result<Self, Error> {
        if min > max || max == 0 {
            return Err(Error::BadSize);
        }
        Ok(Self { min, max, pages: BTreeMap::new(), hand: 0 })
    }

    /// Number of resident pages.
    pub fn size(&self) -> usize {
        self.pages.len()
    }

    /// Adjusts the limits; returns pages that must be trimmed to honour a lower maximum.
    pub fn set_limits(&mut self, min: usize, max: usize) -> Result<Vec<u64>, Error> {
        if min > max || max == 0 {
            return Err(Error::BadSize);
        }
        self.min = min;
        self.max = max;
        Ok(self.trim_to(max))
    }

    /// Records a page fault that made `page` resident; may evict to stay within the maximum.
    /// Returns the evicted pages (the caller unmaps them).
    pub fn fault_in(&mut self, page: u64) -> Vec<u64> {
        if let Some(r) = self.pages.get_mut(&page) {
            *r = true;
            return Vec::new();
        }
        let evicted = if self.pages.len() >= self.max { self.trim_to(self.max - 1) } else { Vec::new() };
        self.pages.insert(page, true);
        evicted
    }

    /// Marks a page as touched (hardware accessed bit sampled by the trimmer).
    pub fn touch(&mut self, page: u64) {
        if let Some(r) = self.pages.get_mut(&page) {
            *r = true;
        }
    }

    /// Removes a page (unmap/decommit).
    pub fn remove(&mut self, page: u64) -> bool {
        self.pages.remove(&page).is_some()
    }

    /// Memory-pressure trim: shrinks toward `min`, returning the pages dropped.
    pub fn trim(&mut self) -> Vec<u64> {
        self.trim_to(self.min)
    }

    fn trim_to(&mut self, target: usize) -> Vec<u64> {
        let mut out = Vec::new();
        while self.pages.len() > target {
            let hand = self.hand;
            let next = self.pages.range(hand..).chain(self.pages.range(..hand)).map(|(k, _)| *k).next();
            let p = match next {
                Some(p) => p,
                None => break,
            };
            let r = self.pages.get_mut(&p);
            match r {
                Some(bit) if *bit => {
                    *bit = false;                      // second chance
                    self.hand = p + 1;
                }
                Some(_) => {
                    self.pages.remove(&p);
                    self.hand = p + 1;
                    out.push(p);
                }
                None => break,
            }
        }
        out
    }
}
