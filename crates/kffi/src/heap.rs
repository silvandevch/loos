//! Kernel heap behind Rust's global allocator.
//!
//! The boot `kalloc` is a bump allocator that never frees, so long-running programs (the Linux layer's
//! page copies, window frames, file buffers) used to leak until the machine ran out of memory. This
//! heap keeps its free lists *inside* the free memory, so it never allocates for itself:
//!
//! * blocks up to 2 KiB: power-of-two size classes, each a singly linked free list of equal blocks
//!   carved from 64 KiB chunks (a block of class `c` is `c`-aligned);
//! * larger blocks: whole 4 KiB pages from an address-ordered free list of runs, first fit, split on
//!   allocation and merged with both neighbours on free.
//!
//! Fresh memory comes from a `grow` callback (the bump allocator in the kernel, a host buffer in tests).

use core::alloc::Layout;
use core::ptr::null_mut;

const CLASSES: usize = 8; // 16 .. 2048
const SMALL_MAX: usize = 2048;
const CHUNK: usize = 64 << 10;
const PAGE: usize = 4096;
const GROW_MIN: usize = 1 << 20;

/// A free page run, stored at its own start.
struct Run {
    pages: usize,
    next: *mut Run,
}

/// The allocator state. `grow(size, align)` returns fresh memory or null.
pub struct Heap {
    small: [*mut u8; CLASSES],
    runs: *mut Run,
    grow: fn(usize, usize) -> *mut u8,
}

fn class(size: usize) -> usize {
    size.max(16).next_power_of_two().trailing_zeros() as usize - 4
}

impl Heap {
    /// An empty heap drawing fresh memory from `grow`.
    pub const fn new(grow: fn(usize, usize) -> *mut u8) -> Self {
        Self { small: [null_mut(); CLASSES], runs: null_mut(), grow }
    }

    /// Allocates `l` (null when out of memory).
    ///
    /// # Safety
    /// Single-threaded use only; the returned memory is uninitialised.
    pub unsafe fn alloc(&mut self, l: Layout) -> *mut u8 {
        let size = l.size().max(l.align());
        if l.align() > PAGE {
            return (self.grow)(l.size(), l.align()); // never freed: not used by kernel code paths
        }
        if size <= SMALL_MAX {
            let c = class(size);
            if self.small[c].is_null() && !self.refill(c) {
                return null_mut();
            }
            let b = self.small[c];
            // SAFETY: free blocks hold the next pointer in their first word.
            self.small[c] = unsafe { *(b as *mut *mut u8) };
            return b;
        }
        let pages = size.div_ceil(PAGE);
        // SAFETY: the run list only contains memory owned by this heap.
        unsafe { self.take_pages(pages) }
    }

    /// Returns a block from [`Self::alloc`] with the same layout.
    ///
    /// # Safety
    /// `p` must come from `alloc(l)` on this heap and not be freed twice.
    pub unsafe fn dealloc(&mut self, p: *mut u8, l: Layout) {
        let size = l.size().max(l.align());
        if l.align() > PAGE {
            return;
        }
        if size <= SMALL_MAX {
            let c = class(size);
            // SAFETY: the block is ours and at least pointer sized (classes start at 16 bytes).
            unsafe { *(p as *mut *mut u8) = self.small[c] };
            self.small[c] = p;
            return;
        }
        // SAFETY: as above, page aligned and `pages` long.
        unsafe { self.free_pages(p, size.div_ceil(PAGE)) }
    }

    fn refill(&mut self, c: usize) -> bool {
        let size = 16usize << c;
        // SAFETY: a fresh chunk from the page allocator, carved into blocks linked through their first word.
        unsafe {
            let chunk = self.take_pages(CHUNK / PAGE);
            if chunk.is_null() {
                return false;
            }
            for i in (0..CHUNK / size).rev() {
                let b = chunk.add(i * size);
                *(b as *mut *mut u8) = self.small[c];
                self.small[c] = b;
            }
        }
        true
    }

    /// First fit from the run list; grows the heap when nothing fits.
    unsafe fn take_pages(&mut self, pages: usize) -> *mut u8 {
        // SAFETY: runs are valid, disjoint, owned by the heap.
        unsafe {
            let mut link: *mut *mut Run = &mut self.runs;
            while !(*link).is_null() {
                let r = *link;
                if (*r).pages >= pages {
                    if (*r).pages == pages {
                        *link = (*r).next;
                    } else {
                        // Hand out the tail: the run keeps its place in the list.
                        (*r).pages -= pages;
                        return (r as *mut u8).add((*r).pages * PAGE);
                    }
                    return r as *mut u8;
                }
                link = &mut (*r).next;
            }
            let want = (pages * PAGE).max(GROW_MIN);
            let fresh = (self.grow)(want, PAGE);
            if fresh.is_null() {
                // Not enough for the batch: try the exact size.
                let exact = (self.grow)(pages * PAGE, PAGE);
                return exact;
            }
            if want > pages * PAGE {
                self.free_pages(fresh.add(pages * PAGE), want / PAGE - pages);
            }
            fresh
        }
    }

    /// Inserts a run in address order, merging with its neighbours.
    unsafe fn free_pages(&mut self, p: *mut u8, pages: usize) {
        // SAFETY: `p` is a run we handed out; list nodes are valid.
        unsafe {
            let node = p as *mut Run;
            let mut prev: *mut Run = null_mut();
            let mut cur = self.runs;
            while !cur.is_null() && (cur as usize) < node as usize {
                prev = cur;
                cur = (*cur).next;
            }
            (*node).pages = pages;
            (*node).next = cur;
            if !cur.is_null() && node as usize + pages * PAGE == cur as usize {
                (*node).pages += (*cur).pages;
                (*node).next = (*cur).next;
            }
            if prev.is_null() {
                self.runs = node;
            } else if prev as usize + (*prev).pages * PAGE == node as usize {
                (*prev).pages += (*node).pages;
                (*prev).next = (*node).next;
            } else {
                (*prev).next = node;
            }
        }
    }

    /// Bytes on the large-run free list (diagnostics and tests).
    pub fn free_run_bytes(&self) -> usize {
        let (mut n, mut r) = (0, self.runs);
        while !r.is_null() {
            // SAFETY: list nodes are valid.
            unsafe {
                n += (*r).pages * PAGE;
                r = (*r).next;
            }
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    thread_local! {
        static ARENA: RefCell<(usize, usize)> = const { RefCell::new((0, 0)) };
    }

    fn grow(size: usize, align: usize) -> *mut u8 {
        ARENA.with(|a| {
            let mut a = a.borrow_mut();
            if a.1 == 0 {
                let buf = Box::leak(vec![0u8; 64 << 20].into_boxed_slice());
                *a = (buf.as_mut_ptr() as usize, buf.as_mut_ptr() as usize + buf.len());
            }
            let p = (a.0 + align - 1) & !(align - 1);
            if p + size > a.1 {
                return null_mut();
            }
            a.0 = p + size;
            p as *mut u8
        })
    }

    #[test]
    fn freed_memory_is_reused_instead_of_growing() {
        let mut h = Heap::new(grow);
        let big = Layout::from_size_align(3 << 20, 8).unwrap();
        // Without frees a 64 MiB arena holds ~21 of these; with reuse, any number fits.
        for _ in 0..200 {
            let p = unsafe { h.alloc(big) };
            assert!(!p.is_null(), "large blocks are recycled");
            assert_eq!(p as usize % PAGE, 0);
            unsafe { h.dealloc(p, big) };
        }
        let small = Layout::from_size_align(24, 8).unwrap();
        let mut v = Vec::new();
        for _ in 0..100_000 {
            let p = unsafe { h.alloc(small) };
            assert_eq!(p as usize % 32, 0, "class alignment");
            v.push(p);
        }
        for p in v.drain(..) {
            unsafe { h.dealloc(p, small) };
        }
        let again = unsafe { h.alloc(small) };
        assert!(!again.is_null());
    }

    #[test]
    fn neighbouring_runs_merge_back_into_one() {
        let mut h = Heap::new(grow);
        let l = Layout::from_size_align(8 * PAGE, PAGE).unwrap();
        let ps: Vec<*mut u8> = (0..6).map(|_| unsafe { h.alloc(l) }).collect();
        let before = h.free_run_bytes();
        for &p in ps.iter().rev().step_by(2).chain(ps.iter().step_by(2)) {
            unsafe { h.dealloc(p, l) };
        }
        assert_eq!(h.free_run_bytes(), before + 6 * 8 * PAGE);
        // fully merged: one allocation of all of it fits without growing
        let all = Layout::from_size_align(before + 48 * PAGE, PAGE).unwrap();
        let p = unsafe { h.alloc(all) };
        assert!(!p.is_null());
        assert_eq!(h.free_run_bytes(), 0);
    }
}
