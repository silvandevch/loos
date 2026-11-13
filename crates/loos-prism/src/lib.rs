//! LoOS Prism: run macOS programs on LoOS.
//!
//! A Mach-O process is an ordinary process of the Linux subsystem ([`loos_lin::LinSystem`]):
//! it shares its address spaces, file descriptors, VFS, scheduler, signals and window plumbing.
//! Prism adds what differs:
//!
//! * [`macho`] / [`loader`] — Mach-O (x86_64, arm64, universal) parsing, mapping, bundled dylibs,
//!   rebase/bind (opcode and chained fixups), initializers.
//! * [`libsys`] — libSystem implemented in Rust; imports from macOS system libraries bind to it.
//! * [`darwin`] — Darwin BSD syscalls and Mach traps, translated onto LoOS syscalls.
//! * [`arm64`] — the AArch64 interpreter for Apple Silicon binaries (kept off the native path).
//! * [`bundle`] — `.app` bundles.
//!
//! The kernel's Linux run loop asks [`Prism::syscall`] first for every syscall of a Prism
//! process, and runs arm64 processes through [`Prism::run_arm`] instead of entering ring 3.
#![no_std]
extern crate alloc;

#[allow(unused_imports)]
use alloc::{boxed::Box, collections::BTreeMap, format, string::{String, ToString}, vec, vec::Vec};
use libsys::{Abi, Act, Args, Post, State};
use loader::{PRISM_NR, SCRATCH, SHIM_BASE, SHIM_DATA};
use loos_lin::fs::LinFs;
use loos_lin::mem::UserMem;
use loos_lin::sys::{LinSystem, Pid, Regs, Sys};

pub mod arm64;
pub mod bundle;
pub mod darwin;
pub mod libsys;
pub mod posix;
pub mod posix2;
pub mod getopt;
pub mod signal;
pub mod loader;
pub mod macho;

/// x86-64 SysV argument access over a trapped thread's registers.
struct X86Call<'a> {
    r: &'a mut Regs,
    xmm: [u64; 8],
    gi: usize,
    fi: usize,
    stack: u64,
}

impl X86Call<'_> {
    fn gpr(&mut self, i: usize) -> &mut u64 {
        match i {
            0 => &mut self.r.rdi,
            1 => &mut self.r.rsi,
            2 => &mut self.r.rdx,
            3 => &mut self.r.rcx,
            4 => &mut self.r.r8,
            _ => &mut self.r.r9,
        }
    }
    fn slot(&mut self, mem: &mut UserMem) -> u64 {
        let v = mem.read_u64(self.stack).unwrap_or(0);
        self.stack += 8;
        v
    }
}

impl Args for X86Call<'_> {
    fn int(&mut self, mem: &mut UserMem) -> u64 {
        if self.gi < 6 {
            self.gi += 1;
            *self.gpr(self.gi - 1)
        } else {
            self.slot(mem)
        }
    }
    fn float(&mut self, mem: &mut UserMem) -> f64 {
        if self.fi < 8 {
            self.fi += 1;
            f64::from_bits(self.xmm[self.fi - 1])
        } else {
            f64::from_bits(self.slot(mem))
        }
    }
}

impl Abi for X86Call<'_> {
    fn va_start(&mut self) {}
    fn ret(&mut self, v: u64) {
        self.r.rax = v;
    }
    fn set_arg(&mut self, i: usize, v: u64) {
        *self.gpr(i) = v;
    }
    fn jump(&mut self, mem: &mut UserMem, target: u64, ret_to: Option<u64>) -> Result<(), i64> {
        // The stub's `ret` pops `target`; the callee then sees `ret_to` as its return address
        // with the ABI's entry alignment (rsp ≡ 8 mod 16).
        let mut sp = self.r.rsp;
        if let Some(r) = ret_to {
            sp = (sp & !15) - 8;
            mem.write_u64(sp, r)?;
        }
        sp -= 8;
        mem.write_u64(sp, target)?;
        self.r.rsp = sp;
        Ok(())
    }
}

/// A Prism process.
struct Proc {
    st: State,
    /// Interpreter state for arm64 programs.
    arm: Option<Box<arm64::Cpu>>,
    /// Result conversion owed to a LoOS syscall that blocked.
    pending: Option<Post>,
}

/// The macOS compatibility layer.
#[derive(Default)]
pub struct Prism {
    procs: BTreeMap<Pid, Proc>,
    /// Run the ARM slice of universal binaries (on the interpreter) instead of the Intel one.
    pub prefer_arm: bool,
}

/// Is `path` (or the file at it) something Prism runs: a Mach-O file or an `.app` bundle?
pub fn recognizes(fs: &mut dyn LinFs, path: &str) -> bool {
    if bundle::open(fs, path).is_some() {
        return true;
    }
    let Ok(real) = loos_lin::fs::resolve(fs, "/", path, true) else { return false };
    let mut head = [0u8; 8];
    matches!(fs.read(&real, 0, &mut head), Ok(8)) && macho::is_macho(&head)
}

impl Prism {
    /// Empty layer.
    pub fn new() -> Self {
        Self::default()
    }

    /// Does Prism own process `pid`?
    pub fn owns(&self, pid: Pid) -> bool {
        self.procs.contains_key(&pid)
    }

    /// Index of the libSystem stub bound for `name` in process `pid`.
    pub fn stub_index(&self, pid: Pid, name: &str) -> Option<usize> {
        self.procs.get(&pid)?.st.stubs.iter().position(|s| s == name)
    }

    /// Is `pid` an arm64 (interpreted) process?
    pub fn is_arm(&self, pid: Pid) -> bool {
        self.procs.get(&pid).is_some_and(|p| p.arm.is_some())
    }

    /// Start a macOS program (Mach-O file or `.app` bundle). Intel code runs natively; arm64-only
    /// programs run on the interpreter.
    pub fn spawn<F: LinFs>(&mut self, sys: &mut LinSystem<F>, path: &str, argv: &[String], envp: &[String]) -> Result<Pid, i64> {
        let exe = match bundle::open(&mut sys.fs, path) {
            Some(b) => b.exe_path(),
            None => path.to_string(),
        };
        let mut mem = UserMem::new();
        let prog = match loader::load(&mut sys.fs, &mut mem, &exe, argv, envp, if self.prefer_arm { &[macho::CPU_ARM64, macho::CPU_X86_64] } else { &[macho::CPU_X86_64, macho::CPU_ARM64] }) {
            Ok(p) => p,
            Err(e) => {
                sys.diag += &format!("prism: cannot load {exe} (errno {e})\n");
                return Err(e);
            }
        };
        sys.diag += &prog.notes;
        let arm = prog.cpu == macho::CPU_ARM64;
        let mut st = State::new(prog.stubs.clone());
        st.inits = prog.inits.clone();
        match prog.entry {
            macho::Entry::Main(a) => st.main = Some(a),
            macho::Entry::Thread(a) => st.thread_entry = a,
        }
        st.start_args = [prog.argc, prog.argv, prog.envp, prog.apple];
        st.tlv = prog.tlv.clone();
        st.arm = arm;
        st.epoch_ns = sys.epoch_base_s.saturating_mul(1_000_000_000);
        // Execution starts in the `__prism_start` stub, which runs initializers, then main.
        let regs = Regs { rip: SHIM_BASE, rsp: prog.sp, rflags: 0x202, ..Regs::default() };
        let pid = sys.spawn_image(mem, regs, exe, argv)?;
        let arm = arm.then(|| {
            let mut c = Box::new(arm64::Cpu::new(SHIM_BASE, prog.sp));
            c.tpidrro = SHIM_DATA + 0x800; // Darwin TSD area
            c
        });
        self.procs.insert(pid, Proc { st, arm, pending: None });
        Ok(pid)
    }

    /// Forget processes that no longer exist.
    pub fn reap<F: LinFs>(&mut self, sys: &LinSystem<F>) {
        self.procs.retain(|pid, _| sys.process(*pid).is_some());
    }

    /// Handle a syscall trapped from a native (x86-64) Prism thread. `None`: not a Prism process.
    pub fn syscall<F: LinFs>(&mut self, sys: &mut LinSystem<F>, tid: Pid, nr: u64, a: [u64; 6]) -> Option<Sys> {
        let pid = sys.thread(tid)?.pid;
        let proc = self.procs.get_mut(&pid)?;
        let now = sys.now_ns;
        if (PRISM_NR..PRISM_NR + 0x1_0000).contains(&nr) {
            let t = sys.thread(tid)?;
            let mut regs = t.regs;
            regs.rcx = regs.r10; // the stub saved rcx (4th argument) before `syscall` clobbered it
            let mut xmm = [0u64; 8];
            for (k, x) in xmm.iter_mut().enumerate() {
                *x = u64::from_le_bytes(t.fx[160 + 16 * k..168 + 16 * k].try_into().unwrap());
            }
            let stack = regs.rsp + 8;
            let mem = &mut sys.process_mut(pid)?.mem;
            let mut call = X86Call { r: &mut regs, xmm, gi: 0, fi: 0, stack };
            let i = (nr - PRISM_NR) as usize;
            let act = proc.st.call(i, &mut call, mem, now);
            sys.thread_mut(tid)?.regs = regs;
            return Some(finish(proc, sys, tid, pid, i, act, false));
        }
        let plan = match nr >> 24 {
            2 => darwin::bsd(nr & 0xff_ffff, a),
            1 => darwin::mach(nr & 0xff_ffff, a),
            3 if nr & 0xff_ffff == 3 => {
                // thread_set_tsd_base: Darwin keeps its TSD pointer in gs.
                sys.thread_mut(tid)?.regs.gs_base = a[0];
                darwin::Plan::Value(0)
            }
            _ => darwin::Plan::Error(38),
        };
        Some(run_plan(proc, sys, tid, plan, false))
    }

    /// Before a Prism thread runs again: finish the result conversion of a syscall that blocked.
    pub fn resume<F: LinFs>(&mut self, sys: &mut LinSystem<F>, tid: Pid) {
        let Some(pid) = sys.thread(tid).map(|t| t.pid) else { return };
        let Some(proc) = self.procs.get_mut(&pid) else { return };
        if let Some(post) = proc.pending.take() {
            let v = sys.thread(tid).map(|t| t.regs.rax as i64).unwrap_or(0);
            let arm = proc.arm.is_some();
            apply_post(proc, sys, tid, post, v, arm);
        }
    }

    /// Run an arm64 thread for up to `budget` instructions (until it makes a call that blocks,
    /// exits, or faults). Returns what the Linux layer reported for the last syscall, if any.
    pub fn run_arm<F: LinFs>(&mut self, sys: &mut LinSystem<F>, tid: Pid, budget: u64) -> Option<Sys> {
        let pid = sys.thread(tid)?.pid;
        self.resume(sys, tid);
        let proc = self.procs.get_mut(&pid)?;
        let mut left = budget;
        while left > 0 {
            let mut cpu = proc.arm.take()?;
            let before = cpu.icount;
            let stop = cpu.run(&mut sys.process_mut(pid)?.mem, left);
            left = left.saturating_sub((cpu.icount - before).max(1));
            let r = match stop {
                arm64::Stop::Budget => {
                    proc.arm = Some(cpu);
                    None
                }
                arm64::Stop::Shim(i) => {
                    let now = sys.now_ns;
                    let mem = &mut sys.process_mut(pid)?.mem;
                    let mut call = arm64::ArmCall::new(&mut cpu);
                    let act = proc.st.call(i, &mut call, mem, now);
                    if !call.jumped {
                        cpu.pc = cpu.x[30];
                    }
                    proc.arm = Some(cpu);
                    Some(finish(proc, sys, tid, pid, i, act, true))
                }
                arm64::Stop::Svc(_) => {
                    let n = cpu.x[16] as i64;
                    let a = [cpu.x[0], cpu.x[1], cpu.x[2], cpu.x[3], cpu.x[4], cpu.x[5]];
                    cpu.pc += 4;
                    proc.arm = Some(cpu);
                    let plan = if n < 0 {
                        darwin::mach((-n) as u64, a)
                    } else if n == 0x8000_0000 {
                        darwin::Plan::Value(0) // platform calls (thread_set_tsd_base) are handled via TPIDRRO_EL0
                    } else {
                        darwin::bsd(n as u64, a)
                    };
                    Some(run_plan(proc, sys, tid, plan, true))
                }
                arm64::Stop::Fault(addr) => {
                    sys.diag += &format!("prism: arm64 fault at {addr:#x} (pc {:#x})\n", cpu.pc);
                    proc.arm = Some(cpu);
                    sys.fault(tid, 14, 0, addr);
                    return Some(Sys::Ret(0));
                }
                arm64::Stop::Undefined(ins) => {
                    sys.diag += &format!("prism: arm64 instruction {ins:#010x} at {:#x} is not supported\n", cpu.pc);
                    proc.arm = Some(cpu);
                    sys.send_signal(pid, Some(tid), 4);
                    return Some(Sys::Ret(0));
                }
                arm64::Stop::Brk => {
                    sys.diag += &format!("prism: arm64 brk at {:#x}\n", cpu.pc);
                    proc.arm = Some(cpu);
                    sys.send_signal(pid, Some(tid), 5);
                    return Some(Sys::Ret(0));
                }
            };
            match r {
                Some(Sys::Ret(_)) | Some(Sys::Restored) | None => {}
                other => return other,
            }
            if sys.thread(tid).is_none_or(|t| t.state != loos_lin::sys::TState::Runnable) {
                return Some(Sys::Blocked);
            }
        }
        None
    }
}

fn set_result<F: LinFs>(proc: &mut Proc, sys: &mut LinSystem<F>, tid: Pid, v: u64, carry: Option<bool>, arm: bool) {
    if arm {
        if let Some(c) = proc.arm.as_mut() {
            c.x[0] = v;
            if let Some(cf) = carry {
                c.c = cf;
            }
        }
    } else if let Some(t) = sys.thread_mut(tid) {
        t.regs.rax = v;
        if let Some(cf) = carry {
            t.regs.rflags = (t.regs.rflags & !1) | cf as u64;
        }
    }
}

fn apply_post<F: LinFs>(proc: &mut Proc, sys: &mut LinSystem<F>, tid: Pid, post: Post, v: i64, arm: bool) {
    let pid = sys.thread(tid).map(|t| t.pid).unwrap_or(0);
    let set_errno = |sys: &mut LinSystem<F>, e: i64| {
        if let Some(p) = sys.process_mut(pid) {
            let _ = p.mem.write_u32(SHIM_DATA + libsys::D_ERRNO, darwin::linux_to_darwin_errno(e) as u32);
        }
    };
    match post {
        Post::Raw => set_result(proc, sys, tid, v as u64, None, arm),
        Post::Cont(id, ctx) => {
            let Some(p) = sys.process_mut(pid) else { return };
            match posix::cont(id, ctx, v, &mut p.mem, &mut proc.st) {
                libsys::Then::Ret(r) => set_result(proc, sys, tid, r, None, arm),
                libsys::Then::Fail(e, r) => {
                    set_errno(sys, e);
                    set_result(proc, sys, tid, r, None, arm);
                }
                libsys::Then::Linux(nr, args, post) => {
                    forward(proc, sys, tid, nr, args, post, arm);
                }
            }
        }
        Post::Libc | Post::Value(_) if v < 0 => {
            set_errno(sys, -v);
            set_result(proc, sys, tid, u64::MAX, None, arm);
        }
        Post::Libc => set_result(proc, sys, tid, v as u64, None, arm),
        Post::Value(x) => set_result(proc, sys, tid, x, None, arm),
        Post::Darwin | Post::DarwinStat(_) | Post::DarwinPipe if v < 0 => set_result(proc, sys, tid, darwin::linux_to_darwin_errno(-v) as u64, Some(true), arm),
        Post::Darwin => set_result(proc, sys, tid, v as u64, Some(false), arm),
        Post::DarwinStat(dst) => {
            let ok = sys.process_mut(pid).is_some_and(|p| {
                let mut l = [0u8; 144];
                p.mem.read(darwin::STAT_STAGE, &mut l).is_ok() && p.mem.write(dst, &darwin::stat_to_darwin(&l)).is_ok()
            });
            if ok {
                set_result(proc, sys, tid, 0, Some(false), arm);
            } else {
                set_result(proc, sys, tid, 14, Some(true), arm);
            }
        }
        Post::DarwinPipe => {
            let fds = sys.process_mut(pid).and_then(|p| p.mem.read_u64(SCRATCH).ok()).unwrap_or(0);
            let (r, w) = (fds & 0xffff_ffff, fds >> 32);
            set_result(proc, sys, tid, r, Some(false), arm);
            if arm {
                if let Some(c) = proc.arm.as_mut() {
                    c.x[1] = w;
                }
            } else if let Some(t) = sys.thread_mut(tid) {
                t.regs.rdx = w;
            }
        }
    }
}

fn forward<F: LinFs>(proc: &mut Proc, sys: &mut LinSystem<F>, tid: Pid, nr: u64, args: [u64; 6], post: Post, arm: bool) -> Sys {
    let saved = sys.thread(tid).map(|t| t.regs);
    let r = sys.syscall(tid, nr, args);
    match r {
        Sys::Ret(v) => {
            if !arm {
                // The Linux layer wrote rax; everything else the program sees stays as it was.
                if let (Some(s), Some(t)) = (saved, sys.thread_mut(tid)) {
                    t.regs = Regs { rax: t.regs.rax, ..s };
                }
            }
            apply_post(proc, sys, tid, post, v, arm);
            Sys::Ret(v)
        }
        Sys::Blocked => {
            proc.pending = Some(post);
            r
        }
        other => other,
    }
}

fn finish<F: LinFs>(proc: &mut Proc, sys: &mut LinSystem<F>, tid: Pid, pid: Pid, i: usize, act: Act, arm: bool) -> Sys {
    match act {
        Act::Done => Sys::Ret(0),
        Act::Linux(nr, args, post) => forward(proc, sys, tid, nr, args, post, arm),
        Act::Missing => {
            sys.diag += &format!("prism: {} is not implemented by Prism's libSystem\n", proc.st.name(i));
            sys.send_signal(pid, None, 6);
            Sys::Ret(0)
        }
        Act::Abort(msg) => {
            sys.diag += &format!("prism: {msg} (in {})\n", proc.st.name(i));
            sys.send_signal(pid, None, 6);
            Sys::Ret(0)
        }
    }
}

fn run_plan<F: LinFs>(proc: &mut Proc, sys: &mut LinSystem<F>, tid: Pid, plan: darwin::Plan, arm: bool) -> Sys {
    match plan {
        darwin::Plan::Linux(nr, args, post) => forward(proc, sys, tid, nr, args, post, arm),
        darwin::Plan::Value(v) => {
            set_result(proc, sys, tid, v, Some(false), arm);
            Sys::Ret(v as i64)
        }
        darwin::Plan::Error(e) => {
            set_result(proc, sys, tid, darwin::linux_to_darwin_errno(e) as u64, Some(true), arm);
            Sys::Ret(-e)
        }
    }
}
