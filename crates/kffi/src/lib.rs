//! Freestanding C ABI linked into the stage-0 kernel (`boot/build-kernel.sh`).
//! Allocation goes to the kernel's `kalloc` (a bump allocator: frees are dropped).
// Host builds (cargo test --workspace) use std; only the kernel target supplies alloc/panic.
#![cfg_attr(target_os = "none", no_std)]
#![allow(clippy::missing_safety_doc)]

extern crate alloc;

#[cfg(target_os = "none")]
use core::alloc::{GlobalAlloc, Layout};
use core::cell::UnsafeCell;
use loos_kdump::format::{Context, DumpWriter, Header, SliceSink};
pub use loos_shell; // shell drawing core for the boot desktop
use loos_kernel::{dispatch, ObjectManager, SyscallArgs};

#[cfg(target_os = "none")]
extern "C" {
    fn kalloc(size: u64, align: u64) -> *mut u8;
    fn serial_puts(s: *const u8);
}

#[cfg(target_os = "none")]
struct KAlloc;
#[cfg(target_os = "none")]
unsafe impl GlobalAlloc for KAlloc {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        // SAFETY: kalloc returns aligned memory or null.
        unsafe { kalloc(l.size() as u64, l.align().max(8) as u64) }
    }
    unsafe fn dealloc(&self, _: *mut u8, _: Layout) {}
}
#[cfg(target_os = "none")]
#[global_allocator]
static A: KAlloc = KAlloc;

#[cfg(target_os = "none")]
#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    // SAFETY: NUL-terminated literal.
    unsafe { serial_puts(b"LoOS kernel: Rust panic\n\0".as_ptr()) };
    loop {
        core::hint::spin_loop();
    }
}

/// Single-CPU boot executive; callers serialize (stage 0 runs on the BSP).
struct Exec(UnsafeCell<Option<ObjectManager>>);
// SAFETY: only touched from the BSP before the scheduler exists.
unsafe impl Sync for Exec {}
static EXEC: Exec = Exec(UnsafeCell::new(None));

/// Creates the executive and the System process with its initial thread.
/// Returns the System pid, or 0 on failure.
#[no_mangle]
pub unsafe extern "C" fn loos_kexec_init(cpus: u32) -> u64 {
    // SAFETY: see `Exec`.
    let slot = unsafe { &mut *EXEC.0.get() };
    let m = slot.insert(ObjectManager::with_cpus(cpus));
    let Ok((h, pid)) = m.create_process(None) else { return 0 };
    if m.create_thread(h).is_err() {
        return 0;
    }
    pid
}

/// One executive syscall (see loos_kernel::SyscallNumber). Returns the status code.
#[no_mangle]
pub unsafe extern "C" fn loos_kexec_syscall(number: u32, args6: *const u64, value: *mut u64) -> i32 {
    // SAFETY: see `Exec`; args6 has six entries, value writable.
    unsafe {
        let Some(m) = (*EXEC.0.get()).as_mut() else { return -1 };
        let mut a = [0u64; 6];
        if !args6.is_null() {
            core::ptr::copy_nonoverlapping(args6, a.as_mut_ptr(), 6);
        }
        let r = dispatch(m, number, SyscallArgs(a));
        if !value.is_null() {
            *value = r.value;
        }
        r.status as i32
    }
}

/// Register frame passed by the trap path (matches `struct ktrap_frame` in trap.h).
#[repr(C)]
pub struct TrapFrame {
    /// RAX..R15 in x64 encoding order.
    pub gpr: [u64; 16],
    /// Vector and error code.
    pub vector: u64,
    /// Error code (0 when the CPU pushes none).
    pub error: u64,
    /// Interrupt return frame.
    pub rip: u64,
    /// Code segment.
    pub cs: u64,
    /// Flags.
    pub rflags: u64,
}

/// Writes a crash dump without allocating. Returns its size or -1.
#[no_mangle]
pub unsafe extern "C" fn loos_kbugcheck_dump(
    out: *mut u8, cap: usize, code: u32, params4: *const u64, f: *const TrapFrame, cr2: u64, frames: *const u64, nframes: usize,
) -> i64 {
    if out.is_null() || params4.is_null() || f.is_null() {
        return -1;
    }
    // SAFETY: caller guarantees buffer/array lengths.
    unsafe {
        let p = core::slice::from_raw_parts(params4, 4);
        let f = &*f;
        let h = Header { bugcheck: loos_kdump::BugCheck::new(code, [p[0], p[1], p[2], p[3]]), time_ns: 0, cpu: 0, ncpus: 1, build_id: 0 };
        let Ok(mut w) = DumpWriter::begin(SliceSink::new(core::slice::from_raw_parts_mut(out, cap)), &h) else { return -1 };
        let ctx = Context { gpr: f.gpr, rip: f.rip, rflags: f.rflags, cr: [0, 0, cr2, 0], cs: f.cs as u16, ss: 0x10 };
        let ok = w.context(&ctx).is_ok() && (nframes == 0 || w.stack(core::slice::from_raw_parts(frames, nframes)).is_ok());
        match (ok, w.finish()) {
            (true, Ok((_, n))) => n as i64,
            _ => -1,
        }
    }
}

/// Stop-code name as a NUL-terminated string written into out (cap bytes). Returns its length.
#[no_mangle]
pub unsafe extern "C" fn loos_kbugcheck_name(code: u32, out: *mut u8, cap: usize) -> usize {
    let s = loos_kdump::bugcheck::name(code).as_bytes();
    if !out.is_null() && cap > 0 {
        let n = s.len().min(cap - 1);
        // SAFETY: out has cap bytes.
        unsafe {
            core::ptr::copy_nonoverlapping(s.as_ptr(), out, n);
            *out.add(n) = 0;
        }
    }
    s.len()
}
