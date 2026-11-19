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
pub mod cf;
pub mod darwin;
pub mod electron;
pub mod libsys;
pub mod posix;
pub mod posix2;
pub mod posix3;
pub mod process;
pub mod wide;
pub mod misc;
pub mod getopt;
pub mod signal;
pub mod loader;
pub mod macho;
pub mod pkg;
pub mod dmg;

/// x86-64 SysV argument access over a trapped thread's registers.
struct X86Call<'a> {
    r: &'a mut Regs,
    xmm: [u64; 8],
    /// A double result for xmm0.
    fret: Option<u64>,
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
    fn ret_f64(&mut self, v: f64) {
        self.fret = Some(v.to_bits());
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
#[derive(Clone)]
struct Proc {
    st: State,
    /// Interpreter state for arm64 programs.
    arm: Option<Box<arm64::Cpu>>,
    /// Result conversion owed to a LoOS syscall that blocked.
    pending: Option<Post>,
    /// A process-level change the last forwarded syscall made, for [`Prism::settle`].
    event: Option<Event>,
}

/// Process-level effects of a forwarded syscall.
#[derive(Clone, Copy)]
enum Event {
    /// `fork`: the child (a copy of this process) finishes the call with this conversion.
    Forked(Pid, Post),
    /// `execve` of a Linux program: the process is no longer Prism's.
    Exec,
    /// `execve` of a Mach-O: Prism loads it ([`LinSystem::foreign_exec`]).
    ExecForeign,
}

/// The macOS compatibility layer.
#[derive(Default)]
pub struct Prism {
    procs: BTreeMap<Pid, Proc>,
    /// Run the ARM slice of universal binaries (on the interpreter) instead of the Intel one.
    pub prefer_arm: bool,
}

/// Is `path` (or the file at it) something Prism runs: a Mach-O file, an `.app` bundle, an installer package
/// or a disk image (the app is installed / copied to /Applications, then runs)?
pub fn recognizes(fs: &mut dyn LinFs, path: &str) -> bool {
    if bundle::open(fs, path).is_some() || pkg::is_package(fs, path) || dmg::is_disk_image(fs, path) {
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
        // Installer packages and disk images: install / copy the app into /Applications, then start it.
        let installer = if pkg::is_package(&mut sys.fs, path) {
            Some(pkg::install(&mut sys.fs, path, &mut sys.diag).and_then(|a| a.ok_or(2)))
        } else if dmg::is_disk_image(&mut sys.fs, path) {
            Some(dmg::open(&mut sys.fs, path, &mut sys.diag))
        } else {
            None
        };
        if let Some(app) = installer {
            let app = app?;
            let mut av = argv.to_vec();
            if let Some(a0) = av.first_mut() {
                *a0 = app.clone();
            }
            return self.spawn(sys, &app, &av, envp);
        }
        // Electron apps run their JavaScript on the Linux build of Electron (see `electron`).
        if let Some(b) = bundle::open(&mut sys.fs, path) {
            if let Some(v) = electron::version(&mut sys.fs, &b) {
                let mut diag = String::new();
                let r = electron::prepare(&mut sys.fs, &b, &v, argv, envp, &mut diag);
                sys.diag += &diag;
                let l = r?;
                sys.diag += &format!("prism: {} is an Electron {v} app; running it on Linux Electron\n", b.path);
                return sys.spawn(&l.exe, &l.argv, &l.envp);
            }
        }
        let (mem, regs, exe, proc) = self.load(sys, path, argv, envp)?;
        let pid = sys.spawn_image(mem, regs, exe, argv)?;
        self.procs.insert(pid, proc);
        Ok(pid)
    }

    /// Finish a [`Sys::ExecForeign`] (an `execve` of a Mach-O by any process). On failure the
    /// caller's syscall returns the error.
    pub fn exec_foreign<F: LinFs>(&mut self, sys: &mut LinSystem<F>) -> Sys {
        let Some(req) = sys.foreign_exec.take() else { return Sys::Ret(-22) };
        let r = self.load(sys, &req.path, &req.argv, &req.envp).and_then(|(mem, regs, exe, proc)| {
            sys.exec_image(req.tid, mem, regs, exe, req.argv.clone())?;
            Ok(proc)
        });
        match r {
            Ok(proc) => {
                if let Some(pid) = sys.thread(req.tid).map(|t| t.pid) {
                    self.procs.insert(pid, proc);
                }
                Sys::Exec
            }
            Err(e) => {
                if let Some(t) = sys.thread_mut(req.tid) {
                    t.regs.rax = (-e) as u64; // a Prism caller converts it in `resume`
                }
                Sys::Ret(-e)
            }
        }
    }

    /// Load a macOS program into a fresh address space.
    fn load<F: LinFs>(&mut self, sys: &mut LinSystem<F>, path: &str, argv: &[String], envp: &[String]) -> Result<(UserMem, Regs, String, Proc), i64> {
        let exe = match bundle::open(&mut sys.fs, path) {
            Some(b) => b.exe_path(),
            None => path.to_string(),
        };
        let mut mem = UserMem::new();
        let mut why = String::new();
        let prog = match loader::load_noting(&mut sys.fs, &mut mem, &exe, argv, envp, if self.prefer_arm { &[macho::CPU_ARM64, macho::CPU_X86_64] } else { &[macho::CPU_X86_64, macho::CPU_ARM64] }, &mut why) {
            Ok(p) => p,
            Err(e) => {
                sys.diag += &why;
                sys.diag += &format!("prism: cannot load {exe} (errno {e})\n");
                return Err(e);
            }
        };
        sys.diag += &why;
        let arm = prog.cpu == macho::CPU_ARM64;
        let mut st = State::new(prog.stubs.clone());
        st.inits = prog.inits.clone();
        match prog.entry {
            macho::Entry::Main(a) => st.main = Some(a),
            macho::Entry::Thread(a) => st.thread_entry = a,
        }
        st.start_args = [prog.argc, prog.argv, prog.envp, prog.apple];
        st.tlv = prog.tlv.clone();
        st.images = prog
            .images
            .iter()
            .map(|im| {
                let text = im.macho.segments.iter().find(|s| s.fileoff == 0 && s.filesize > 0).map(|s| s.vmaddr).unwrap_or(0);
                (text.wrapping_add(im.slide), im.slide, im.path.clone(), 0)
            })
            .collect();
        st.runtime = prog.runtime;
        st.loaded = prog.images;
        for im in &mut st.loaded {
            im.macho.fixups = Vec::new(); // applied; `dlopen` needs the rest
        }
        st.arm = arm;
        st.epoch_ns = sys.epoch_base_s.saturating_mul(1_000_000_000);
        // Execution starts in the `__prism_start` stub, which runs initializers, then main.
        let regs = Regs { rip: SHIM_BASE, rsp: prog.sp, rflags: 0x202, ..Regs::default() };
        let arm = arm.then(|| {
            let mut c = Box::new(arm64::Cpu::new(SHIM_BASE, prog.sp));
            c.tpidrro = SHIM_DATA + 0x800; // Darwin TSD area
            c
        });
        Ok((mem, regs, exe, Proc { st, arm, pending: None, event: None }))
    }

    /// Apply the process-level effect (fork, exec) the last call of `pid` had, if any.
    fn settle<F: LinFs>(&mut self, sys: &mut LinSystem<F>, pid: Pid, r: Sys) -> Sys {
        let Some(ev) = self.procs.get_mut(&pid).and_then(|p| p.event.take()) else { return r };
        match ev {
            Event::Forked(child, post) => {
                let mut c = self.procs[&pid].clone();
                c.pending = Some(post);
                self.procs.insert(child, c);
                r
            }
            Event::Exec => {
                self.procs.remove(&pid);
                Sys::Exec
            }
            Event::ExecForeign => self.exec_foreign(sys),
        }
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
            let mut call = X86Call { r: &mut regs, xmm, fret: None, gi: 0, fi: 0, stack };
            let i = (nr - PRISM_NR) as usize;
            let act = proc.st.call(i, &mut call, mem, now);
            let fret = call.fret;
            let t = sys.thread_mut(tid)?;
            t.regs = regs;
            if let Some(f) = fret {
                t.fx[160..176].copy_from_slice(&(f as u128).to_le_bytes());
            }
            let r = finish(proc, sys, tid, pid, i, act, false);
            return Some(self.settle(sys, pid, r));
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
        let r = run_plan(proc, sys, tid, plan, false);
        Some(self.settle(sys, pid, r))
    }

    /// Before a Prism thread runs again: finish the result conversion of a syscall that blocked.
    pub fn resume<F: LinFs>(&mut self, sys: &mut LinSystem<F>, tid: Pid) {
        let Some(pid) = sys.thread(tid).map(|t| t.pid) else { return };
        let Some(proc) = self.procs.get_mut(&pid) else { return };
        if let Some(post) = proc.pending.take() {
            let v = sys.thread(tid).map(|t| t.regs.rax as i64).unwrap_or(0);
            let arm = proc.arm.is_some();
            apply_post(proc, sys, tid, post, v, arm);
            self.settle(sys, pid, Sys::Ret(0));
        }
    }

    /// Run an arm64 thread for up to `budget` instructions (until it makes a call that blocks,
    /// exits, or faults). Returns what the Linux layer reported for the last syscall, if any.
    pub fn run_arm<F: LinFs>(&mut self, sys: &mut LinSystem<F>, tid: Pid, budget: u64) -> Option<Sys> {
        let pid = sys.thread(tid)?.pid;
        self.resume(sys, tid);
        // Finishing the pending call can end the process (posix_spawn's child exits) or block it.
        if sys.thread(tid).is_none_or(|t| t.state != loos_lin::sys::TState::Runnable) || !self.is_arm(pid) {
            return Some(Sys::Ret(0));
        }
        let r = self.run_arm_inner(sys, tid, pid, budget)?;
        Some(self.settle(sys, pid, r))
    }

    fn run_arm_inner<F: LinFs>(&mut self, sys: &mut LinSystem<F>, tid: Pid, pid: Pid, budget: u64) -> Option<Sys> {
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
                    // abort() and missing functions do not return: stop before running past them
                    let fatal = matches!(act, Act::Abort(_) | Act::Missing);
                    let r = finish(proc, sys, tid, pid, i, act, true);
                    if fatal {
                        return Some(r);
                    }
                    Some(r)
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
                    sys.diag += &format!("prism: arm64 fault at {addr:#x} (pc {:#x}, pid {pid})\n", cpu.pc);
                    proc.arm = Some(cpu);
                    sys.fault(tid, 14, 0, addr);
                    return Some(Sys::Ret(0));
                }
                arm64::Stop::Undefined(ins) => {
                    sys.diag += &format!("prism: arm64 instruction {ins:#010x} at {:#x} is not supported (lr {:#x}, pid {pid})\n", cpu.pc, cpu.x[30]);
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
            if proc.event.is_some() {
                return r;
            }
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
    let forks = nr == 57 || (nr == 56 && args[0] & 0x100 == 0);
    match r {
        Sys::Ret(v) if forks && v > 0 => {
            proc.event = Some(Event::Forked(v as Pid, post));
        }
        Sys::Exec => proc.event = Some(Event::Exec),
        Sys::ExecForeign => {
            proc.pending = Some(post);
            proc.event = Some(Event::ExecForeign);
            return r;
        }
        _ => {}
    }
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
        Act::Dlopen(path) => {
            let handle = dlopen(&mut proc.st, sys, pid, &path).unwrap_or(0);
            set_result(proc, sys, tid, handle, None, arm);
            Sys::Ret(0)
        }
    }
}

/// Map `path` and its dylibs into process `pid`; the program's `dlopen` then runs their initializers.
fn dlopen<F: LinFs>(st: &mut State, sys: &mut LinSystem<F>, pid: Pid, path: &str) -> Option<u64> {
    let mut notes = String::new();
    let (fs, p) = sys.fs_and_process(pid)?;
    let cpu = if st.arm { macho::CPU_ARM64 } else { macho::CPU_X86_64 };
    let cwd = p.cwd.clone();
    let first = st.loaded.len();
    let r = loader::dlopen(fs, &mut p.mem, &cwd, cpu, &mut st.loaded, st.runtime, &st.stubs, path, &mut notes);
    let done = r.and_then(|o| {
        loader::map_stubs(&mut p.mem, st.stubs.len(), o.stubs.len())?;
        Ok(o)
    });
    sys.diag += &notes;
    let o = done.ok()?;
    st.set_stubs(o.stubs);
    st.dl_inits.extend(o.inits);
    st.tlv.extend(o.tlv);
    for im in &st.loaded[first..] {
        st.images.push((im.macho.base().wrapping_add(im.slide), im.slide, im.path.clone(), 0));
    }
    Some(o.handle)
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
