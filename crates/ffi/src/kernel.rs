//! Executive pieces: object manager + syscalls (loos-kernel), dispatcher (loos-ksync), memory (loos-kmem),
//! registry hives (loos-registry), hardware tables (loos-khal) and input decoders (loos-drivers).

use crate::{bytes, bytes_mut, cstr};
use std::ffi::c_char;

// ------------------------------------------------------------------ loos-kernel
use loos_kernel::{dispatch, ObjectManager, SyscallArgs};

#[no_mangle]
pub extern "C" fn loos_kobj_new() -> *mut ObjectManager {
    Box::into_raw(Box::new(ObjectManager::new()))
}
#[no_mangle]
pub unsafe extern "C" fn loos_kobj_free(m: *mut ObjectManager) {
    if !m.is_null() {
        // SAFETY: allocated by loos_kobj_new.
        drop(unsafe { Box::from_raw(m) });
    }
}
/// Dispatches one executive syscall. Returns the Status code (0 = success; see loos_kernel::Status) and stores the
/// primary result in `*value`.
#[no_mangle]
pub unsafe extern "C" fn loos_kobj_syscall(m: *mut ObjectManager, number: u32, args6: *const u64, value: *mut u64) -> i32 {
    // SAFETY: `args6` has six entries, `value` is writable.
    unsafe {
        let Some(m) = m.as_mut() else { return -1 };
        let mut a = [0u64; 6];
        if !args6.is_null() {
            a.copy_from_slice(std::slice::from_raw_parts(args6, 6));
        }
        let r = dispatch(m, number, SyscallArgs(a));
        if !value.is_null() {
            *value = r.value;
        }
        r.status as i32
    }
}

// ------------------------------------------------------------------ loos-ksync
use loos_ksync::{Dispatcher, EventKind, ObjId, WaitKind, WaitStart, WaitStatus};

#[no_mangle]
pub extern "C" fn loos_disp_new() -> *mut Dispatcher {
    Box::into_raw(Box::new(Dispatcher::new()))
}
#[no_mangle]
pub unsafe extern "C" fn loos_disp_free(d: *mut Dispatcher) {
    if !d.is_null() {
        // SAFETY: allocated by loos_disp_new.
        drop(unsafe { Box::from_raw(d) });
    }
}
/// Object ids are non-zero 64-bit values; 0 means failure.
#[no_mangle]
pub unsafe extern "C" fn loos_disp_create_event(d: *mut Dispatcher, synchronization: bool, signaled: bool) -> u64 {
    // SAFETY: see crate contract.
    unsafe { d.as_mut().map_or(0, |d| d.create_event(if synchronization { EventKind::Synchronization } else { EventKind::Notification }, signaled).to_raw()) }
}
#[no_mangle]
pub unsafe extern "C" fn loos_disp_create_semaphore(d: *mut Dispatcher, initial: u32, max: u32) -> u64 {
    // SAFETY: see crate contract.
    unsafe { d.as_mut().and_then(|d| d.create_semaphore(initial, max).ok()).map_or(0, |o| o.to_raw()) }
}
#[no_mangle]
pub unsafe extern "C" fn loos_disp_create_mutex(d: *mut Dispatcher, owner_thread: u64) -> u64 {
    // SAFETY: see crate contract.
    unsafe { d.as_mut().map_or(0, |d| d.create_mutex(if owner_thread == 0 { None } else { Some(owner_thread) }).to_raw()) }
}
#[no_mangle]
pub unsafe extern "C" fn loos_disp_set_event(d: *mut Dispatcher, id: u64) -> i32 {
    // SAFETY: see crate contract.
    unsafe { d.as_mut().zip(ObjId::from_raw(id)).map_or(-1, |(d, o)| d.set_event(o).map_or(-1, |_| 0)) }
}
#[no_mangle]
pub unsafe extern "C" fn loos_disp_reset_event(d: *mut Dispatcher, id: u64) -> i32 {
    // SAFETY: see crate contract.
    unsafe { d.as_mut().zip(ObjId::from_raw(id)).map_or(-1, |(d, o)| d.reset_event(o).map_or(-1, |_| 0)) }
}
#[no_mangle]
pub unsafe extern "C" fn loos_disp_release_semaphore(d: *mut Dispatcher, id: u64, n: u32) -> i64 {
    // SAFETY: see crate contract.
    unsafe { d.as_mut().zip(ObjId::from_raw(id)).map_or(-1, |(d, o)| d.release_semaphore(o, n).map_or(-1, |p| p as i64)) }
}
#[no_mangle]
pub unsafe extern "C" fn loos_disp_release_mutex(d: *mut Dispatcher, id: u64, thread: u64) -> i64 {
    // SAFETY: see crate contract.
    unsafe { d.as_mut().zip(ObjId::from_raw(id)).map_or(-1, |(d, o)| d.release_mutex(o, thread).map_or(-1, |p| p as i64)) }
}

fn status_code(s: WaitStatus) -> (i32, u32) {
    match s {
        WaitStatus::Signaled(i) => (0, i as u32),
        WaitStatus::Abandoned(i) => (1, i as u32),
        WaitStatus::Timeout => (2, 0),
        WaitStatus::Cancelled => (3, 0),
        WaitStatus::Alerted => (5, 0),
    }
}

/// Starts a wait. Returns the status kind: 0 signaled, 1 abandoned, 2 timeout, 3 cancelled (index in `*index`),
/// 4 blocked (the result arrives through loos_disp_drain), -1 error. `timeout_ticks` < 0 waits forever.
#[no_mangle]
pub unsafe extern "C" fn loos_disp_wait(d: *mut Dispatcher, thread: u64, ids: *const u64, n: usize, wait_all: bool, timeout_ticks: i64, index: *mut u32) -> i32 {
    // SAFETY: `ids` has n entries.
    unsafe {
        let Some(d) = d.as_mut() else { return -1 };
        let mut objs = Vec::new();
        for &r in std::slice::from_raw_parts(ids, n) {
            match ObjId::from_raw(r) {
                Some(o) => objs.push(o),
                None => return -1,
            }
        }
        match d.wait(thread, &objs, if wait_all { WaitKind::All } else { WaitKind::Any }, (timeout_ticks >= 0).then_some(timeout_ticks as u64)) {
            Ok(WaitStart::Done(s)) => {
                let (k, i) = status_code(s);
                *index = i;
                k
            }
            Ok(WaitStart::Blocked) => 4,
            Err(_) => -1,
        }
    }
}
#[no_mangle]
pub unsafe extern "C" fn loos_disp_advance(d: *mut Dispatcher, ticks: u64) {
    // SAFETY: see crate contract.
    if let Some(d) = unsafe { d.as_mut() } {
        d.advance(ticks);
    }
}
#[no_mangle]
pub unsafe extern "C" fn loos_disp_thread_exit(d: *mut Dispatcher, thread: u64) {
    // SAFETY: see crate contract.
    if let Some(d) = unsafe { d.as_mut() } {
        d.thread_exit(thread);
    }
}
/// Collects finished waits: threads[i], kinds[i] (0..3 as above), indices[i]. Returns the count written.
#[no_mangle]
pub unsafe extern "C" fn loos_disp_drain(d: *mut Dispatcher, threads: *mut u64, kinds: *mut i32, indices: *mut u32, cap: usize) -> usize {
    // SAFETY: arrays have `cap` entries.
    unsafe {
        let Some(d) = d.as_mut() else { return 0 };
        let w = d.drain_wakeups();
        for (i, x) in w.iter().enumerate().take(cap) {
            let (k, idx) = status_code(x.status);
            *threads.add(i) = x.thread;
            *kinds.add(i) = k;
            *indices.add(i) = idx;
        }
        w.len().min(cap)
    }
}

// ------------------------------------------------------------------ loos-kmem
use loos_kmem::vad::{AddressSpace, Backing, Protect};
use loos_kmem::workingset::WorkingSet;
use loos_kmem::BuddyAllocator;

#[no_mangle]
pub extern "C" fn loos_buddy_new(base_frame: u64, frames: u64) -> *mut BuddyAllocator {
    Box::into_raw(Box::new(BuddyAllocator::new(base_frame, frames)))
}
#[no_mangle]
pub unsafe extern "C" fn loos_buddy_free_allocator(b: *mut BuddyAllocator) {
    if !b.is_null() {
        // SAFETY: allocated by loos_buddy_new.
        drop(unsafe { Box::from_raw(b) });
    }
}
#[no_mangle]
pub unsafe extern "C" fn loos_buddy_add_free(b: *mut BuddyAllocator, start: u64, count: u64) -> i32 {
    // SAFETY: see crate contract.
    unsafe { b.as_mut().map_or(-1, |b| b.add_free(start, count).map_or(-1, |_| 0)) }
}
/// Allocates 2^order frames; returns the first frame number, or -1.
#[no_mangle]
pub unsafe extern "C" fn loos_buddy_alloc(b: *mut BuddyAllocator, order: u32) -> i64 {
    // SAFETY: see crate contract.
    unsafe { b.as_mut().and_then(|b| b.alloc(order as usize).ok()).map_or(-1, |f| f as i64) }
}
#[no_mangle]
pub unsafe extern "C" fn loos_buddy_release(b: *mut BuddyAllocator, frame: u64) -> i32 {
    // SAFETY: see crate contract.
    unsafe { b.as_mut().map_or(-1, |b| b.free(frame).map_or(-1, |_| 0)) }
}
#[no_mangle]
pub unsafe extern "C" fn loos_buddy_free_frames(b: *const BuddyAllocator) -> u64 {
    // SAFETY: see crate contract.
    unsafe { b.as_ref().map_or(0, |b| b.free_frames()) }
}

fn prot(p: u32) -> Protect {
    match p {
        1 => Protect::Read,
        2 => Protect::ReadWrite,
        3 => Protect::ReadExecute,
        4 => Protect::ReadWriteExecute,
        _ => Protect::None,
    }
}

#[no_mangle]
pub extern "C" fn loos_vas_new(lo: u64, hi: u64, wx_enforced: bool) -> *mut AddressSpace {
    Box::into_raw(Box::new(AddressSpace::new(lo, hi, wx_enforced)))
}
#[no_mangle]
pub unsafe extern "C" fn loos_vas_free(a: *mut AddressSpace) {
    if !a.is_null() {
        // SAFETY: allocated by loos_vas_new.
        drop(unsafe { Box::from_raw(a) });
    }
}
/// Reserves address space (`at` 0 = anywhere). protect: 0 none 1 R 2 RW 3 RX 4 RWX. Returns the base, or 0.
#[no_mangle]
pub unsafe extern "C" fn loos_vas_reserve(a: *mut AddressSpace, at: u64, size: u64, max_protect: u32, top_down: bool) -> u64 {
    // SAFETY: see crate contract.
    unsafe { a.as_mut().and_then(|a| a.reserve((at != 0).then_some(at), size, prot(max_protect), Backing::Private, top_down).ok()).unwrap_or(0) }
}
#[no_mangle]
pub unsafe extern "C" fn loos_vas_commit(a: *mut AddressSpace, addr: u64, size: u64, protect: u32) -> i32 {
    // SAFETY: see crate contract.
    unsafe { a.as_mut().map_or(-1, |a| a.commit(addr, size, prot(protect)).map_or(-1, |_| 0)) }
}
#[no_mangle]
pub unsafe extern "C" fn loos_vas_decommit(a: *mut AddressSpace, addr: u64, size: u64) -> i32 {
    // SAFETY: see crate contract.
    unsafe { a.as_mut().map_or(-1, |a| a.decommit(addr, size).map_or(-1, |_| 0)) }
}
#[no_mangle]
pub unsafe extern "C" fn loos_vas_release(a: *mut AddressSpace, base: u64) -> i32 {
    // SAFETY: see crate contract.
    unsafe { a.as_mut().map_or(-1, |a| a.release(base).map_or(-1, |_| 0)) }
}
#[no_mangle]
pub unsafe extern "C" fn loos_vas_check_access(a: *const AddressSpace, addr: u64, len: u64, write: bool, exec: bool) -> bool {
    // SAFETY: see crate contract.
    unsafe { a.as_ref().is_some_and(|a| a.check_access(addr, len, write, exec)) }
}
#[no_mangle]
pub unsafe extern "C" fn loos_vas_committed_bytes(a: *const AddressSpace) -> u64 {
    // SAFETY: see crate contract.
    unsafe { a.as_ref().map_or(0, |a| a.committed_bytes()) }
}

#[no_mangle]
pub extern "C" fn loos_ws_new(min_pages: usize, max_pages: usize) -> *mut WorkingSet {
    WorkingSet::new(min_pages, max_pages).map_or(std::ptr::null_mut(), |w| Box::into_raw(Box::new(w)))
}
#[no_mangle]
pub unsafe extern "C" fn loos_ws_free(w: *mut WorkingSet) {
    if !w.is_null() {
        // SAFETY: allocated by loos_ws_new.
        drop(unsafe { Box::from_raw(w) });
    }
}
/// Faults `page` in; writes pages evicted to make room into `evicted` (cap entries). Returns the number evicted.
#[no_mangle]
pub unsafe extern "C" fn loos_ws_fault_in(w: *mut WorkingSet, page: u64, evicted: *mut u64, cap: usize) -> usize {
    // SAFETY: see crate contract.
    unsafe {
        let Some(w) = w.as_mut() else { return 0 };
        let ev = w.fault_in(page);
        for (i, p) in ev.iter().enumerate().take(cap) {
            *evicted.add(i) = *p;
        }
        ev.len().min(cap)
    }
}
#[no_mangle]
pub unsafe extern "C" fn loos_ws_touch(w: *mut WorkingSet, page: u64) {
    // SAFETY: see crate contract.
    if let Some(w) = unsafe { w.as_mut() } {
        w.touch(page);
    }
}
#[no_mangle]
pub unsafe extern "C" fn loos_ws_size(w: *const WorkingSet) -> usize {
    // SAFETY: see crate contract.
    unsafe { w.as_ref().map_or(0, |w| w.size()) }
}

// ------------------------------------------------------------------ loos-registry
use loos_registry::{Hive, ValueType};

#[no_mangle]
pub extern "C" fn loos_hive_new() -> *mut Hive {
    Box::into_raw(Box::new(Hive::new()))
}
/// Parses a `regf` image; NULL on failure.
#[no_mangle]
pub unsafe extern "C" fn loos_hive_load(data: *const u8, len: usize) -> *mut Hive {
    // SAFETY: see crate contract.
    unsafe { Hive::load(bytes(data, len)).map_or(std::ptr::null_mut(), |h| Box::into_raw(Box::new(h))) }
}
#[no_mangle]
pub unsafe extern "C" fn loos_hive_free(h: *mut Hive) {
    if !h.is_null() {
        // SAFETY: allocated by loos_hive_new / loos_hive_load.
        drop(unsafe { Box::from_raw(h) });
    }
}
/// Serializes the hive; returns its size (may exceed `cap`, then nothing is written), or -1.
#[no_mangle]
pub unsafe extern "C" fn loos_hive_save(h: *const Hive, out: *mut u8, cap: usize) -> i64 {
    // SAFETY: see crate contract.
    unsafe {
        match h.as_ref().map(|h| h.save()) {
            Some(Ok(v)) => {
                if v.len() <= cap {
                    bytes_mut(out, cap)[..v.len()].copy_from_slice(&v);
                }
                v.len() as i64
            }
            _ => -1,
        }
    }
}
/// Sets a value (types: 1 SZ 2 EXPAND_SZ 3 BINARY 4 DWORD 7 MULTI_SZ 11 QWORD).
#[no_mangle]
pub unsafe extern "C" fn loos_hive_set(h: *mut Hive, path: *const c_char, name: *const c_char, kind: u32, data: *const u8, len: usize) -> i32 {
    let kind = match kind {
        1 => ValueType::Sz,
        2 => ValueType::ExpandSz,
        3 => ValueType::Binary,
        4 => ValueType::Dword,
        7 => ValueType::MultiSz,
        11 => ValueType::Qword,
        k => ValueType::Other(k),
    };
    // SAFETY: see crate contract.
    unsafe {
        let (Some(h), Some(p), Some(n)) = (h.as_mut(), cstr(path), cstr(name)) else { return -1 };
        h.set_value(p, n, kind, bytes(data, len)).map_or(-1, |_| 0)
    }
}
/// Reads a value; returns its data length (may exceed `cap`), or -1. `*kind` receives the type number.
#[no_mangle]
pub unsafe extern "C" fn loos_hive_get(h: *const Hive, path: *const c_char, name: *const c_char, kind: *mut u32, out: *mut u8, cap: usize) -> i64 {
    // SAFETY: see crate contract.
    unsafe {
        let (Some(h), Some(p), Some(n)) = (h.as_ref(), cstr(path), cstr(name)) else { return -1 };
        match h.value(p, n) {
            Ok(v) => {
                let k = match v.kind {
                    ValueType::Sz => 1,
                    ValueType::ExpandSz => 2,
                    ValueType::Binary => 3,
                    ValueType::Dword => 4,
                    ValueType::MultiSz => 7,
                    ValueType::Qword => 11,
                    ValueType::Other(k) => k,
                };
                if !kind.is_null() {
                    *kind = k;
                }
                if v.data.len() <= cap {
                    bytes_mut(out, cap)[..v.data.len()].copy_from_slice(&v.data);
                }
                v.data.len() as i64
            }
            Err(_) => -1,
        }
    }
}
#[no_mangle]
pub unsafe extern "C" fn loos_hive_delete_value(h: *mut Hive, path: *const c_char, name: *const c_char) -> i32 {
    // SAFETY: see crate contract.
    unsafe {
        let (Some(h), Some(p), Some(n)) = (h.as_mut(), cstr(path), cstr(name)) else { return -1 };
        h.delete_value(p, n).map_or(-1, |_| 0)
    }
}

// ------------------------------------------------------------------ loos-khal (pure descriptor builders)
/// Encodes a 16-byte IDT gate. `trap` selects a trap gate (interrupts stay enabled) instead of an interrupt gate.
#[no_mangle]
pub unsafe extern "C" fn loos_idt_gate(handler: u64, selector: u16, trap: bool, dpl: u8, ist: u8, out16: *mut u8) {
    use loos_khal::idt::{Gate, GateType};
    let g = Gate::new(handler, selector, if trap { GateType::Trap } else { GateType::Interrupt }, dpl, ist);
    // SAFETY: Gate is repr(C), 16 bytes; `out16` has 16 bytes.
    unsafe { std::ptr::copy_nonoverlapping(&g as *const Gate as *const u8, out16, std::mem::size_of::<Gate>()) }
}
/// Builds the 7-entry (56 byte) kernel GDT: null, kernel code/data, user data/code, 16-byte TSS descriptor.
#[no_mangle]
pub unsafe extern "C" fn loos_gdt_build(tss_base: u64, out56: *mut u8) {
    let g = loos_khal::gdt::Gdt::new(tss_base);
    // SAFETY: `out56` has 56 bytes.
    unsafe {
        for (i, e) in g.entries.iter().enumerate() {
            std::ptr::copy_nonoverlapping(e.to_le_bytes().as_ptr(), out56.add(i * 8), 8);
        }
    }
}

// ------------------------------------------------------------------ loos-drivers input
use loos_drivers::ps2::{KeyboardDecoder, MouseDecoder};

#[repr(C)]
#[derive(Default)]
pub struct KeyEv {
    key: u32,
    ch: u32,
    down: u8,
    mods: u8,
}
#[repr(C)]
#[derive(Default)]
pub struct MouseEv {
    dx: i32,
    dy: i32,
    wheel: i32,
    buttons: u8,
}

#[no_mangle]
pub extern "C" fn loos_kbd_new() -> *mut KeyboardDecoder {
    Box::into_raw(Box::new(KeyboardDecoder::new()))
}
#[no_mangle]
pub unsafe extern "C" fn loos_kbd_free(k: *mut KeyboardDecoder) {
    if !k.is_null() {
        // SAFETY: allocated by loos_kbd_new.
        drop(unsafe { Box::from_raw(k) });
    }
}
/// Feeds one scancode-set-1 byte; returns 1 and fills `ev` when a key event completes.
#[no_mangle]
pub unsafe extern "C" fn loos_kbd_feed(k: *mut KeyboardDecoder, byte: u8, ev: *mut KeyEv) -> i32 {
    // SAFETY: see crate contract.
    unsafe {
        match k.as_mut().and_then(|k| k.feed(byte)) {
            Some(e) => {
                *ev = KeyEv { key: e.key, ch: e.ch, down: e.down as u8, mods: e.mods };
                1
            }
            None => 0,
        }
    }
}
#[no_mangle]
pub extern "C" fn loos_mouse_new() -> *mut MouseDecoder {
    Box::into_raw(Box::new(MouseDecoder::new()))
}
#[no_mangle]
pub unsafe extern "C" fn loos_mouse_free(m: *mut MouseDecoder) {
    if !m.is_null() {
        // SAFETY: allocated by loos_mouse_new.
        drop(unsafe { Box::from_raw(m) });
    }
}
#[no_mangle]
pub unsafe extern "C" fn loos_mouse_feed(m: *mut MouseDecoder, byte: u8, ev: *mut MouseEv) -> i32 {
    // SAFETY: see crate contract.
    unsafe {
        match m.as_mut().and_then(|m| m.feed(byte)) {
            Some(e) => {
                *ev = MouseEv { dx: e.dx, dy: e.dy, wheel: e.wheel, buttons: e.buttons };
                1
            }
            None => 0,
        }
    }
}
