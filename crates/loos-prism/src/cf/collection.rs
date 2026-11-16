//! Storage of CFData, CFArray, CFDictionary and CFSet. Data and arrays are a buffer of elements
//! (bytes or object pointers) `{data, count, capacity}`; dictionaries and sets are an open-addressing
//! hash table of `{hash | OCCUPIED, key, value}` slots. Retaining, releasing and comparing the
//! objects in them is the class layer's part: only it can send messages.

use super::{rd, wr, F_OWNED, O_CAP, O_COUNT, O_DATA, O_FLAGS};
use crate::libsys::{Act, Call, Func};
use alloc::{vec, vec::Vec};

macro_rules! f {
    ($name:literal, |$c:ident| $body:expr) => {
        ($name, { fn h($c: &mut Call) -> Act { $body } h } as Func)
    };
}

/// Give `obj` a buffer of its own with room for `want` elements of `elem` bytes.
fn reserve(c: &mut Call, obj: u64, elem: u64, want: u64) {
    let (flags, data, count, cap) = (rd(c, obj + O_FLAGS), rd(c, obj + O_DATA), rd(c, obj + O_COUNT), rd(c, obj + O_CAP));
    let owned = flags & F_OWNED != 0;
    if owned && cap >= want {
        return;
    }
    let new_cap = want.max(if owned { cap * 2 } else { 0 }).max(16);
    let new = c.st.heap.alloc(c.mem, new_cap * elem);
    if data != 0 {
        if let Ok(old) = c.mem.read_vec(data, (count * elem) as usize) {
            let _ = c.mem.write(new, &old);
        }
        if owned {
            c.st.heap.free(c.mem, data);
        }
    }
    wr(c, obj + O_FLAGS, flags | F_OWNED);
    wr(c, obj + O_DATA, new);
    wr(c, obj + O_CAP, new_cap);
}

const OCCUPIED: u64 = 1 << 63;
const SLOT: u64 = 24;

fn slot(c: &mut Call, table: u64, i: u64) -> (u64, u64, u64) {
    (rd(c, table + i * SLOT), rd(c, table + i * SLOT + 8), rd(c, table + i * SLOT + 16))
}
fn set_slot(c: &mut Call, table: u64, i: u64, s: (u64, u64, u64)) {
    wr(c, table + i * SLOT, s.0);
    wr(c, table + i * SLOT + 8, s.1);
    wr(c, table + i * SLOT + 16, s.2);
}
/// First free slot for `hash` (the table always has one).
fn free_slot(c: &mut Call, table: u64, cap: u64, hash: u64) -> u64 {
    let mut i = hash % cap;
    while slot(c, table, i).0 != 0 {
        i = (i + 1) % cap;
    }
    i
}

pub static FUNCS: &[(&str, Func)] = &[
    // (object, element size, loc, len, source, n): replace `len` elements at `loc` with `n` from `source`
    f!("___prism_cf_vec_splice", |c| {
        let (obj, elem, loc, len, src, n) = (c.arg(), c.arg(), c.arg(), c.arg(), c.arg(), c.arg());
        let count = rd(c, obj + O_COUNT);
        let loc = loc.min(count);
        let len = len.min(count - loc);
        let ins = if n == 0 { Vec::new() } else if src == 0 { vec![0u8; (n * elem) as usize] } else { c.mem.read_vec(src, (n * elem) as usize).unwrap_or_default() };
        reserve(c, obj, elem, count - len + n + (elem == 1) as u64);
        let data = rd(c, obj + O_DATA);
        let tail = c.mem.read_vec(data + (loc + len) * elem, ((count - loc - len) * elem) as usize).unwrap_or_default();
        let _ = c.mem.write(data + loc * elem, &ins);
        let _ = c.mem.write(data + (loc + n) * elem, &tail);
        wr(c, obj + O_COUNT, count - len + n);
        c.ret(1)
    }),
    // (object, element size, count): grow with zeros or cut
    f!("___prism_cf_vec_resize", |c| {
        let (obj, elem, n) = (c.arg(), c.arg(), c.arg());
        let count = rd(c, obj + O_COUNT);
        reserve(c, obj, elem, n);
        if n > count {
            let data = rd(c, obj + O_DATA);
            let _ = c.mem.write(data + count * elem, &vec![0u8; ((n - count) * elem) as usize]);
        }
        wr(c, obj + O_COUNT, n);
        c.ret(1)
    }),
    f!("___prism_cf_bytes_hash", |c| {
        let (p, n) = (c.arg(), c.arg());
        let b = c.mem.read_vec(p, n.min(80) as usize).unwrap_or_default();
        c.ret(b.iter().fold(0xcbf2_9ce4_8422_2325u64 ^ n, |h, &u| (h ^ u as u64).wrapping_mul(0x100_0000_01b3)) >> 1)
    }),
    // (hay, hay length, needle, needle length, backwards) → offset or -1
    f!("___prism_cf_bytes_find", |c| {
        let (h, hn, nd, nn, back) = (c.arg(), c.arg(), c.arg(), c.arg(), c.arg());
        let hay = c.mem.read_vec(h, hn as usize).unwrap_or_default();
        let needle = c.mem.read_vec(nd, nn as usize).unwrap_or_default();
        let r = if needle.is_empty() || needle.len() > hay.len() {
            None
        } else if back != 0 {
            hay.windows(needle.len()).rposition(|w| w == needle)
        } else {
            hay.windows(needle.len()).position(|w| w == needle)
        };
        c.ret(r.map(|i| i as u64).unwrap_or(u64::MAX))
    }),
    // (table object, hash, cursor) → the next slot from `cursor` (-1: the start) holding this
    // hash, or -1 once the probe sequence ends. The caller compares the keys.
    f!("___prism_cf_dict_probe", |c| {
        let (obj, hash, cursor) = (c.arg(), c.arg() | OCCUPIED, c.arg() as i64);
        let (table, cap) = (rd(c, obj + O_DATA), rd(c, obj + O_CAP));
        if table == 0 || cap == 0 {
            return c.ret(u64::MAX);
        }
        let mut i = if cursor < 0 { hash % cap } else { cursor as u64 % cap };
        for _ in 0..cap {
            let h = slot(c, table, i).0;
            if h == 0 {
                break;
            }
            if h == hash {
                return c.ret(i);
            }
            i = (i + 1) % cap;
        }
        c.ret(u64::MAX)
    }),
    // (table object, hash, key, value) → the slot of the new entry
    f!("___prism_cf_dict_insert", |c| {
        let (obj, hash, key, value) = (c.arg(), c.arg() | OCCUPIED, c.arg(), c.arg());
        let (mut table, count, mut cap) = (rd(c, obj + O_DATA), rd(c, obj + O_COUNT), rd(c, obj + O_CAP));
        if (count + 1) * 3 > cap * 2 {
            let new_cap = (cap * 2).max(8);
            let new = c.st.heap.alloc(c.mem, new_cap * SLOT);
            let _ = c.mem.write(new, &vec![0u8; (new_cap * SLOT) as usize]);
            for i in 0..cap {
                let s = slot(c, table, i);
                if s.0 != 0 {
                    let at = free_slot(c, new, new_cap, s.0);
                    set_slot(c, new, at, s);
                }
            }
            if table != 0 {
                c.st.heap.free(c.mem, table);
            }
            table = new;
            cap = new_cap;
            wr(c, obj + O_DATA, table);
            wr(c, obj + O_CAP, cap);
        }
        let at = free_slot(c, table, cap, hash);
        set_slot(c, table, at, (hash, key, value));
        wr(c, obj + O_COUNT, count + 1);
        c.ret(at)
    }),
    // (table object, slot): remove the entry, closing the gap in its probe sequence
    f!("___prism_cf_dict_remove", |c| {
        let (obj, mut i) = (c.arg(), c.arg());
        let (table, count, cap) = (rd(c, obj + O_DATA), rd(c, obj + O_COUNT), rd(c, obj + O_CAP));
        if table == 0 || i >= cap {
            return c.ret(0);
        }
        let mut j = (i + 1) % cap;
        loop {
            let s = slot(c, table, j);
            if s.0 == 0 {
                break;
            }
            let home = s.0 % cap;
            if (j + cap - home) % cap >= (j + cap - i) % cap {
                set_slot(c, table, i, s);
                i = j;
            }
            j = (j + 1) % cap;
        }
        set_slot(c, table, i, (0, 0, 0));
        wr(c, obj + O_COUNT, count - 1);
        c.ret(1)
    }),
    f!("___prism_cf_dict_clear", |c| {
        let obj = c.arg();
        let table = rd(c, obj + O_DATA);
        if table != 0 {
            c.st.heap.free(c.mem, table);
        }
        wr(c, obj + O_DATA, 0);
        wr(c, obj + O_COUNT, 0);
        wr(c, obj + O_CAP, 0);
        c.ret(1)
    }),
];
