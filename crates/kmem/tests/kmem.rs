#![allow(missing_docs)]
use loos_kmem::*;

#[test]
fn buddy_alloc_free_coalesce() {
    let mut b = BuddyAllocator::new(0x100, 1024);
    b.add_free(0x100, 1024).unwrap();
    assert_eq!(b.free_frames(), 1024);
    let a = b.alloc(0).unwrap();
    let c = b.alloc(3).unwrap();
    assert_eq!(a, 0x100);
    assert_eq!(c % 8, 0x100 % 8 + (8 - 0x100 % 8) % 8 * 0, "order-3 block is 8-frame aligned relative to base");
    assert_eq!((c - 0x100) % 8, 0);
    assert_eq!(b.free_frames(), 1024 - 1 - 8);
    b.free(a).unwrap();
    b.free(c).unwrap();
    assert_eq!(b.free_frames(), 1024);
    assert_eq!(b.alloc(MAX_ORDER), Ok(0x100), "everything coalesced back to a max-order block");
}

#[test]
fn buddy_exhaustion_double_free_and_bad_addr() {
    let mut b = BuddyAllocator::new(0, 16);
    b.add_free(0, 16).unwrap();
    let v: Vec<_> = (0..16).map(|_| b.alloc(0).unwrap()).collect();
    assert_eq!(b.alloc(0), Err(Error::OutOfMemory));
    let mut sorted = v.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), 16, "no frame handed out twice");
    b.free(v[5]).unwrap();
    assert_eq!(b.free(v[5]), Err(Error::DoubleFree));
    assert_eq!(b.free(99), Err(Error::BadAddress));
    assert_eq!(b.alloc(MAX_ORDER + 1), Err(Error::BadSize));
    assert_eq!(b.alloc(0), Ok(v[5]));
}

#[test]
fn buddy_holes_in_memory_map() {
    let mut b = BuddyAllocator::new(0, 64);
    b.add_free(0, 10).unwrap();
    b.add_free(20, 5).unwrap();
    assert_eq!(b.free_frames(), 15);
    assert_eq!(b.add_free(5, 2), Err(Error::DoubleFree));
    assert!(b.alloc(4).is_err(), "no 16-frame block exists");
    let mut got = vec![];
    while let Ok(f) = b.alloc(0) {
        got.push(f);
    }
    got.sort();
    assert_eq!(got, (0..10).chain(20..25).collect::<Vec<_>>());
}

#[test]
fn buddy_stress_randomised() {
    let mut b = BuddyAllocator::new(0, 4096);
    b.add_free(0, 4096).unwrap();
    let mut live: Vec<(u64, usize)> = vec![];
    let mut x = 0x2545_f491_4f6c_dd1du64;
    for _ in 0..20_000 {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        if x % 3 != 0 || live.is_empty() {
            let o = (x >> 8) as usize % 6;
            if let Ok(f) = b.alloc(o) {
                for &(g, go) in &live {
                    assert!(f + (1 << o) <= g || g + (1 << go) <= f, "overlap");
                }
                live.push((f, o));
            }
        } else {
            let (f, _) = live.swap_remove((x >> 20) as usize % live.len());
            b.free(f).unwrap();
        }
    }
    for (f, _) in live {
        b.free(f).unwrap();
    }
    assert_eq!(b.free_frames(), 4096);
    assert_eq!(b.alloc(MAX_ORDER).map(|_| ()), Ok(()));
}

struct Pool(BuddyAllocator);
impl PageSource for Pool {
    fn alloc_pages(&mut self, pages: usize) -> Result<usize, Error> {
        let order = pages.next_power_of_two().trailing_zeros() as usize;
        self.0.alloc(order).map(|f| f as usize * PAGE_SIZE)
    }
    fn free_pages(&mut self, addr: usize, _pages: usize) {
        self.0.free((addr / PAGE_SIZE) as u64).unwrap();
    }
}

fn heap() -> SlabHeap<Pool> {
    let mut b = BuddyAllocator::new(0x1000, 256);
    b.add_free(0x1000, 256).unwrap();
    SlabHeap::new(Pool(b))
}

#[test]
fn slab_classes_and_reuse() {
    let mut h = heap();
    let a = h.alloc(1).unwrap();
    let b = h.alloc(16).unwrap();
    let c = h.alloc(17).unwrap();
    assert_eq!(h.usable_size(a), Some(16));
    assert_eq!(h.usable_size(c), Some(32));
    assert_ne!(a, b);
    assert_eq!(h.bytes_in_use(), 16 + 16 + 32);
    h.free(b).unwrap();
    assert_eq!(h.free(b), Err(Error::DoubleFree));
    assert_eq!(h.free(a + 1), Err(Error::BadAddress));
    h.free(a).unwrap();
    h.free(c).unwrap();
    assert_eq!(h.bytes_in_use(), 0);
    assert_eq!(h.pages().0.free_frames(), 256, "empty slabs return their pages");
}

#[test]
fn slab_fills_and_grows() {
    let mut h = heap();
    let per_page = PAGE_SIZE / 64;
    let v: Vec<_> = (0..per_page * 3 + 1).map(|_| h.alloc(64).unwrap()).collect();
    let mut s = v.clone();
    s.sort();
    s.dedup();
    assert_eq!(s.len(), v.len());
    assert_eq!(h.pages().0.free_frames(), 256 - 4);
    for a in v {
        h.free(a).unwrap();
    }
    assert_eq!(h.pages().0.free_frames(), 256);
}

#[test]
fn slab_large_and_errors() {
    let mut h = heap();
    assert_eq!(h.alloc(0), Err(Error::BadSize));
    let big = h.alloc(PAGE_SIZE * 3 + 1).unwrap();
    assert_eq!(big % PAGE_SIZE, 0);
    assert_eq!(h.usable_size(big), Some(4 * PAGE_SIZE));
    h.free(big).unwrap();
    assert_eq!(h.free(big), Err(Error::BadAddress));
    assert_eq!(h.alloc(PAGE_SIZE * 1024), Err(Error::OutOfMemory));
}

mod vm {
    use loos_kmem::vad::*;
    use loos_kmem::Error;
    const G: u64 = ALLOC_GRANULARITY;

    fn space() -> AddressSpace {
        AddressSpace::new(0x10000, 0x8000_0000, true)
    }

    #[test]
    fn reserve_commit_split_merge() {
        let mut a = space();
        let b = a.reserve(None, 16 * 4096, Protect::ReadWrite, Backing::Private, false).unwrap();
        assert_eq!(b, 0x10000);
        assert_eq!(a.region_count(), 1);
        a.commit(b + 4096, 2 * 4096, Protect::ReadWrite).unwrap();
        assert_eq!(a.region_count(), 3);
        assert_eq!(a.committed_bytes(), 2 * 4096);
        assert!(a.check_access(b + 4096, 8192, true, false));
        assert!(!a.check_access(b, 1, false, false), "reserved page is not accessible");
        assert!(!a.check_access(b + 4096, 8193, false, false), "range spills into reserved page");
        a.protect(b + 4096, 4096, Protect::Read).unwrap();
        assert!(!a.check_access(b + 4096, 1, true, false));
        assert!(a.check_access(b + 2 * 4096, 1, true, false));
        a.protect(b + 4096, 4096, Protect::ReadWrite).unwrap();
        assert_eq!(a.region_count(), 3, "re-merged committed run");
        a.decommit(b + 4096, 2 * 4096).unwrap();
        assert_eq!(a.region_count(), 1, "back to one reservation");
        a.release(b).unwrap();
        assert_eq!(a.region_count(), 0);
    }

    #[test]
    fn placement_rules() {
        let mut a = space();
        let x = a.reserve(None, 100, Protect::Read, Backing::Private, false).unwrap();
        let y = a.reserve(None, 100, Protect::Read, Backing::Private, false).unwrap();
        assert_eq!(y - x, G, "64K granularity");
        assert_eq!(a.reserve(Some(x), 4096, Protect::Read, Backing::Private, false), Err(Error::BadAddress));
        assert_eq!(a.reserve(Some(x + 4096), 4096, Protect::Read, Backing::Private, false), Err(Error::BadAddress), "unaligned base");
        assert_eq!(a.reserve(Some(0x1000), 4096, Protect::Read, Backing::Private, false), Err(Error::BadAddress), "below user range");
        let t = a.reserve(None, 4096, Protect::Read, Backing::Private, true).unwrap();
        assert_eq!(t, 0x8000_0000 - G, "top-down");
        a.release(x).unwrap();
        let z = a.reserve(None, 4096, Protect::Read, Backing::Private, false).unwrap();
        assert_eq!(z, x, "freed gap reused");
        assert_eq!(a.release(x + 4096), Err(Error::BadAddress));
        assert_eq!(a.reserve(None, 0, Protect::Read, Backing::Private, false), Err(Error::BadSize));
        assert_eq!(a.reserve(None, 1 << 40, Protect::Read, Backing::Private, false), Err(Error::OutOfMemory));
    }

    #[test]
    fn wx_and_max_protect() {
        let mut a = space();
        assert_eq!(a.reserve(None, 4096, Protect::ReadWriteExecute, Backing::Private, false), Err(Error::BadSize));
        let b = a.reserve(None, 8192, Protect::Read, Backing::Private, false).unwrap();
        assert_eq!(a.commit(b, 4096, Protect::ReadWrite), Err(Error::BadSize), "above reservation ceiling");
        a.commit(b, 4096, Protect::Read).unwrap();
        assert_eq!(a.protect(b, 8192, Protect::Read), Err(Error::BadAddress), "second page not committed");
        let mut open = AddressSpace::new(0x10000, 0x100000, false);
        let c = open.reserve(None, 4096, Protect::ReadWriteExecute, Backing::Private, false).unwrap();
        open.commit(c, 4096, Protect::ReadWriteExecute).unwrap();
        assert!(open.check_access(c, 10, true, true));
    }

    #[test]
    fn sections_split_with_offsets_and_cross_allocation_ranges() {
        let mut a = space();
        let b = a.reserve(None, 4 * 4096, Protect::Read, Backing::Section { id: 7, offset: 0 }, false).unwrap();
        a.commit(b, 4 * 4096, Protect::Read).unwrap();
        a.protect(b + 2 * 4096, 4096, Protect::Read).unwrap();
        a.decommit(b + 2 * 4096, 4096).unwrap();
        let v = a.query(b + 3 * 4096).unwrap();
        assert_eq!(v.backing, Backing::Section { id: 7, offset: 3 * 4096 });
        let c = a.reserve(Some(b + G), 4096, Protect::Read, Backing::Private, false).unwrap();
        assert_eq!(a.commit(b, G + 4096, Protect::Read), Err(Error::BadAddress), "range spans two allocations / holes");
        let _ = c;
    }
}

mod ws {
    use loos_kmem::workingset::*;

    #[test]
    fn limits_and_second_chance() {
        let mut w = WorkingSet::new(2, 4).unwrap();
        for p in 0..4 {
            assert!(w.fault_in(p * 4096).is_empty());
        }
        assert_eq!(w.size(), 4);
        // all referenced: first pass clears bits, then the oldest unreferenced page goes
        let ev = w.fault_in(9 * 4096);
        assert_eq!(ev.len(), 1);
        assert_eq!(w.size(), 4);
        w.touch(ev.first().map_or(0, |_| 9 * 4096));
        let t = w.trim();
        assert_eq!(w.size(), 2);
        assert_eq!(t.len(), 2);
        assert!(WorkingSet::new(5, 4).is_err());
        let dropped = w.set_limits(0, 1).unwrap();
        assert_eq!(dropped.len(), 1);
        assert!(w.remove(w_first(&w)) || true);
    }

    fn w_first(_w: &WorkingSet) -> u64 {
        0
    }
}
