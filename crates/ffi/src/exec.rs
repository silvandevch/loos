//! Ps scheduling, timers/APC/DPC (ksync), SEH dispatch and the read-only loos-btrfs reader.

use crate::{bytes, bytes_mut};
use loos_kernel::ObjectManager;
use loos_ksync::apc::{Apc, ApcQueues, Mode, QueueResult};
use loos_ksync::dpc::{Dpc, DpcQueue, Importance, WorkClass, WorkQueue};
use loos_ksync::{Dispatcher, EventKind, ObjId};
use std::ffi::c_void;

fn boxed<T>(v: T) -> *mut T {
    Box::into_raw(Box::new(v))
}
unsafe fn unbox<T>(p: *mut T) {
    if !p.is_null() {
        // SAFETY: p came from `boxed`.
        drop(unsafe { Box::from_raw(p) });
    }
}

// ---- Ps (via the object manager)

#[no_mangle]
pub extern "C" fn loos_kobj_new_cpus(cpus: u32) -> *mut ObjectManager {
    boxed(ObjectManager::with_cpus(cpus))
}

/// Matches `loos_sched_query_fn` (src/kernel/core.h); `opaque` is the loos_kobj.
#[no_mangle]
pub unsafe extern "C" fn loos_kobj_sched_query(opaque: *mut c_void, thread: u64, priority: *mut u32, runnable: *mut bool) -> bool {
    // SAFETY: opaque is a loos_kobj; outputs writable.
    unsafe {
        let Some(m) = (opaque as *const ObjectManager).as_ref() else { return false };
        match (m.ps.base_priority(thread), m.ps.runnable(thread)) {
            (Ok(p), Ok(r)) if !priority.is_null() && !runnable.is_null() => {
                *priority = p as u32;
                *runnable = r;
                true
            }
            _ => false,
        }
    }
}

// ---- ksync timers and alerts

#[no_mangle]
pub unsafe extern "C" fn loos_disp_create_timer(d: *mut Dispatcher, synchronization: bool) -> u64 {
    // SAFETY: see crate contract.
    unsafe { d.as_mut() }.map_or(0, |d| d.create_timer(if synchronization { EventKind::Synchronization } else { EventKind::Notification }).to_raw())
}
/// 1 was armed, 0 was not, -1 error.
#[no_mangle]
pub unsafe extern "C" fn loos_disp_set_timer(d: *mut Dispatcher, id: u64, delay: u64, period: u64) -> i32 {
    // SAFETY: see crate contract.
    match (unsafe { d.as_mut() }, ObjId::from_raw(id)) {
        (Some(d), Some(o)) => d.set_timer(o, delay, period).map_or(-1, i32::from),
        _ => -1,
    }
}
#[no_mangle]
pub unsafe extern "C" fn loos_disp_cancel_timer(d: *mut Dispatcher, id: u64) -> i32 {
    // SAFETY: see crate contract.
    match (unsafe { d.as_mut() }, ObjId::from_raw(id)) {
        (Some(d), Some(o)) => d.cancel_timer(o).map_or(-1, i32::from),
        _ => -1,
    }
}
/// Alertable variant of loos_disp_wait (same return codes; 5 = alerted).
#[no_mangle]
pub unsafe extern "C" fn loos_disp_wait_alertable(d: *mut Dispatcher, thread: u64, ids: *const u64, n: usize, wait_all: bool, timeout_ticks: i64) -> i32 {
    // SAFETY: ids has n entries.
    unsafe {
        let Some(d) = d.as_mut() else { return -1 };
        let mut objs = Vec::new();
        for &r in std::slice::from_raw_parts(ids, n) {
            let Some(o) = ObjId::from_raw(r) else { return -1 };
            objs.push(o);
        }
        let kind = if wait_all { loos_ksync::WaitKind::All } else { loos_ksync::WaitKind::Any };
        match d.wait_ex(thread, &objs, kind, (timeout_ticks >= 0).then_some(timeout_ticks as u64), true) {
            Ok(loos_ksync::WaitStart::Blocked) => 4,
            Ok(loos_ksync::WaitStart::Done(_)) => 0,
            Err(_) => -1,
        }
    }
}
#[no_mangle]
pub unsafe extern "C" fn loos_disp_alert(d: *mut Dispatcher, thread: u64) -> bool {
    // SAFETY: see crate contract.
    unsafe { d.as_mut() }.is_some_and(|d| d.alert(thread))
}

// ---- APC

#[no_mangle]
pub extern "C" fn loos_apc_new() -> *mut ApcQueues {
    boxed(ApcQueues::new())
}
#[no_mangle]
pub unsafe extern "C" fn loos_apc_free(q: *mut ApcQueues) {
    unsafe { unbox(q) }
}
/// 0 queued, 1 queued + must alert the thread, -1 rejected.
#[no_mangle]
pub unsafe extern "C" fn loos_apc_queue(q: *mut ApcQueues, thread: u64, user: bool, routine: u64, a0: u64, a1: u64, a2: u64) -> i32 {
    // SAFETY: see crate contract.
    let Some(q) = (unsafe { q.as_mut() }) else { return -1 };
    match q.queue(thread, if user { Mode::User } else { Mode::Kernel }, Apc { routine, args: [a0, a1, a2] }) {
        QueueResult::Queued => 0,
        QueueResult::Alert => 1,
        QueueResult::Rejected => -1,
    }
}
/// Pops one deliverable APC (kernel first, then user when `user_ok`) into routine/args4[0..3]. Returns 1 if one was taken.
#[no_mangle]
pub unsafe extern "C" fn loos_apc_next(q: *mut ApcQueues, thread: u64, user_ok: bool, out4: *mut u64) -> i32 {
    // SAFETY: out4 has 4 entries.
    unsafe {
        let Some(q) = q.as_mut() else { return -1 };
        let a = q.next_kernel(thread).or_else(|| if user_ok { q.drain_user(thread).pop_front() } else { None });
        match a {
            Some(a) if !out4.is_null() => {
                *out4 = a.routine;
                std::ptr::copy_nonoverlapping(a.args.as_ptr(), out4.add(1), 3);
                1
            }
            _ => 0,
        }
    }
}
/// True when user APCs are pending (the alertable wait must not block).
#[no_mangle]
pub unsafe extern "C" fn loos_apc_begin_alertable(q: *mut ApcQueues, thread: u64) -> bool {
    // SAFETY: see crate contract.
    unsafe { q.as_mut() }.is_some_and(|q| q.begin_alertable_wait(thread))
}

// ---- DPC / work queue

#[no_mangle]
pub extern "C" fn loos_dpc_new() -> *mut DpcQueue {
    boxed(DpcQueue::new())
}
#[no_mangle]
pub unsafe extern "C" fn loos_dpc_free(q: *mut DpcQueue) {
    unsafe { unbox(q) }
}
/// importance 0 low 1 medium 2 high. Returns false if already queued.
#[no_mangle]
pub unsafe extern "C" fn loos_dpc_insert(q: *mut DpcQueue, id: u64, routine: u64, a0: u64, a1: u64, importance: u32) -> bool {
    let imp = match importance { 2 => Importance::High, 1 => Importance::Medium, _ => Importance::Low };
    // SAFETY: see crate contract.
    unsafe { q.as_mut() }.is_some_and(|q| q.insert(Dpc { id, routine, args: [a0, a1] }, imp))
}
/// Drains up to `cap` DPCs as (id, routine, a0, a1) quadruples into out. Returns the count.
#[no_mangle]
pub unsafe extern "C" fn loos_dpc_drain(q: *mut DpcQueue, out: *mut u64, cap: usize) -> usize {
    // SAFETY: out has 4*cap entries.
    unsafe {
        let Some(q) = q.as_mut() else { return 0 };
        let v = q.drain();
        let n = v.len().min(cap);
        for (i, d) in v.iter().enumerate().take(cap) {
            let o = out.add(i * 4);
            *o = d.id;
            *o.add(1) = d.routine;
            *o.add(2) = d.args[0];
            *o.add(3) = d.args[1];
        }
        for d in v.into_iter().skip(cap) {
            q.insert(d, Importance::Medium); // re-queue what did not fit
        }
        n
    }
}

#[no_mangle]
pub extern "C" fn loos_workq_new() -> *mut WorkQueue<u64> {
    boxed(WorkQueue::new())
}
#[no_mangle]
pub unsafe extern "C" fn loos_workq_free(q: *mut WorkQueue<u64>) {
    unsafe { unbox(q) }
}
/// class 0 delayed 1 critical 2 hypercritical.
#[no_mangle]
pub unsafe extern "C" fn loos_workq_push(q: *mut WorkQueue<u64>, class: u32, item: u64) {
    let c = match class { 2 => WorkClass::HyperCritical, 1 => WorkClass::Critical, _ => WorkClass::Delayed };
    // SAFETY: see crate contract.
    if let Some(q) = unsafe { q.as_mut() } {
        q.push(c, item);
    }
}
#[no_mangle]
pub unsafe extern "C" fn loos_workq_pop(q: *mut WorkQueue<u64>, item: *mut u64) -> bool {
    // SAFETY: item writable.
    unsafe {
        match q.as_mut().and_then(WorkQueue::pop) {
            Some((_, v)) if !item.is_null() => {
                *item = v;
                true
            }
            _ => false,
        }
    }
}

// ---- SEH

use loos_kernel::seh::{self, Context, Disposition, HandlerCall, Image, Memory, Outcome, Vectored};

/// Reads `len` bytes at `addr` into `out`; returns false if unmapped.
pub type ReadFn = unsafe extern "C" fn(opaque: *mut c_void, addr: u64, out: *mut u8, len: usize) -> bool;
/// Runs a handler. kind: 0 vectored, 1 frame, 2 continue, 3 unhandled filter. `ctx18` = rip, gpr[16], pad;
/// for frame handlers data/establisher are set. Returns true for ContinueExecution.
pub type HandlerFn = unsafe extern "C" fn(opaque: *mut c_void, kind: u32, routine: u64, data: u64, establisher: u64, ctx17: *mut u64) -> bool;

struct CMem(ReadFn, *mut c_void);
impl Memory for CMem {
    fn read(&self, addr: u64, buf: &mut [u8]) -> bool {
        // SAFETY: C callback contract.
        unsafe { (self.0)(self.1, addr, buf.as_mut_ptr(), buf.len()) }
    }
}

#[repr(C)]
pub struct SehImage {
    base: u64,
    size: u64,
    pdata_rva: u32,
    pdata_len: u32,
}

/// Exception dispatch over `n` images. `ctx17` (rip + 16 GPRs) is updated with the resume context.
/// veh/vch: arrays of handler addresses in call order. Returns 0 continue, 1 unhandled, 2 bad stack, -1 args.
#[no_mangle]
pub unsafe extern "C" fn loos_seh_dispatch(
    ctx17: *mut u64, images: *const SehImage, n: usize, veh: *const u64, nveh: usize, vch: *const u64, nvch: usize,
    unhandled: u64, stack_lo: u64, stack_hi: u64, read: ReadFn, handler: HandlerFn, opaque: *mut c_void,
) -> i32 {
    if ctx17.is_null() {
        return -1;
    }
    // SAFETY: arrays have the stated lengths; callbacks follow their contracts.
    unsafe {
        let mem = CMem(read, opaque);
        let mut imgs = Vec::new();
        for i in 0..n {
            let d = &*images.add(i);
            let Some(img) = Image::load(d.base, d.size, &mem, d.pdata_rva, d.pdata_len) else { return -1 };
            imgs.push(img);
        }
        let mut v = Vectored::new();
        for i in 0..nveh {
            v.add_exception(false, *veh.add(i));
        }
        for i in 0..nvch {
            v.add_continue(false, *vch.add(i));
        }
        v.unhandled_filter = (unhandled != 0).then_some(unhandled);
        let raw = std::slice::from_raw_parts_mut(ctx17, 17);
        let mut ctx = Context { rip: raw[0], gpr: raw[1..].try_into().unwrap_or([0; 16]) };
        let out = seh::dispatch(&mut ctx, &imgs, &mem, &v, (stack_lo, stack_hi), &mut |call, c| {
            let mut buf = [0u64; 17];
            buf[0] = c.rip;
            buf[1..].copy_from_slice(&c.gpr);
            let (k, r, d, e) = match call {
                HandlerCall::Vectored(r) => (0, r, 0, 0),
                HandlerCall::Frame(h) => (1, h.routine, h.data, h.establisher),
                HandlerCall::Continue(r) => (2, r, 0, 0),
                HandlerCall::Unhandled(r) => (3, r, 0, 0),
            };
            let go = handler(opaque, k, r, d, e, buf.as_mut_ptr());
            c.rip = buf[0];
            c.gpr.copy_from_slice(&buf[1..]);
            if go { Disposition::ContinueExecution } else { Disposition::ContinueSearch }
        });
        raw[0] = ctx.rip;
        raw[1..].copy_from_slice(&ctx.gpr);
        match out {
            Outcome::Continue => 0,
            Outcome::Unhandled => 1,
            Outcome::BadStack => 2,
        }
    }
}

// ---- loos-btrfs (read-only, validating reader over an in-memory image)

use loos_btrfs::{BtrfsReader, ReadAt, ReadError};

pub struct OwnedImage(Vec<u8>);
impl ReadAt for OwnedImage {
    fn read_at(&self, offset: u64, out: &mut [u8]) -> Result<(), ReadError> {
        let s = usize::try_from(offset).ok().and_then(|o| self.0.get(o..o.checked_add(out.len())?));
        let s = s.ok_or(ReadError::OutOfBounds { offset, length: out.len() })?;
        out.copy_from_slice(s);
        Ok(())
    }
    fn len(&self) -> Option<u64> {
        Some(self.0.len() as u64)
    }
}

/// Copies the image and opens it; null if it is not a valid btrfs filesystem.
#[no_mangle]
pub unsafe extern "C" fn loos_btrfsr_open(data: *const u8, len: usize) -> *mut BtrfsReader<OwnedImage> {
    // SAFETY: data valid for len bytes.
    let img = OwnedImage(unsafe { bytes(data, len) }.to_vec());
    BtrfsReader::open(img).map_or(std::ptr::null_mut(), boxed)
}
#[no_mangle]
pub unsafe extern "C" fn loos_btrfsr_close(r: *mut BtrfsReader<OwnedImage>) {
    unsafe { unbox(r) }
}
/// stat of an absolute path: out4 = inode, size, mode, nlink. 0 ok, -1 error.
#[no_mangle]
pub unsafe extern "C" fn loos_btrfsr_stat(r: *const BtrfsReader<OwnedImage>, path: *const u8, plen: usize, out4: *mut u64) -> i32 {
    // SAFETY: see crate contract.
    unsafe {
        match r.as_ref().map(|r| r.stat_path(bytes(path, plen))) {
            Some(Ok(s)) if !out4.is_null() => {
                *out4 = s.inode;
                *out4.add(1) = s.size;
                *out4.add(2) = u64::from(s.mode);
                *out4.add(3) = u64::from(s.nlink);
                0
            }
            _ => -1,
        }
    }
}
/// Reads up to cap bytes at offset; returns bytes read or -1.
#[no_mangle]
pub unsafe extern "C" fn loos_btrfsr_read(r: *const BtrfsReader<OwnedImage>, path: *const u8, plen: usize, offset: u64, out: *mut u8, cap: usize) -> i64 {
    // SAFETY: see crate contract.
    unsafe {
        match r.as_ref().map(|r| r.read_file(bytes(path, plen), offset, cap)) {
            Some(Ok(v)) => {
                bytes_mut(out, cap)[..v.len()].copy_from_slice(&v);
                v.len() as i64
            }
            _ => -1,
        }
    }
}

// ---- Windows <-> LoOS paths (loos-vfs::winpath)

/// UTF-8 Windows/NT path to LoOS path. Returns the length (may exceed cap) or -1 if unmappable.
#[no_mangle]
pub unsafe extern "C" fn loos_winpath_to_loos(win: *const std::ffi::c_char, out: *mut u8, cap: usize) -> i64 {
    // SAFETY: NUL-terminated input, out has cap bytes.
    unsafe {
        match crate::cstr(win).and_then(loos_vfs::winpath::to_loos) {
            Some(p) => crate::put_str(&p, out, cap) as i64,
            None => -1,
        }
    }
}
/// LoOS path to the Windows view of it. Returns the length (may exceed cap) or -1.
#[no_mangle]
pub unsafe extern "C" fn loos_winpath_to_windows(path: *const std::ffi::c_char, out: *mut u8, cap: usize) -> i64 {
    // SAFETY: as above.
    unsafe { crate::cstr(path).map_or(-1, |p| crate::put_str(&loos_vfs::winpath::to_windows(p), out, cap) as i64) }
}
