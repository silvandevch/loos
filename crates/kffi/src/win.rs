//! Kernel side of the Windows subsystem: runs PE processes from `loos-winrt` in ring 3 and hosts the session
//! manager (`loos_kernel::smss`) on top of them as the real process launcher.
//!
//! Cooperative: a process runs until it exits, parks itself (`Sleep(INFINITE)`, a resident service), or
//! faults. Resident processes keep their address space but are not scheduled again until threads/timers exist.

use crate::lin::{build_pml4, map_page, read_cr2, run_user, say, time_us, user_init, wrmsr, UFrame, MSR_FS_BASE};
use alloc::collections::BTreeMap;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use loos_kernel::smss::{Launcher, Outcome, SmConfig, Smss};
use loos_kernel::ProcessId;
use loos_lin::fs::MemFs;
use loos_win32k::Win32k;
use loos_security::sam::{Lsa, Sam};
use loos_security::uac::SID_ADMINS;
use loos_winrt::{Process, Ret};

extern "C" {
    fn serial_dec(v: u64);
}

enum End {
    Exit(u32),
    Park,
}

extern "C" {
    /// Kernel entropy (RDRAND when present, else timer jitter): `src/kernel/boot/ktime.c`.
    fn krandom(out: *mut u8, n: usize);
}

fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    // SAFETY: krandom fills exactly `n` bytes of the buffer it is given.
    unsafe { krandom(b.as_mut_ptr(), N) };
    b
}

/// PBKDF2 cost of the boot-time account database (the logon UI must be built with the same value).
pub(crate) const BOOT_ITERATIONS: u32 = 4096;

/// The machine's local accounts for this boot: a fresh machine SID and LSA boot key from kernel entropy, the disabled
/// built-in Administrator/Guest, and one administrator "User" with a blank password (there is no remote logon; a real
/// password is set through the SAM). Returns `(SAM, LSA, RID of the user processes run as)`.
pub(crate) fn boot_accounts() -> (Sam, Lsa, u32) {
    let sid = random::<12>();
    let part = |i: usize| u32::from_le_bytes(sid[4 * i..4 * i + 4].try_into().unwrap());
    let mut sam = Sam::new("LOOS", &alloc::format!("S-1-5-21-{}-{}-{}", part(0), part(1), part(2)), BOOT_ITERATIONS);
    sam.policy.min_len = 0;
    let rid = sam.create_user("User", b"", random::<16>(), 0).unwrap_or(1000);
    sam.policy.min_len = 8;
    if let Some(sid) = sam.user(rid).map(|u| u.account.sid.clone()) {
        let _ = sam.add_member(SID_ADMINS, &sid);
    }
    (sam, Lsa::new(&random::<32>()), rid)
}

struct WinKernel {
    accounts: (Sam, Lsa, u32),
    fs: MemFs,
    next_pid: u64,
    resident: BTreeMap<u64, Process>,
    /// Processes that ended on their own: reported to smss after the launch returns.
    exited: Vec<(u64, u32)>,
}

fn flush(p: &mut Process) {
    let out = p.take_console();
    if !out.is_empty() {
        say("win: ");
        say(&String::from_utf8_lossy(&out));
    }
}

/// Why [`drive`] returned.
enum Stop {
    Exit(u32),
    Park,
    /// The GUI thread waits for input (`GetMessage` retries when re-entered).
    Idle,
    /// A window-procedure callback returned this value.
    CbRet(u64),
}

/// The process and frame currently executing, for window-procedure upcalls (single CPU, cooperative).
static mut CUR: (*mut Process, *mut UFrame) = (core::ptr::null_mut(), core::ptr::null_mut());
/// Set when a callback called `ExitProcess`: the outer loop ends the process.
static mut PENDING_EXIT: Option<u32> = None;

/// Threads other than the executing one: (thread index, saved frame). Single CPU, cooperative.
static mut SCHED: Vec<(usize, UFrame)> = Vec::new();
/// Nesting depth of guest callbacks (threads only switch at depth 0).
static mut DEPTH: u32 = 0;
/// Consecutive blocked results without any thread making progress, whether one was an idle GUI wait, syscall tick.
static mut SPIN: u32 = 0;
static mut ANY_IDLE: bool = false;
static mut TICKS: u32 = 0;

fn new_frame(p: &Process) -> UFrame {
    let mut frame = UFrame { gpr: [0; 16], vector: 0, error: 0, rip: p.entry, cs: 0x23, rflags: 2, rsp: p.rsp, ss: 0x1b, cr3: 0, _pad: 0, fx: [0; 512] };
    // The FXSAVE area (offset 192 of the frame) starts in `_pad`: FCW is its low half, MXCSR sits at FXSAVE byte 24 = fx[16].
    frame._pad = 0x037f;
    frame.fx[16..20].copy_from_slice(&0x1f80u32.to_le_bytes());
    frame
}

/// Runs `frame` in ring 3, serving syscalls. With a window manager `w`, user32/gdi32 calls are served too.
fn drive(p: &mut Process, frame: &mut UFrame, mut w: Option<&mut Win32k>) -> Stop {
    let (mut ver, mut cr3) = (u64::MAX, 0u64);
    loop {
        if p.mem.version != ver {
            ver = p.mem.version;
            cr3 = build_pml4(&p.mem);
            if cr3 == 0 {
                say("win: out of memory building page tables\n");
                return Stop::Exit(0xc000_0017);
            }
            p.mem.take_remap();
        }
        for va in p.mem.take_remap() {
            let (pa, prot) = p.mem.pte(va).unwrap_or((0, 0));
            if !map_page(cr3 as *mut u64, va, pa, prot) {
                return Stop::Exit(0xc000_0017);
            }
        }
        frame.cr3 = cr3;
        // SAFETY: FS base MSR; run_user enters ring 3 with a fully initialised frame and returns on trap.
        let vector = unsafe {
            wrmsr(MSR_FS_BASE, 0);
            wrmsr(0xC000_0102, p.threads[p.cur].teb); // IA32_KERNEL_GS_BASE: becomes the user GS base on the swapgs into ring 3
            run_user(frame)
        };
        if vector == 0x100 {
            let g = frame.gpr;
            let nr = g[0] as u32;
            if nr == loos_winrt::gui::CB_RET {
                return Stop::CbRet(g[10]);
            }
            // SAFETY: monotonic microsecond clock.
            let now_ms = unsafe { time_us() } / 1000;
            let args = [g[10], g[2], g[8], g[9]];
            p.cur_rsp = frame.rsp;
            p.net_poll(now_ms);
            p.regs = g;
            for k in 0..4 {
                p.xmm[k].copy_from_slice(&frame.fx[152 + 16 * k..168 + 16 * k]);
            }
            p.ret_addr = p.mem.read_u64(frame.rsp).unwrap_or(0);
            let r = match w.as_deref_mut() {
                Some(win) if (loos_winrt::gui::GUI_BASE..0x1000).contains(&nr) || nr >= loos_winrt::k32::K32_BASE => {
                    // SAFETY: published for the upcall hook, which runs inside this call on the same CPU.
                    unsafe { CUR = (p as *mut Process, frame as *mut UFrame) };
                    p.gui_syscall(nr, args, frame.rsp, now_ms, win)
                }
                _ => p.syscall(nr, args, now_ms),
            };
            flush(p);
            // SAFETY: single CPU; written only by the upcall hook inside the call above.
            if let Some(c) = unsafe { PENDING_EXIT.take() } {
                return Stop::Exit(c);
            }
            // New threads queue their frames; they start when the running thread blocks or its quantum ends.
            while let Some(i) = p.take_spawn() {
                let t = &p.threads[i];
                let mut f = new_frame(p);
                (f.rip, f.rsp, f.gpr[1]) = (t.entry, t.rsp, t.param);
                // SAFETY: single CPU.
                unsafe { (*core::ptr::addr_of_mut!(SCHED)).push((i, f)) };
            }
            match r {
                Ret::Value(v) => {
                    frame.gpr[0] = v;
                    // SAFETY: single CPU statics.
                    unsafe {
                        SPIN = 0;
                        TICKS = TICKS.wrapping_add(1);
                        if TICKS % 32 == 0 && DEPTH == 0 {
                            rotate(p, frame);
                        }
                    }
                }
                Ret::Exit(c) => return Stop::Exit(c),
                Ret::Park => return Stop::Park,
                Ret::Init(h) => {
                    // DllMain(hinst, DLL_PROCESS_ATTACH, NULL) for each module the call queued, dependencies first.
                    while let Some((entry, base)) = p.next_init() {
                        let _ = call_guest(p, frame, w.as_deref_mut(), entry, [base, 1, 0, 0]);
                    }
                    frame.gpr[0] = h;
                }
                Ret::Yield | Ret::Idle => {
                    // Retry the blocking call when this thread runs again: re-execute the 2-byte `syscall`.
                    frame.rip -= 2;
                    frame.gpr[0] = u64::from(nr);
                    if let Some(stop) = block(p, frame, matches!(r, Ret::Idle)) {
                        return stop;
                    }
                }
                Ret::Raise { code, flags, nargs, args } => {
                    let mark = p.scratch_mark();
                    let ctx = p.capture_ctx();
                    let argv: Vec<u64> = (0..u64::from(nargs.min(15))).map(|i| p.mem.read_u64(args + i * 8).unwrap_or(0)).collect();
                    let rec = make_record(p, code, flags, ctx.rip, &argv);
                    let ctxa = make_ctx(p, &ctx);
                    let ok = dispatch(p, frame, w.as_deref_mut(), rec, ctxa, false) || unhandled(p, frame, w.as_deref_mut(), rec, ctxa);
                    p.scratch_release(mark);
                    if !ok {
                        return Stop::Exit(code);
                    }
                }
                Ret::Unwind { frame: target, ip, rec, ret } => {
                    let mark = p.scratch_mark();
                    let rec = if rec == 0 { make_record(p, 0xC000_0027, 0, ip, &[]) } else { rec };
                    let start = p.capture_ctx();
                    let ok = unwind_to(p, frame, w.as_deref_mut(), target, ip, rec, ret, start);
                    p.scratch_release(mark);
                    if !ok {
                        say("win: RtlUnwindEx: target frame not found\n");
                        return Stop::Exit(0xC000_0027);
                    }
                }
                Ret::ThreadExit(code) => {
                    // SAFETY: single CPU.
                    let next = unsafe { (*core::ptr::addr_of_mut!(SCHED)).pop() };
                    match next {
                        Some((i, f)) => {
                            *frame = f;
                            p.cur = i;
                        }
                        None => return Stop::Exit(code),
                    }
                }
            }
            continue;
        }
        let cr2 = read_cr2();
        if vector == 14 && p.mem.handle_fault(cr2, frame.error & 2 != 0).is_some_and(|(va, pa, prot)| map_page(cr3 as *mut u64, va, pa, prot)) {
            continue;
        }
        if raise_fault(p, frame, w.as_deref_mut(), vector, cr2) {
            continue;
        }
        say("win: fault vector ");
        // SAFETY: serial output.
        unsafe { serial_dec(vector) };
        say(" rip ");
        unsafe { serial_dec(frame.rip) };
        say(" cr2 ");
        unsafe { serial_dec(cr2) };
        say(" gpr:");
        for r in frame.gpr {
            say(" ");
            // SAFETY: serial output.
            unsafe { serial_dec(r) };
        }
        say(" stack:");
        for i in 0..12u64 {
            say(" ");
            // SAFETY: serial output.
            unsafe { serial_dec(p.mem.read_u64(frame.rsp + i * 8).unwrap_or(0)) };
        }
        say("\n");
        flush(p);
        return Stop::Exit(0xc000_0005);
    }
}

// ---- structured exception handling ------------------------------------------------------------------------------

use loos_winrt::seh::{Ctx, CONTEXT_SIZE, EXC_TARGET_UNWIND, EXC_UNWINDING, UNW_EHANDLER, UNW_UHANDLER};

fn w32(p: &mut Process, a: u64, v: u32) {
    let _ = p.mem.write(a, &v.to_le_bytes());
}
fn w64(p: &mut Process, a: u64, v: u64) {
    let _ = p.mem.write(a, &v.to_le_bytes());
}

fn apply_ctx(frame: &mut UFrame, c: &Ctx) {
    frame.gpr = c.r;
    frame.rsp = c.r[4];
    frame.rip = c.rip;
}

fn make_ctx(p: &mut Process, c: &Ctx) -> u64 {
    let a = p.scratch_alloc(CONTEXT_SIZE as u64);
    p.write_ctx(a, c);
    w32(p, a + 0x30, 0x10_000B);
    w32(p, a + 0x44, 0x202);
    a
}

/// EXCEPTION_RECORD in scratch memory.
fn make_record(p: &mut Process, code: u32, flags: u32, addr: u64, args: &[u64]) -> u64 {
    let a = p.scratch_alloc(0x98);
    w32(p, a, code);
    w32(p, a + 4, flags);
    w64(p, a + 0x10, addr);
    w32(p, a + 0x18, args.len() as u32);
    for (i, v) in args.iter().enumerate() {
        w64(p, a + 0x20 + 8 * i as u64, *v);
    }
    a
}

fn dispatcher_context(p: &mut Process, pc: u64, base: u64, func: u64, frame_addr: u64, target_ip: u64, ctxa: u64, handler: u64, data: u64) -> u64 {
    let dc = p.scratch_alloc(0x50);
    for (o, v) in [(0, pc), (8, base), (0x10, func), (0x18, frame_addr), (0x20, target_ip), (0x28, ctxa), (0x30, handler), (0x38, data)] {
        w64(p, dc + o, v);
    }
    dc
}

/// Calls a language handler: the native `__C_specific_handler` or a guest routine. Returns its disposition
/// (100: the handler already transferred control by unwinding).
fn run_handler(p: &mut Process, frame: &mut UFrame, mut w: Option<&mut Win32k>, h: u64, rec: u64, est: u64, ctxa: u64, dc: u64) -> i32 {
    if p.stub_nr(h).is_some_and(|n| Some(n) == loos_winrt::api::lookup("__C_specific_handler")) {
        return c_specific(p, frame, w.as_deref_mut(), rec, est, ctxa, dc);
    }
    match call_guest(p, frame, w, h, [rec, est, ctxa, dc]) {
        Stop::CbRet(v) => v as i32,
        _ => 1,
    }
}

/// `__C_specific_handler`: walks the scope table of the function (`HandlerData`) for the exception or unwind.
fn c_specific(p: &mut Process, frame: &mut UFrame, mut w: Option<&mut Win32k>, rec: u64, est: u64, ctxa: u64, dc: u64) -> i32 {
    let rd = |p: &mut Process, a: u64| p.mem.read_u64(a).unwrap_or(0);
    let flags = p.mem.read_u32(rec + 4).unwrap_or(0);
    let (cp, base, target_ip, data) = (rd(p, dc), rd(p, dc + 8), rd(p, dc + 0x20), rd(p, dc + 0x38));
    let rva = cp.wrapping_sub(base) as u32;
    let count = p.mem.read_u32(data).unwrap_or(0).min(4096);
    for i in 0..u64::from(count) {
        let e = data + 4 + 16 * i;
        let f = |p: &mut Process, o: u64| p.mem.read_u32(e + o).unwrap_or(0);
        let (begin, end, handler, jump) = (f(p, 0), f(p, 4), f(p, 8), f(p, 12));
        if rva < begin || rva >= end {
            continue;
        }
        if flags & EXC_UNWINDING != 0 {
            if jump != 0 {
                if target_ip == base + u64::from(jump) {
                    return 1; // reached the scope that handles it
                }
            } else {
                let _ = call_guest(p, frame, w.as_deref_mut(), base + u64::from(handler), [1, est, 0, 0]);
            }
            continue;
        }
        if jump == 0 {
            continue;
        }
        let decision = if handler == 1 {
            1
        } else {
            let ep = p.scratch_alloc(16);
            w64(p, ep, rec);
            w64(p, ep + 8, ctxa);
            match call_guest(p, frame, w.as_deref_mut(), base + u64::from(handler), [ep, est, 0, 0]) {
                Stop::CbRet(v) => v as i32,
                _ => 0,
            }
        };
        if decision < 0 {
            return 0;
        }
        if decision > 0 {
            let Some(start) = p.read_ctx(ctxa) else { return 1 };
            let code = u64::from(p.mem.read_u32(rec).unwrap_or(0));
            return if unwind_to(p, frame, w.as_deref_mut(), est, base + u64::from(jump), rec, code, start) { 100 } else { 1 };
        }
    }
    1
}

/// `RtlUnwindEx`: from `start` up to the frame `target` (0: all), running termination handlers, then continues at `ip`
/// in that frame with `ret` in rax. Returns false if the frame was not found.
fn unwind_to(p: &mut Process, frame: &mut UFrame, mut w: Option<&mut Win32k>, target: u64, ip: u64, rec: u64, ret: u64, start: Ctx) -> bool {
    let fl = p.mem.read_u32(rec + 4).unwrap_or(0);
    let mut ctx = start;
    let mut first = true;
    for _ in 0..512 {
        let pc = if first { ctx.rip } else { ctx.rip.wrapping_sub(1) };
        first = false;
        let Some((ii, fi)) = p.lookup_function(pc) else {
            let ra = p.mem.read_u64(ctx.r[4]).unwrap_or(0);
            if ra == 0 {
                return false;
            }
            ctx.rip = ra;
            ctx.r[4] += 8;
            continue;
        };
        let before = ctx;
        let Some(out) = p.virtual_unwind(ii, fi, pc, UNW_UHANDLER, &mut ctx) else { return false };
        let is_target = out.frame == target;
        if let Some(h) = out.handler {
            let cc = make_ctx(p, &before);
            let (base, func) = (p.images[ii].base, p.function_entry_addr(ii, fi));
            let dc = dispatcher_context(p, pc, base, func, out.frame, ip, cc, h, out.data);
            w32(p, rec + 4, fl | EXC_UNWINDING | if is_target { EXC_TARGET_UNWIND } else { 0 });
            let _ = run_handler(p, frame, w.as_deref_mut(), h, rec, out.frame, cc, dc);
        }
        if is_target {
            let mut fin = before;
            fin.rip = ip;
            fin.r[0] = ret;
            apply_ctx(frame, &fin);
            return true;
        }
    }
    false
}

/// Dispatches an exception: walks the stack from the context in `ctxa`, offering it to each frame's handler.
/// Returns true if execution continues (handled), false if nobody handled it.
fn dispatch(p: &mut Process, frame: &mut UFrame, mut w: Option<&mut Win32k>, rec: u64, ctxa: u64, exact: bool) -> bool {
    let Some(mut ctx) = p.read_ctx(ctxa) else { return false };
    let mut first = exact;
    for _ in 0..512 {
        let pc = if first { ctx.rip } else { ctx.rip.wrapping_sub(1) };
        first = false;
        let Some((ii, fi)) = p.lookup_function(pc) else {
            let ra = p.mem.read_u64(ctx.r[4]).unwrap_or(0);
            if ra == 0 {
                return false;
            }
            ctx.rip = ra;
            ctx.r[4] += 8;
            continue;
        };
        let before = ctx;
        let Some(out) = p.virtual_unwind(ii, fi, pc, UNW_EHANDLER, &mut ctx) else { return false };
        if let Some(h) = out.handler {
            let cc = make_ctx(p, &before);
            let (base, func) = (p.images[ii].base, p.function_entry_addr(ii, fi));
            let dc = dispatcher_context(p, pc, base, func, out.frame, 0, cc, h, out.data);
            match run_handler(p, frame, w.as_deref_mut(), h, rec, out.frame, ctxa, dc) {
                0 => {
                    if let Some(c) = p.read_ctx(ctxa) {
                        apply_ctx(frame, &c);
                    }
                    return true;
                }
                100 => return true,
                _ => {}
            }
        }
    }
    false
}

/// Gives an unhandled exception to the program's top-level filter; true if it asks to continue.
fn unhandled(p: &mut Process, frame: &mut UFrame, w: Option<&mut Win32k>, rec: u64, ctxa: u64) -> bool {
    let code = p.mem.read_u32(rec).unwrap_or(0);
    let addr = p.mem.read_u64(rec + 0x10).unwrap_or(0);
    if p.unhandled_filter != 0 {
        let ep = p.scratch_alloc(16);
        w64(p, ep, rec);
        w64(p, ep + 8, ctxa);
        if let Stop::CbRet(v) = call_guest(p, frame, w, p.unhandled_filter, [ep, 0, 0, 0]) {
            if v as i32 == -1 {
                return true;
            }
        }
    }
    say(&alloc::format!("win: unhandled exception {code:#x} at {addr:#x}\n"));
    false
}

/// Raises a CPU fault as an SEH exception. True if it was handled (the frame was updated).
fn raise_fault(p: &mut Process, frame: &mut UFrame, mut w: Option<&mut Win32k>, vector: u64, cr2: u64) -> bool {
    let code = match vector {
        0 => 0xC000_0094u32,
        3 => 0x8000_0003,
        6 => 0xC000_001D,
        13 | 14 => 0xC000_0005,
        _ => return false,
    };
    let mark = p.scratch_mark();
    let mut ctx = Ctx { r: frame.gpr, rip: frame.rip };
    ctx.r[4] = frame.rsp;
    let args: Vec<u64> = if vector == 14 { alloc::vec![(frame.error >> 1) & 1, cr2] } else if vector == 13 { alloc::vec![0, 0] } else { Vec::new() };
    let rec = make_record(p, code, 0, frame.rip, &args);
    let ctxa = make_ctx(p, &ctx);
    let ok = dispatch(p, frame, w.as_deref_mut(), rec, ctxa, true) || unhandled(p, frame, w.as_deref_mut(), rec, ctxa);
    p.scratch_release(mark);
    ok
}

/// Switches to the next thread round-robin (the current one goes to the back).
fn rotate(p: &mut Process, frame: &mut UFrame) {
    // SAFETY: single CPU.
    let sched = unsafe { &mut *core::ptr::addr_of_mut!(SCHED) };
    if sched.is_empty() {
        return;
    }
    let (i, f) = sched.remove(0);
    sched.push((p.cur, *frame));
    *frame = f;
    p.cur = i;
}

/// A thread's call cannot finish: run someone else, or report that nothing can make progress.
fn block(p: &mut Process, frame: &mut UFrame, idle: bool) -> Option<Stop> {
    // SAFETY: single CPU statics.
    p.net_poll(unsafe { time_us() } / 1000);
    unsafe {
        SPIN += 1;
        ANY_IDLE |= idle;
        let n = (*core::ptr::addr_of!(SCHED)).len();
        if DEPTH > 0 {
            return if idle { Some(Stop::Idle) } else { None };
        }
        if SPIN > 8 * (n as u32 + 1) + 16 {
            if ANY_IDLE {
                return Some(Stop::Idle);
            }
            if !p.has_timed_waiters() {
                say("win: all threads blocked (deadlock)\n");
                return Some(Stop::Park);
            }
        }
        if n > 0 {
            rotate(p, frame);
        }
    }
    None
}

fn run(p: &mut Process) -> End {
    let mut frame = new_frame(p);
    match drive(p, &mut frame, None) {
        Stop::Park | Stop::Idle => End::Park,
        Stop::Exit(c) => End::Exit(c),
        Stop::CbRet(_) => End::Exit(0),
    }
}

/// Calls `addr(args...)` in ring 3 on the current thread's stack and returns how it ended (`CbRet` = returned).
fn call_guest(p: &mut Process, frame: &mut UFrame, w: Option<&mut Win32k>, addr: u64, args: [u64; 4]) -> Stop {
    let saved = *frame;
    let sp = ((saved.rsp - 0x400) & !0xf) - 8;
    if p.mem.write(sp, &p.cb_stub.to_le_bytes()).is_err() {
        return Stop::CbRet(0);
    }
    frame.rip = addr;
    frame.rsp = sp;
    frame.gpr[1] = args[0];
    frame.gpr[2] = args[1];
    frame.gpr[8] = args[2];
    frame.gpr[9] = args[3];
    // SAFETY: single CPU.
    unsafe { DEPTH += 1 };
    let r = drive(p, frame, w);
    // SAFETY: single CPU.
    unsafe { DEPTH -= 1 };
    *frame = saved;
    r
}

/// Window-procedure upcall: runs `addr(hwnd, msg, wparam, lparam)` in ring 3.
fn upcall(w: &mut Win32k, addr: u64, h: u32, msg: u32, wp: usize, lp: isize) -> isize {
    // SAFETY: CUR was published by `drive` for the syscall that is executing this hook.
    let (pp, fp) = unsafe { CUR };
    if pp.is_null() {
        return 0;
    }
    // SAFETY: both pointers refer to live locals of the enclosing `drive` (see CUR).
    let (p, frame) = unsafe { (&mut *pp, &mut *fp) };
    let r = call_guest(p, frame, Some(w), addr, [u64::from(h), u64::from(msg), wp as u64, lp as u64]);
    // SAFETY: restore the outer syscall's context.
    unsafe { CUR = (pp, fp) };
    match r {
        Stop::CbRet(v) => v as isize,
        Stop::Exit(c) => {
            // SAFETY: single CPU; read by the outer `drive` right after the syscall returns.
            unsafe { PENDING_EXIT = Some(c) };
            0
        }
        _ => 0,
    }
}

extern "C" {
    fn gui_present();
}

/// Runs one Windows GUI program from the tar `initrd` (`Windows/System32/<image>`) on a `w` x `h` desktop,
/// composing into `back` (0xAARRGGBB) and calling the C `gui_present` after every idle point. `keys` (ASCII, may be
/// empty) is typed into the program once it first waits for input. Never returns: the final frame stays on screen.
///
/// # Safety
/// `initrd`/`len` describe the boot module; `back` points at `w * h` writable pixels that outlive the call.
#[no_mangle]
pub unsafe extern "C" fn loos_win_gui(initrd: *const u8, len: usize, image: *const u8, back: *mut u32, w: i32, h: i32, keys: *const u8, open: *const u8, save: *const u8) -> i32 {
    // SAFETY: per the contract; `image` and `keys` are NUL-terminated.
    let (tar, image, (keys, open, save), fb) = unsafe {
        user_init();
        let cs = |p: *const u8| {
            let mut n = 0;
            while !p.is_null() && *p.add(n) != 0 {
                n += 1;
            }
            if p.is_null() { String::new() } else { String::from_utf8_lossy(core::slice::from_raw_parts(p, n)).into_owned() }
        };
        (if initrd.is_null() { &[][..] } else { core::slice::from_raw_parts(initrd, len) }, cs(image), (cs(keys), cs(open), cs(save)), core::slice::from_raw_parts_mut(back, (w * h) as usize))
    };
    say("win: gui start\n");
    let mut fs = MemFs::new();
    for d in ["/Windows", "/Windows/System32"] {
        fs.add_dir(d);
    }
    fs.load_tar(tar);
    say("win: gui rootfs loaded\n");
    let Some(bytes) = fs.file(&alloc::format!("/Windows/System32/{image}")).map(<[u8]>::to_vec) else {
        say("win: gui image not found\n");
        return -1;
    };
    let mut p = match Process::create(0x200, 1, &image, "", &bytes) {
        Ok(p) => p,
        Err(e) => {
            say(&alloc::format!("win: {image}: {e:?}\n"));
            return -1;
        }
    };
    p.soft_stubs = true;
    let (sam, lsa, rid) = boot_accounts();
    p.set_accounts(sam, lsa, rid);
    if crate::sys::KNet::present() {
        p.set_net(alloc::boxed::Box::new(crate::sys::KNet));
        say("win: network stack attached\n");
    }
    for (path, data) in fs.files() {
        p.add_file(path, data);
    }
    p.dialog_open = open;
    p.dialog_save = save;
    p.trace = keys.starts_with('!');
    let keys = keys.trim_start_matches('!').to_string();
    let mut win = Win32k::new(w, h);
    win.set_upcall(alloc::boxed::Box::new(upcall));
    let mut frame = new_frame(&p);
    // Input script: plain characters are typed, `~` is Enter, `@x,y;` clicks at (x, y). After every action the
    // program runs until it waits for input again and the frame is presented.
    enum Act {
        Key(u32),
        Click(i32, i32),
    }
    let mut acts = Vec::new();
    let kb = keys.as_bytes();
    let mut i = 0;
    while i < kb.len() {
        match kb[i] {
            b'@' => {
                let end = keys[i..].find(';').map_or(kb.len(), |e| i + e);
                let mut it = keys[i + 1..end].split(',').map(|n| n.parse::<i32>().unwrap_or(0));
                acts.push(Act::Click(it.next().unwrap_or(0), it.next().unwrap_or(0)));
                i = end + 1;
            }
            b'~' => {
                acts.push(Act::Key(13));
                i += 1;
            }
            b'_' => {
                acts.push(Act::Key(0x20));
                i += 1;
            }
            c => {
                acts.push(Act::Key(u32::from(c.to_ascii_uppercase())));
                i += 1;
            }
        }
    }
    let before = p.vfs.clone();
    let mut next = 0;
    let end = loop {
        // SAFETY: single CPU statics.
        unsafe {
            SPIN = 0;
            ANY_IDLE = false;
        }
        let stop = drive(&mut p, &mut frame, Some(&mut win));
        win.compose(fb);
        // SAFETY: C glue presenting the back buffer.
        unsafe { gui_present() };
        if !matches!(stop, Stop::Idle) {
            break stop;
        }
        let Some(act) = acts.get(next) else { break stop };
        next += 1;
        match *act {
            Act::Key(vk) => {
                win.inject_key(vk, true);
                win.inject_key(vk, false);
            }
            Act::Click(x, y) => {
                win.inject_mouse_move(x, y);
                win.inject_mouse_button(0, true);
                win.inject_mouse_button(0, false);
            }
        }
    };
    match end {
        Stop::Idle => say("win: gui idle (final frame presented)\n"),
        Stop::Exit(c) => say(&alloc::format!("win: gui process exited {c:#x}\n")),
        _ => say("win: gui process parked\n"),
    }
    for (path, data) in &p.vfs {
        if before.get(path) != Some(data) {
            say(&alloc::format!("win: file {path} = {:?}\n", String::from_utf8_lossy(&data[..data.len().min(120)])));
        }
    }
    loop {
        // SAFETY: halts the CPU; QEMU keeps the last presented frame on screen.
        unsafe { core::arch::asm!("hlt") };
    }
}

impl WinKernel {
    fn spawn_and_run(&mut self, image: &str, cmdline: &str, session: u32) -> Result<(u64, End), String> {
        let path = alloc::format!("/Windows/System32/{image}");
        let bytes = self.fs.file(&path).ok_or_else(|| alloc::format!("{path} not found"))?.to_vec();
        self.next_pid += 4;
        let pid = self.next_pid;
        let mut p = Process::create(pid, session, image, cmdline, &bytes).map_err(|e| alloc::format!("{image}: {e:?}"))?;
        let (sam, lsa, rid) = &self.accounts;
        p.set_accounts(sam.clone(), lsa.clone(), *rid);
        let end = run(&mut p);
        if matches!(end, End::Park) {
            self.resident.insert(pid, p);
        }
        Ok((pid, end))
    }
}

impl Launcher for WinKernel {
    fn launch(&mut self, image: &str, cmdline: &str, session: u32, _env: &[(String, String)]) -> Result<ProcessId, String> {
        let (pid, end) = self.spawn_and_run(image, cmdline, session)?;
        if let End::Exit(c) = end {
            self.exited.push((pid, c));
        }
        Ok(pid)
    }
    fn run_to_completion(&mut self, image: &str, cmdline: &str) -> Result<u32, String> {
        match self.spawn_and_run(image, cmdline, 0)?.1 {
            End::Exit(c) => Ok(c),
            End::Park => Err("did not exit".to_string()),
        }
    }
    fn terminate(&mut self, pid: ProcessId) {
        self.resident.remove(&pid);
    }
}

/// Boots the session manager with real PE processes from the tar `initrd` (`Windows/System32/*.exe`).
/// Prints the session summary; returns a bitmask: 1 boot ok, 2 session 0 resident, 4 interactive session created,
/// 8 session teardown released its processes, 16 critical-exit policy reported a bugcheck.
#[no_mangle]
pub unsafe extern "C" fn loos_win_boot(initrd: *const u8, len: usize) -> u32 {
    // SAFETY: initrd/len describe the boot module.
    let tar = unsafe {
        user_init();
        if initrd.is_null() { &[][..] } else { core::slice::from_raw_parts(initrd, len) }
    };
    let mut fs = MemFs::new();
    for d in ["/Windows", "/Windows/System32"] {
        fs.add_dir(d);
    }
    let files = fs.load_tar(tar);
    say("win: rootfs ");
    // SAFETY: serial output.
    unsafe { serial_dec(files as u64) };
    say(" entries\n");
    let mut k = WinKernel { accounts: boot_accounts(), fs, next_pid: 0x100, resident: BTreeMap::new(), exited: Vec::new() };
    let mut cfg = SmConfig::default();
    cfg.boot_execute = alloc::vec![String::from("autochk.exe autochk *")];
    let mut s = Smss::new(cfg);
    let mut r = 0u32;
    if let Err(e) = s.boot(&mut k) {
        say("win: smss boot failed: ");
        say(&alloc::format!("{e:?}\n"));
        return r;
    }
    r |= 1;
    if k.resident.len() == 4 {
        r |= 2;
    }
    if let Ok(id) = s.create_session(&mut k) {
        if id == 1 && k.resident.len() == 6 {
            r |= 4;
        }
        if s.terminate_session(id, &mut k).is_ok() && k.resident.len() == 4 {
            r |= 8;
        }
    }
    if let Some(&lsass) = k.resident.iter().find(|(_, p)| p.name == "lsass.exe").map(|(pid, _)| pid) {
        if matches!(s.on_exit(lsass, 0xc000_0005, &mut k), Outcome::Bugcheck { .. }) {
            r |= 16;
        }
    }
    for line in &s.log {
        say("smss: ");
        say(line);
        say("\n");
    }
    r
}
