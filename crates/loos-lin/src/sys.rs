//! L1/L5: Linux x86-64 syscall layer over the LoOS executive model: processes,
//! threads (`clone`), futexes, pipes, signals, fork/exec/wait, time, VFS.

#[allow(unused_imports)]
use alloc::{boxed::Box, format, string::{String, ToString}, vec, vec::Vec};
use crate::errno::*;
const ENOTSOCK: i64 = 88;
const ENOTCONN: i64 = 107;
const ECONNRESET: i64 = 104;
const ECONNREFUSED: i64 = 111;
const EINPROGRESS: i64 = 115;
const EALREADY: i64 = 114;
const EDESTADDRREQ: i64 = 89;
use crate::fs::{normalize, resolve, Kind, LinFs, Stat};
use crate::loader;
use crate::mem::*;
use crate::wayland::{Frame as WaylandFrame, Server as WaylandServer, SharedFd};
use core::cell::RefCell;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::rc::Rc;
use core::net::{IpAddr, Ipv4Addr};
use loos_net::stack::Stack;
use loos_net::tcp::State;

/// Process/thread id (one shared id space, as in Linux).
pub type Pid = u32;

/// Base of the `liblooswin` bridge syscall range ("LO" << 16).
pub const BRIDGE_BASE: u64 = 0x4C4F_0000;
const AT_FDCWD: i32 = -100;
/// Wall-clock epoch offset (2025-10-09) added to the monotonic clock.
pub const EPOCH_BASE_S: u64 = 1_760_000_000;
const PIPE_CAP: usize = 64 * 1024;

/// CPU register state of a thread.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(missing_docs)]
pub struct Regs {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub rsp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub rflags: u64,
    pub fs_base: u64,
    pub gs_base: u64,
}

/// The network the Linux layer's AF_INET sockets run on. The kernel implements it over its NIC-backed stack; a bare
/// [`Stack`] (host tests) implements it with no hardware to pump.
pub trait LinNet {
    /// The TCP/IP stack, if a network adapter exists.
    fn stack(&mut self) -> Option<&mut Stack>;
    /// Moves frames between the adapter and the stack and runs timers.
    fn pump(&mut self, now_ms: u64);
}

impl LinNet for Stack {
    fn stack(&mut self) -> Option<&mut Stack> {
        Some(self)
    }
    fn pump(&mut self, now_ms: u64) {
        self.poll(now_ms);
    }
}

/// What a blocked thread waits for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wait {
    /// futex word (pid, address).
    Futex(Pid, u64),
    /// Timed sleep.
    Sleep,
    /// Pipe readable/writable.
    Pipe(usize),
    /// A child to exit.
    Child,
    /// Console input.
    Stdin,
    /// Socket readable / connection pending.
    Sock(usize),
    /// `poll`/`select`/`epoll_wait`: re-checked after every event.
    Poll,
    /// eventfd counter.
    Event(usize),
}

/// Thread scheduling state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TState {
    /// Can run.
    Runnable,
    /// Blocked inside a syscall.
    Blocked {
        /// Reason.
        wait: Wait,
        /// Syscall to re-run on wake (None: result is set by the waker).
        restart: Option<(u64, [u64; 6])>,
        /// Absolute deadline (ns).
        deadline: Option<u64>,
    },
}

/// A Linux thread.
#[derive(Clone, Debug)]
pub struct Thread {
    /// Thread id.
    pub tid: Pid,
    /// Owning process.
    pub pid: Pid,
    /// Registers.
    pub regs: Regs,
    /// State.
    pub state: TState,
    /// Blocked-signal mask.
    pub sigmask: u64,
    /// Pending signals.
    pub pending: u64,
    /// Alternate signal stack (sp, size).
    pub altstack: (u64, u64),
    /// `set_tid_address` / CLONE_CHILD_CLEARTID.
    pub clear_tid: u64,
    /// Robust futex list head.
    pub robust: u64,
    /// Deadline of a poll that is being re-run after a wake-up (so waking does not restart its timeout).
    pub resume_deadline: Option<u64>,
}

/// Signal action (`struct kernel_sigaction`).
#[derive(Clone, Copy, Debug, Default)]
pub struct SigAct {
    /// SIG_DFL=0, SIG_IGN=1 or handler address.
    pub handler: u64,
    /// SA_* flags.
    pub flags: u64,
    /// SA_RESTORER trampoline.
    pub restorer: u64,
    /// Extra mask during the handler.
    pub mask: u64,
}

enum FKind {
    Reg(String),
    Gen(Vec<u8>),
    Null,
    Zero,
    Random,
    Tty,
    PipeR(usize),
    PipeW(usize),
    Dir(String),
    Sock(usize),
    EventFd(usize),
    Epoll(usize),
}

struct OpenFile {
    kind: FKind,
    off: u64,
    flags: u32,
}

type FileRef = Rc<RefCell<OpenFile>>;

#[derive(Clone)]
struct FdEnt {
    file: FileRef,
    cloexec: bool,
}

#[derive(Default)]
struct Sock {
    key: Option<String>,
    listening: bool,
    backlog: VecDeque<usize>,
    peer: Option<usize>,
    peer_closed: bool,
    rx: VecDeque<u8>,
    closed: bool,
    udp: Option<usize>,
    /// Handle of a real TCP connection in the network stack.
    tcp: Option<usize>,
    /// Default destination of a connected UDP socket.
    udp_peer: Option<(Ipv4Addr, u16)>,
    datagram: bool,
    rights: VecDeque<Vec<FileRef>>,
    wayland: Option<WaylandServer>,
}

struct Pipe {
    buf: VecDeque<u8>,
    readers: usize,
    writers: usize,
}

/// A Linux process.
#[derive(Clone)]
pub struct Process {
    /// Pid.
    pub pid: Pid,
    /// Parent pid (0 = none).
    pub ppid: Pid,
    /// Address space.
    pub mem: UserMem,
    fds: BTreeMap<i32, FdEnt>,
    /// Working directory.
    pub cwd: String,
    /// Executable path.
    pub exe: String,
    /// argv (for /proc/self/cmdline).
    pub argv: Vec<String>,
    /// Signal actions (index = signo).
    pub acts: Vec<SigAct>,
    /// Threads.
    pub threads: Vec<Pid>,
    /// Wait status once zombie.
    pub exit: Option<i32>,
    /// umask.
    pub umask: u32,
    /// File-backed MAP_SHARED mappings: (start, length, path, file offset) — written back on msync/munmap/exit.
    pub fmaps: Vec<(u64, u64, String, u64)>,
}

/// Outcome of a syscall.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sys {
    /// Value for RAX (already stored in the thread's regs).
    Ret(i64),
    /// Thread is blocked.
    Blocked,
    /// This thread ended.
    ThreadExit,
    /// Whole process ended with this wait-status.
    ProcessExit(i32),
    /// `execve` replaced the image (regs fully rewritten).
    Exec,
    /// `rt_sigreturn` restored regs (RAX is the restored value).
    Restored,
}

/// What `deliver_signal` did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Delivery {
    /// Nothing deliverable.
    None,
    /// Frame pushed; thread will enter the handler.
    Handler(u32),
    /// Default action killed the process (wait status).
    Killed(u32),
}

/// Bridge into the LoOS/Win32 API (`liblooswin`).
pub trait Bridge {
    /// Handle `func = nr - BRIDGE_BASE` for thread `tid`.
    fn call(&mut self, tid: Pid, mem: &mut UserMem, func: u32, a: [u64; 6]) -> i64;
}

/// The Linux subsystem state.
pub struct LinSystem<F: LinFs> {
    /// Filesystem.
    pub fs: F,
    procs: BTreeMap<Pid, Process>,
    threads: BTreeMap<Pid, Thread>,
    pipes: Vec<Pipe>,
    socks: Vec<Sock>,
    next_id: Pid,
    /// Monotonic clock in ns (driven by [`LinSystem::advance`]).
    pub now_ns: u64,
    /// Unix time (s) at `now_ns == 0`; the kernel sets it from the RTC.
    pub epoch_base_s: u64,
    /// Syscall numbers that returned ENOSYS, with how often (a to-do list for the compatibility layer).
    pub missing: BTreeMap<u64, u32>,
    /// Bytes written to the console (fd on /dev/tty).
    pub console_out: Vec<u8>,
    /// When set, reads from the console return EOF instead of blocking (batch runs).
    pub stdin_eof: bool,
    console_in: VecDeque<u8>,
    bridge: Option<Box<dyn Bridge>>,
    rng: u64,
    wakeq: Vec<Pid>,
    deferred: Vec<(Pid, Option<Pid>, u32)>,
    /// Optional LoOS network stack used by AF_INET datagram sockets.
    net: Option<Box<dyn LinNet>>,
    eventfds: Vec<(u64, bool)>,
    epolls: Vec<Vec<(i32, u32, u64)>>,
}

#[path = "sysio.rs"]
mod sysio;

fn err<T>(e: i64) -> Result<T, i64> {
    Err(e)
}

impl<F: LinFs> LinSystem<F> {
    /// New subsystem over `fs`. Pid 1 is reserved.
    pub fn new(fs: F) -> Self {
        Self {
            fs,
            procs: BTreeMap::new(),
            threads: BTreeMap::new(),
            pipes: Vec::new(),
            socks: vec![Sock { key: Some("unix:/run/user/1000/wayland-0".into()), listening: true, ..Sock::default() }],
            next_id: 100,
            now_ns: 0,
            epoch_base_s: EPOCH_BASE_S,
            missing: BTreeMap::new(),
            console_out: Vec::new(),
            stdin_eof: false,
            console_in: VecDeque::new(),
            bridge: None,
            rng: 0x9E37_79B9_7F4A_7C15,
            wakeq: Vec::new(),
            deferred: Vec::new(),
            net: None,
            eventfds: Vec::new(),
            epolls: Vec::new(),
        }
    }

    /// Install the liblooswin bridge.
    pub fn set_bridge(&mut self, b: Box<dyn Bridge>) {
        self.bridge = Some(b);
    }
    /// Attaches the LoOS network to Linux AF_INET sockets.
    pub fn attach_net(&mut self, net: impl LinNet + 'static) {
        self.net = Some(Box::new(net));
    }
    /// True when a network is attached (even if its adapter is absent).
    pub fn net_attached(&self) -> bool {
        self.net.is_some()
    }
    fn stack_mut(&mut self) -> Option<&mut Stack> {
        self.net.as_mut().and_then(|n| n.stack())
    }
    /// True when some thread waits on I/O (so the caller must keep pumping the network instead of declaring deadlock).
    pub fn io_waiters(&self) -> bool {
        self.threads.values().any(|t| matches!(t.state, TState::Blocked { wait: Wait::Poll | Wait::Sock(_), .. }))
    }
    /// Polls the attached network. The kernel calls this from its timer/NIC loop.
    pub fn poll_net(&mut self, now_ms: u64) {
        if let Some(net) = self.net.as_mut() {
            net.pump(now_ms);
        }
        let io: Vec<usize> = self.socks.iter().enumerate().filter_map(|(i, s)| (s.udp.is_some() || s.tcp.is_some()).then_some(i)).collect();
        for i in io {
            self.wake_where(|t| matches!(t.state, TState::Blocked { wait: Wait::Sock(q), .. } if q == i));
        }
        self.wake_pollers();
        self.drain();
    }

    fn wake_pollers(&mut self) {
        self.wake_where(|t| matches!(t.state, TState::Blocked { wait: Wait::Poll, .. }));
    }

    /// Takes frames committed by Wayland clients for compositor presentation.
    pub fn take_wayland_frames(&mut self) -> Vec<WaylandFrame> {
        let mut out = Vec::new();
        for socket in &mut self.socks {
            if let Some(server) = socket.wayland.as_mut() {
                out.extend(server.take_frames());
            }
        }
        out
    }
    /// Process by pid.
    pub fn process(&self, pid: Pid) -> Option<&Process> {
        self.procs.get(&pid)
    }
    /// Mutable process (tests, kernel glue).
    pub fn process_mut(&mut self, pid: Pid) -> Option<&mut Process> {
        self.procs.get_mut(&pid)
    }
    /// All thread ids, ascending.
    pub fn thread_ids(&self) -> Vec<Pid> {
        self.threads.keys().copied().collect()
    }
    /// Earliest blocking deadline (ns) of any thread, if one exists.
    pub fn next_deadline(&self) -> Option<u64> {
        self.threads.values().filter_map(|t| if let TState::Blocked { deadline, .. } = &t.state { *deadline } else { None }).min()
    }
    /// Thread by tid.
    pub fn thread(&self, tid: Pid) -> Option<&Thread> {
        self.threads.get(&tid)
    }
    /// Mutable thread.
    pub fn thread_mut(&mut self, tid: Pid) -> Option<&mut Thread> {
        self.threads.get_mut(&tid)
    }
    /// Feed console input (unblocks readers of /dev/tty).
    pub fn console_feed(&mut self, data: &[u8]) {
        self.console_in.extend(data);
        let w: Vec<Pid> = self.threads.values().filter(|t| matches!(t.state, TState::Blocked { wait: Wait::Stdin, .. })).map(|t| t.tid).collect();
        self.wakeq.extend(w);
        self.drain();
    }
    /// Take and clear console output.
    pub fn console_take(&mut self) -> Vec<u8> {
        core::mem::take(&mut self.console_out)
    }

    fn rand_u64(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        x
    }

    fn alloc_id(&mut self) -> Pid {
        self.next_id += 1;
        self.next_id
    }

    fn new_fd(&mut self, kind: FKind, flags: u32) -> FileRef {
        match &kind {
            FKind::PipeR(i) => self.pipes[*i].readers += 1,
            FKind::PipeW(i) => self.pipes[*i].writers += 1,
            _ => {}
        }
        Rc::new(RefCell::new(OpenFile { kind, off: 0, flags }))
    }

    fn release(&mut self, f: FileRef) {
        if Rc::strong_count(&f) == 1 {
            if let FKind::Sock(i) = f.borrow().kind {
                if let Some(h) = self.socks[i].udp.take() {
                    if let Some(net) = self.stack_mut() {
                        net.udp_close(h);
                    }
                }
                if let Some(h) = self.socks[i].tcp.take() {
                    if let Some(net) = self.stack_mut() {
                        let _ = net.tcp_close(h);
                    }
                }
                self.socks[i].closed = true;
                if let Some(pe) = self.socks[i].peer {
                    self.socks[pe].peer_closed = true;
                    self.wake_where(|t| matches!(t.state, TState::Blocked { wait: Wait::Sock(q), .. } if q == pe));
                }
                self.socks[i].key = None;
            }
            let (r, w) = match f.borrow().kind {
                FKind::PipeR(i) => (Some(i), None),
                FKind::PipeW(i) => (None, Some(i)),
                _ => (None, None),
            };
            if let Some(i) = r {
                self.pipes[i].readers -= 1;
            }
            if let Some(i) = w {
                self.pipes[i].writers -= 1;
            }
            if r.is_some() || w.is_some() {
                let i = r.or(w).unwrap();
                self.wake_where(|t| matches!(t.state, TState::Blocked { wait: Wait::Pipe(p), .. } if p == i));
            }
        }
    }

    fn wake_where(&mut self, pred: impl Fn(&Thread) -> bool) {
        let v: Vec<Pid> = self.threads.values().filter(|t| pred(t)).map(|t| t.tid).collect();
        self.wakeq.extend(v);
    }

    /// Create a process running `path` with fds 0-2 on the console.
    pub fn spawn(&mut self, path: &str, argv: &[String], envp: &[String]) -> Result<Pid, i64> {
        let mut mem = UserMem::new();
        let seed = self.rand_u64();
        let l = loader::load(&mut self.fs, &mut mem, path, argv, envp, seed)?;
        let pid = self.alloc_id();
        let tty = self.new_fd(FKind::Tty, 2);
        let mut fds = BTreeMap::new();
        for i in 0..3 {
            fds.insert(i, FdEnt { file: tty.clone(), cloexec: false });
        }
        let regs = Regs { rip: l.entry, rsp: l.sp, rflags: 0x202, ..Regs::default() };
        let mut acts = vec![SigAct::default(); 65];
        acts[0] = SigAct::default();
        self.procs.insert(
            pid,
            Process { pid, ppid: 0, mem, fds, cwd: "/".into(), exe: l.exe, argv: argv.to_vec(), acts, threads: vec![pid], exit: None, umask: 0o022, fmaps: Vec::new() },
        );
        self.threads.insert(pid, Thread { tid: pid, pid, regs, state: TState::Runnable, sigmask: 0, pending: 0, altstack: (0, 0), clear_tid: 0, robust: 0, resume_deadline: None });
        Ok(pid)
    }

    /// Advance the clock: fires sleep/futex timeouts.
    pub fn advance(&mut self, ns: u64) {
        self.now_ns += ns;
        let now = self.now_ns;
        let due: Vec<(Pid, Wait)> = self
            .threads
            .values()
            .filter_map(|t| match &t.state {
                TState::Blocked { wait, deadline: Some(d), .. } if *d <= now => Some((t.tid, *wait)),
                _ => None,
            })
            .collect();
        for (tid, w) in due {
            let t = self.threads.get_mut(&tid).unwrap();
            t.state = TState::Runnable;
            t.regs.rax = if matches!(w, Wait::Sleep | Wait::Poll) { 0 } else { (-ETIMEDOUT) as u64 };
        }
    }

    fn drain(&mut self) {
        while let Some(tid) = self.wakeq.pop() {
            let Some(t) = self.threads.get_mut(&tid) else { continue };
            if let TState::Blocked { restart, wait, deadline } = t.state.clone() {
                t.state = TState::Runnable;
                if wait == Wait::Poll {
                    t.resume_deadline = deadline;
                }
                match restart {
                    Some((nr, a)) => {
                        self.syscall_inner(tid, nr, a);
                    }
                    None => t.regs.rax = 0,
                }
            }
        }
        let d = core::mem::take(&mut self.deferred);
        for (pid, tid, sig) in d {
            self.send_signal(pid, tid, sig);
        }
    }

    /// Queue signal `sig` for `pid` (or a specific `tid`). Wakes interruptible sleepers.
    pub fn send_signal(&mut self, pid: Pid, tid: Option<Pid>, sig: u32) -> bool {
        if sig == 0 || sig > 64 {
            return false;
        }
        let Some(p) = self.procs.get(&pid) else { return false };
        if p.exit.is_some() {
            return false;
        }
        let target = match tid {
            Some(t) if p.threads.contains(&t) => t,
            Some(_) => return false,
            None => *p.threads.iter().find(|t| self.threads[t].sigmask & (1 << (sig - 1)) == 0).unwrap_or(&p.threads[0]),
        };
        let ignored = {
            let a = p.acts[sig as usize];
            a.handler == 1 || (a.handler == 0 && matches!(sig, 17 | 18 | 23 | 28 | 19..=22))
        };
        let t = self.threads.get_mut(&target).unwrap();
        if ignored && sig != 9 {
            return true;
        }
        t.pending |= 1 << (sig - 1);
        if t.sigmask & (1 << (sig - 1)) == 0 || sig == 9 {
            if let TState::Blocked { .. } = t.state {
                t.state = TState::Runnable;
                t.regs.rax = (-EINTR) as u64;
            }
        }
        true
    }

    /// Deliver one pending, unblocked signal to `tid` (call when the thread returns to user mode).
    pub fn deliver_signal(&mut self, tid: Pid) -> Delivery {
        let Some(t) = self.threads.get(&tid) else { return Delivery::None };
        if t.state != TState::Runnable {
            return Delivery::None;
        }
        let ready = t.pending & !t.sigmask | (t.pending & (1 << 8));
        if ready == 0 {
            return Delivery::None;
        }
        let sig = ready.trailing_zeros() + 1;
        let pid = t.pid;
        let act = self.procs[&pid].acts[sig as usize];
        self.threads.get_mut(&tid).unwrap().pending &= !(1 << (sig - 1));
        if sig != 9 && act.handler == 1 {
            return Delivery::None;
        }
        if sig == 9 || act.handler == 0 {
            if matches!(sig, 17 | 18 | 23 | 28 | 19..=22) {
                return Delivery::None;
            }
            self.terminate_process(pid, sig as i32);
            return Delivery::Killed(sig);
        }
        let mut p = self.procs.remove(&pid).unwrap();
        let t = self.threads.get_mut(&tid).unwrap();
        let r = t.regs;
        let on_alt = act.flags & 0x0800_0000 != 0 && t.altstack.1 != 0;
        let mut sp = if on_alt { t.altstack.0 + t.altstack.1 } else { r.rsp.wrapping_sub(128) };
        sp = (sp.wrapping_sub(560)) & !15;
        let frame = sp.wrapping_sub(8);
        let mut buf = vec![0u8; 560];
        let put = |b: &mut Vec<u8>, off: usize, v: u64| b[off..off + 8].copy_from_slice(&v.to_le_bytes());
        put(&mut buf, 0, act.restorer);
        // ucontext @8: flags, link, stack_t(sp, flags, size)
        put(&mut buf, 8 + 16, t.altstack.0);
        put(&mut buf, 8 + 32, t.altstack.1);
        let mc = 8 + 40;
        let order = [r.r8, r.r9, r.r10, r.r11, r.r12, r.r13, r.r14, r.r15, r.rdi, r.rsi, r.rbp, r.rbx, r.rdx, r.rax, r.rcx, r.rsp, r.rip, r.rflags];
        for (i, v) in order.iter().enumerate() {
            put(&mut buf, mc + i * 8, *v);
        }
        put(&mut buf, 8 + 296 - 8 + 8, t.sigmask); // uc_sigmask @ mcontext+256
        // siginfo @ 8+424: si_signo, errno, code
        buf[432..436].copy_from_slice(&sig.to_le_bytes());
        let ok = p.mem.write(frame, &buf).is_ok();
        if !ok {
            self.procs.insert(pid, p);
            self.terminate_process(pid, 11);
            return Delivery::Killed(11);
        }
        t.sigmask |= act.mask | if act.flags & 0x4000_0000 != 0 { 0 } else { 1 << (sig - 1) };
        t.regs.rsp = frame;
        t.regs.rip = act.handler;
        t.regs.rdi = sig as u64;
        t.regs.rsi = frame + 432;
        t.regs.rdx = frame + 8;
        t.regs.rax = 0;
        self.procs.insert(pid, p);
        Delivery::Handler(sig)
    }

    fn terminate_process(&mut self, pid: Pid, sig: i32) {
        let status = sig;
        self.exit_process(pid, status);
    }

    fn exit_process(&mut self, pid: Pid, status: i32) {
        let Some(mut p) = self.procs.remove(&pid) else { return };
        for t in p.threads.drain(..) {
            if let Some(th) = self.threads.remove(&t) {
                if th.clear_tid != 0 && p.mem.write_u32(th.clear_tid, 0).is_ok() {
                    self.futex_wake(pid, th.clear_tid, 1);
                }
            }
        }
        let fds: Vec<FdEnt> = core::mem::take(&mut p.fds).into_values().collect();
        for f in fds {
            self.release(f.file);
        }
        p.exit = Some(status);
        let ppid = p.ppid;
        // Orphans are reparented to nobody and auto-reaped.
        for c in self.procs.values_mut().filter(|c| c.ppid == pid) {
            c.ppid = 0;
        }
        let reap = ppid == 0 || !self.procs.contains_key(&ppid);
        if !reap {
            self.procs.insert(pid, p);
            self.deferred.push((ppid, None, 17));
            self.wake_where(|t| t.pid == ppid && matches!(t.state, TState::Blocked { wait: Wait::Child, .. }));
        }
        // Dead zombies whose parent is gone.
        let dead: Vec<Pid> = self.procs.values().filter(|c| c.exit.is_some() && c.ppid == 0).map(|c| c.pid).collect();
        for d in dead {
            self.procs.remove(&d);
        }
    }

    fn futex_wake(&mut self, pid: Pid, addr: u64, n: usize) -> usize {
        let mut ws: Vec<(u64, Pid)> = self
            .threads
            .values()
            .filter(|t| matches!(t.state, TState::Blocked { wait: Wait::Futex(p, a), .. } if p == pid && a == addr))
            .map(|t| (t.tid as u64, t.tid))
            .collect();
        ws.sort();
        let mut woken = 0;
        for (_, tid) in ws.into_iter().take(n) {
            let t = self.threads.get_mut(&tid).unwrap();
            t.state = TState::Runnable;
            t.regs.rax = 0;
            woken += 1;
        }
        woken
    }

    /// Execute a syscall for `tid`. The result is also written to the thread's RAX.
    pub fn syscall(&mut self, tid: Pid, nr: u64, a: [u64; 6]) -> Sys {
        let r = self.syscall_inner(tid, nr, a);
        self.drain();
        self.wake_pollers();
        self.drain();
        r
    }

    fn syscall_inner(&mut self, tid: Pid, nr: u64, a: [u64; 6]) -> Sys {
        let Some(pid) = self.threads.get(&tid).map(|t| t.pid) else { return Sys::Ret(-ESRCH) };
        let Some(mut p) = self.procs.remove(&pid) else { return Sys::Ret(-ESRCH) };
        let r = if nr >= BRIDGE_BASE && nr < BRIDGE_BASE + 0x1_0000 {
            match self.bridge.as_mut() {
                Some(b) => Ok(Sys::Ret(b.call(tid, &mut p.mem, (nr - BRIDGE_BASE) as u32, a))),
                None => Ok(Sys::Ret(-ENOSYS)),
            }
        } else {
            self.dispatch(&mut p, tid, nr, a)
        };
        let r = r.unwrap_or_else(|e| Sys::Ret(-e));
        if matches!(r, Sys::Ret(v) if v == -ENOSYS) {
            *self.missing.entry(nr).or_insert(0) += 1;
        }
        // exit_group may have removed the process from the map already.
        if !self.procs.contains_key(&pid) && !matches!(r, Sys::ProcessExit(_)) {
            self.procs.insert(pid, p);
        }
        if let Sys::Ret(v) = r {
            if let Some(t) = self.threads.get_mut(&tid) {
                t.regs.rax = v as u64;
            }
        }
        r
    }

    // ----- helpers -----

    fn block(&mut self, tid: Pid, wait: Wait, nr: u64, a: [u64; 6], deadline: Option<u64>, restart: bool) -> Sys {
        if let Some(t) = self.threads.get_mut(&tid) {
            t.state = TState::Blocked { wait, restart: restart.then_some((nr, a)), deadline };
        }
        Sys::Blocked
    }

    fn path_for(&mut self, p: &Process, dirfd: i32, path: &str, follow: bool) -> Result<String, i64> {
        if path.is_empty() {
            return err(ENOENT);
        }
        let base = if path.starts_with('/') || dirfd == AT_FDCWD {
            p.cwd.clone()
        } else {
            match &p.fds.get(&dirfd).ok_or(EBADF)?.file.borrow().kind {
                FKind::Dir(d) => d.clone(),
                _ => return err(ENOTDIR),
            }
        };
        if path.starts_with("/proc/self/") || path.starts_with("/dev/") || path.starts_with("/proc/") {
            return Ok(normalize(&base, path));
        }
        resolve(&mut self.fs, &base, path, follow)
    }

    fn gen_proc(&self, p: &Process, path: &str) -> Option<Vec<u8>> {
        let t = match path {
            "/proc/self/maps" => p.mem.maps(),
            "/proc/self/cmdline" => p.argv.iter().map(|a| format!("{a}\0")).collect(),
            "/proc/self/comm" => format!("{}\n", p.exe.rsplit('/').next().unwrap_or("")),
            "/proc/self/status" => format!("Name:\t{}\nState:\tR (running)\nTgid:\t{}\nPid:\t{}\nPPid:\t{}\nUid:\t0\t0\t0\t0\nGid:\t0\t0\t0\t0\nThreads:\t{}\nVmSize:\t{} kB\n", p.exe.rsplit('/').next().unwrap_or(""), p.pid, p.pid, p.ppid, p.threads.len(), p.mem.mapped() / 1024),
            "/proc/cpuinfo" => "processor\t: 0\nvendor_id\t: LoOS\nmodel name\t: x86-64 (LoOS native)\ncpu MHz\t\t: 2000.000\nflags\t\t: fpu sse sse2 syscall nx lm\n\n".into(),
            "/proc/meminfo" => "MemTotal:        4194304 kB\nMemFree:         2097152 kB\nMemAvailable:    2097152 kB\n".into(),
            "/proc/version" => "Linux version 6.1.0-loos (loos@loos) #1 SMP LoOS\n".into(),
            "/proc/self/stat" => format!("{} ({}) R {} 0 0 0 0 0 0 0 0 0 0 0 0 0 20 0 {} 0 0\n", p.pid, p.exe.rsplit('/').next().unwrap_or(""), p.ppid, p.threads.len()),
            "/proc/sys/kernel/random/uuid" => "00000000-0000-4000-8000-000000000000\n".into(),
            _ => return None,
        };
        Some(t.into_bytes())
    }

    fn stat_abs(&mut self, p: &Process, path: &str) -> Result<Stat, i64> {
        let dev = |ino| Stat { ino, kind: Kind::Char, mode: 0o666, size: 0, nlink: 1, uid: 0, gid: 0, mtime: 0 };
        match path {
            "/dev/null" => return Ok(dev(3)),
            "/dev/zero" => return Ok(dev(5)),
            "/dev/urandom" | "/dev/random" => return Ok(dev(9)),
            "/dev/tty" | "/dev/console" => return Ok(dev(11)),
            "/run/user/1000/wayland-0" => return Ok(Stat { ino: 17, kind: Kind::Fifo, mode: 0o777, size: 0, nlink: 1, uid: 1000, gid: 1000, mtime: 0 }),
            _ => {}
        }
        if let Some(d) = self.gen_proc(p, path) {
            return Ok(Stat { ino: 7, kind: Kind::File, mode: 0o444, size: d.len() as u64, nlink: 1, uid: 0, gid: 0, mtime: 0 });
        }
        if path == "/proc/self/exe" {
            return Ok(Stat { ino: 8, kind: Kind::Symlink, mode: 0o777, size: p.exe.len() as u64, nlink: 1, uid: 0, gid: 0, mtime: 0 });
        }
        self.fs.lstat(path)
    }

    fn write_stat(&self, mem: &mut UserMem, addr: u64, st: &Stat) -> Result<(), i64> {
        let mut b = [0u8; 144];
        let w = |b: &mut [u8], o: usize, v: u64| b[o..o + 8].copy_from_slice(&v.to_le_bytes());
        w(&mut b, 0, 1);
        w(&mut b, 8, st.ino);
        w(&mut b, 16, st.nlink);
        b[24..28].copy_from_slice(&st.st_mode().to_le_bytes());
        b[28..32].copy_from_slice(&st.uid.to_le_bytes());
        b[32..36].copy_from_slice(&st.gid.to_le_bytes());
        w(&mut b, 48, st.size);
        w(&mut b, 56, 4096);
        w(&mut b, 64, st.size.div_ceil(512));
        for o in [72, 88, 104] {
            w(&mut b, o, st.mtime as u64);
        }
        mem.write(addr, &b)
    }

    fn write_statx(&self, mem: &mut UserMem, addr: u64, st: &Stat) -> Result<(), i64> {
        let mut b = [0u8; 256];
        b[0..4].copy_from_slice(&0x7ffu32.to_le_bytes());
        b[4..8].copy_from_slice(&4096u32.to_le_bytes());
        b[16..20].copy_from_slice(&(st.nlink as u32).to_le_bytes());
        b[20..24].copy_from_slice(&st.uid.to_le_bytes());
        b[24..28].copy_from_slice(&st.gid.to_le_bytes());
        b[28..30].copy_from_slice(&(st.st_mode() as u16).to_le_bytes());
        b[32..40].copy_from_slice(&st.ino.to_le_bytes());
        b[40..48].copy_from_slice(&st.size.to_le_bytes());
        b[48..56].copy_from_slice(&st.size.div_ceil(512).to_le_bytes());
        for o in [64, 80, 96, 112] {
            b[o..o + 8].copy_from_slice(&st.mtime.to_le_bytes());
        }
        mem.write(addr, &b)
    }

    fn open_path(&mut self, p: &mut Process, abs: &str, flags: u32, mode: u32) -> Result<FileRef, i64> {
        let acc = flags & 3;
        let kind = match abs {
            "/dev/null" => Some(FKind::Null),
            "/dev/zero" => Some(FKind::Zero),
            "/dev/urandom" | "/dev/random" => Some(FKind::Random),
            "/dev/tty" | "/dev/console" => Some(FKind::Tty),
            _ => self.gen_proc(p, abs).map(FKind::Gen),
        };
        if let Some(k) = kind {
            return Ok(self.new_fd(k, flags));
        }
        let st = match self.fs.lstat(abs) {
            Ok(st) => {
                if flags & 0o200 != 0 && flags & 0o100 != 0 {
                    return err(EEXIST);
                }
                st
            }
            Err(e) if e == ENOENT && flags & 0o100 != 0 => {
                self.fs.create(abs, mode & !p.umask & 0o7777)?;
                self.fs.lstat(abs)?
            }
            Err(e) => return Err(e),
        };
        match st.kind {
            Kind::Dir => {
                if acc != 0 {
                    return err(EISDIR);
                }
                Ok(self.new_fd(FKind::Dir(abs.to_string()), flags))
            }
            Kind::File => {
                if flags & 0o200000 != 0 {
                    return err(ENOTDIR);
                }
                if flags & 0o1000 != 0 && acc != 0 {
                    self.fs.truncate(abs, 0)?;
                }
                Ok(self.new_fd(FKind::Reg(abs.to_string()), flags))
            }
            Kind::Symlink => err(ELOOP),
            _ => err(ENODEV),
        }
    }

    fn fd(&self, p: &Process, fd: i32) -> Result<FileRef, i64> {
        p.fds.get(&fd).map(|f| f.file.clone()).ok_or(EBADF)
    }

    fn alloc_fd(&self, p: &Process, min: i32) -> Result<i32, i64> {
        (min..1024).find(|i| !p.fds.contains_key(i)).ok_or(EMFILE)
    }

    fn do_read(&mut self, p: &mut Process, tid: Pid, nr: u64, a: [u64; 6], f: &FileRef, buf: u64, count: u64, at: Option<u64>) -> Result<Sys, i64> {
        let count = count.min(1 << 20) as usize;
        let mut data = vec![0u8; count];
        let flags = f.borrow().flags;
        let sock = if let FKind::Sock(i) = f.borrow().kind { Some(i) } else { None };
        if let Some(i) = sock {
            if let Some(h) = self.socks[i].udp {
                let Some(net) = self.stack_mut() else { return err(ENOTCONN) };
                let Some((_, _, d)) = net.udp_recv(h) else {
                    if flags & 0o4000 != 0 {
                        return err(EAGAIN);
                    }
                    return Ok(self.block(tid, Wait::Sock(i), nr, a, None, true));
                };
                let n = d.len().min(count);
                p.mem.write(buf, &d[..n])?;
                return Ok(Sys::Ret(n as i64));
            }
            if let Some(h) = self.socks[i].tcp {
                let Some(net) = self.stack_mut() else { return err(ENOTCONN) };
                let n = net.tcp_recv(h, &mut data).map_err(|_| ECONNRESET)?;
                if n > 0 {
                    p.mem.write(buf, &data[..n])?;
                    return Ok(Sys::Ret(n as i64));
                }
                if net.tcp_eof(h) {
                    return Ok(Sys::Ret(0));
                }
                if net.tcp_failure(h).is_some() || net.tcp_state(h) == Some(State::Closed) {
                    return err(ECONNRESET);
                }
                if flags & 0o4000 != 0 {
                    return err(EAGAIN);
                }
                return Ok(self.block(tid, Wait::Sock(i), nr, a, None, true));
            }
            let sk = &mut self.socks[i];
            if sk.rx.is_empty() {
                if sk.peer_closed || sk.peer.is_none() {
                    return Ok(Sys::Ret(0));
                }
                if flags & 0o4000 != 0 {
                    return err(EAGAIN);
                }
                return Ok(self.block(tid, Wait::Sock(i), nr, a, None, true));
            }
            let n = count.min(sk.rx.len());
            for b in data.iter_mut().take(n) {
                *b = sk.rx.pop_front().unwrap();
            }
            p.mem.write(buf, &data[..n])?;
            return Ok(Sys::Ret(n as i64));
        }
        let n = {
            let kind_off = {
                let o = f.borrow();
                (o.off, matches!(o.kind, FKind::Reg(_)))
            };
            let _ = kind_off;
            let mut fb = f.borrow_mut();
            let off = at.unwrap_or(fb.off);
            let n = match &fb.kind {
                FKind::Reg(path) => {
                    let path = path.clone();
                    drop(fb);
                    let n = self.fs.read(&path, off, &mut data)?;
                    fb = f.borrow_mut();
                    n
                }
                FKind::Gen(d) => {
                    let o = (off as usize).min(d.len());
                    let n = count.min(d.len() - o);
                    data[..n].copy_from_slice(&d[o..o + n]);
                    n
                }
                FKind::Null => 0,
                FKind::Zero => count,
                FKind::Random => {
                    drop(fb);
                    for c in data.chunks_mut(8) {
                        let r = self.rand_u64().to_le_bytes();
                        c.copy_from_slice(&r[..c.len()]);
                    }
                    fb = f.borrow_mut();
                    count
                }
                FKind::Tty => {
                    drop(fb);
                    if self.console_in.is_empty() {
                        if self.stdin_eof {
                            return Ok(Sys::Ret(0));
                        }
                        if flags & 0o4000 != 0 {
                            return err(EAGAIN);
                        }
                        return Ok(self.block(tid, Wait::Stdin, nr, a, None, true));
                    }
                    let n = count.min(self.console_in.len());
                    for b in data.iter_mut().take(n) {
                        *b = self.console_in.pop_front().unwrap();
                    }
                    fb = f.borrow_mut();
                    n
                }
                FKind::PipeR(i) => {
                    let i = *i;
                    drop(fb);
                    let pipe = &mut self.pipes[i];
                    if pipe.buf.is_empty() {
                        if pipe.writers == 0 {
                            return Ok(Sys::Ret(0));
                        }
                        if flags & 0o4000 != 0 {
                            return err(EAGAIN);
                        }
                        return Ok(self.block(tid, Wait::Pipe(i), nr, a, None, true));
                    }
                    let n = count.min(pipe.buf.len());
                    for b in data.iter_mut().take(n) {
                        *b = pipe.buf.pop_front().unwrap();
                    }
                    self.wake_where(|t| matches!(t.state, TState::Blocked { wait: Wait::Pipe(q), .. } if q == i));
                    fb = f.borrow_mut();
                    n
                }
                FKind::EventFd(i) => {
                    let i = *i;
                    drop(fb);
                    if count < 8 {
                        return err(EINVAL);
                    }
                    let (v, sem) = self.eventfds[i];
                    if v == 0 {
                        if flags & 0o4000 != 0 {
                            return err(EAGAIN);
                        }
                        return Ok(self.block(tid, Wait::Event(i), nr, a, None, true));
                    }
                    let out = if sem { 1 } else { v };
                    self.eventfds[i].0 = v - out;
                    data[..8].copy_from_slice(&out.to_le_bytes());
                    fb = f.borrow_mut();
                    8
                }
                FKind::PipeW(_) | FKind::Sock(_) | FKind::Epoll(_) => return err(EBADF),
                FKind::Dir(_) => return err(EISDIR),
            };
            if at.is_none() && matches!(fb.kind, FKind::Reg(_) | FKind::Gen(_)) {
                fb.off += n as u64;
            }
            n
        };
        p.mem.write(buf, &data[..n])?;
        Ok(Sys::Ret(n as i64))
    }

    #[allow(clippy::too_many_arguments)]
    fn do_write(&mut self, p: &mut Process, tid: Pid, nr: u64, a: [u64; 6], f: &FileRef, buf: u64, count: u64, at: Option<u64>) -> Result<Sys, i64> {
        let count = count.min(1 << 20) as usize;
        let data = p.mem.read_vec(buf, count)?;
        let flags = f.borrow().flags;
        let sock = if let FKind::Sock(i) = f.borrow().kind { Some(i) } else { None };
        if let Some(i) = sock {
            if let Some(h) = self.socks[i].udp {
                let (dst, port) = self.socks[i].udp_peer.ok_or(EDESTADDRREQ)?;
                let Some(net) = self.stack_mut() else { return err(ENOTCONN) };
                net.udp_send(h, IpAddr::V4(dst), port, &data).map_err(|_| 101i64)?;
                return Ok(Sys::Ret(data.len() as i64));
            }
            if let Some(h) = self.socks[i].tcp {
                let Some(net) = self.stack_mut() else { return err(ENOTCONN) };
                if net.tcp_failure(h).is_some() || net.tcp_state(h) == Some(State::Closed) {
                    self.deferred.push((p.pid, Some(tid), 13));
                    return err(EPIPE);
                }
                let n = net.tcp_send(h, &data).map_err(|_| EPIPE)?;
                if n == 0 && !data.is_empty() {
                    if flags & 0o4000 != 0 {
                        return err(EAGAIN);
                    }
                    return Ok(self.block(tid, Wait::Sock(i), nr, a, None, true));
                }
                return Ok(Sys::Ret(n as i64));
            }
            let Some(pe) = self.socks[i].peer else { return err(ENOTCONN) };
            if self.socks[pe].closed || self.socks[i].peer_closed {
                self.deferred.push((p.pid, Some(tid), 13));
                return err(EPIPE);
            }
            let room = PIPE_CAP.saturating_sub(self.socks[pe].rx.len());
            if room == 0 {
                if flags & 0o4000 != 0 {
                    return err(EAGAIN);
                }
                return Ok(self.block(tid, Wait::Sock(pe), nr, a, None, true));
            }
            let n = room.min(data.len());
            let wayland_events = if let Some(server) = self.socks[pe].wayland.as_mut() {
                server.receive(&data[..n]).map_err(|_| EPROTO)?;
                Some(server.take_events())
            } else {
                None
            };
            if let Some(events) = wayland_events {
                self.socks[i].rx.extend(events);
                self.wake_where(|t| matches!(t.state, TState::Blocked { wait: Wait::Sock(q), .. } if q == i));
            } else {
                self.socks[pe].rx.extend(&data[..n]);
                self.wake_where(|t| matches!(t.state, TState::Blocked { wait: Wait::Sock(q), .. } if q == pe));
            }
            return Ok(Sys::Ret(n as i64));
        }
        let (off, kind) = {
            let o = f.borrow();
            (o.off, match &o.kind {
                FKind::Reg(path) => Some(Ok(path.clone())),
                FKind::PipeW(i) => Some(Err(*i)),
                FKind::Tty | FKind::Null | FKind::Zero | FKind::Random => None,
                FKind::EventFd(i) => {
                    let i = *i;
                    drop(o);
                    if data.len() < 8 {
                        return err(EINVAL);
                    }
                    let add = u64::from_le_bytes(data[..8].try_into().unwrap());
                    if add == u64::MAX || self.eventfds[i].0.checked_add(add).is_none_or(|v| v >= u64::MAX) {
                        if flags & 0o4000 != 0 {
                            return err(EAGAIN);
                        }
                        return Ok(self.block(tid, Wait::Event(i), nr, a, None, true));
                    }
                    self.eventfds[i].0 += add;
                    self.wake_where(|t| matches!(t.state, TState::Blocked { wait: Wait::Event(q), .. } if q == i));
                    return Ok(Sys::Ret(8));
                }
                _ => return err(EBADF),
            })
        };
        let n = match kind {
            Some(Ok(path)) => {
                if flags & 3 == 0 {
                    return err(EBADF);
                }
                let o = if flags & 0o2000 != 0 { self.fs.lstat(&path)?.size } else { at.unwrap_or(off) };
                let n = self.fs.write(&path, o, &data)?;
                if at.is_none() {
                    f.borrow_mut().off = o + n as u64;
                }
                n
            }
            Some(Err(i)) => {
                if self.pipes[i].readers == 0 {
                    self.deferred.push((p.pid, Some(tid), 13));
                    return err(EPIPE);
                }
                let room = PIPE_CAP - self.pipes[i].buf.len();
                if room == 0 {
                    if flags & 0o4000 != 0 {
                        return err(EAGAIN);
                    }
                    return Ok(self.block(tid, Wait::Pipe(i), nr, a, None, true));
                }
                let n = room.min(data.len());
                self.pipes[i].buf.extend(&data[..n]);
                self.wake_where(|t| matches!(t.state, TState::Blocked { wait: Wait::Pipe(q), .. } if q == i));
                n
            }
            None => {
                if matches!(f.borrow().kind, FKind::Tty) {
                    self.console_out.extend(&data);
                }
                data.len()
            }
        };
        Ok(Sys::Ret(n as i64))
    }

    fn read_strv(mem: &mut UserMem, mut addr: u64) -> Result<Vec<String>, i64> {
        let mut v = Vec::new();
        if addr == 0 {
            return Ok(v);
        }
        loop {
            let p = mem.read_u64(addr)?;
            if p == 0 {
                break;
            }
            if v.len() > 8192 {
                return err(7);
            }
            v.push(mem.read_cstr(p, 128 * 1024)?);
            addr += 8;
        }
        Ok(v)
    }

    fn timespec(mem: &mut UserMem, addr: u64) -> Result<u64, i64> {
        let s = mem.read_u64(addr)? as i64;
        let n = mem.read_u64(addr + 8)? as i64;
        if s < 0 || !(0..1_000_000_000).contains(&n) {
            return err(EINVAL);
        }
        Ok((s as u64).saturating_mul(1_000_000_000).saturating_add(n as u64))
    }

    fn dispatch(&mut self, p: &mut Process, tid: Pid, nr: u64, a: [u64; 6]) -> Result<Sys, i64> {
        let ok = |v: i64| Ok(Sys::Ret(v));
        match nr {
            0 => {
                let f = self.fd(p, a[0] as i32)?;
                self.do_read(p, tid, nr, a, &f, a[1], a[2], None)
            }
            1 => {
                let f = self.fd(p, a[0] as i32)?;
                self.do_write(p, tid, nr, a, &f, a[1], a[2], None)
            }
            17 | 18 => {
                // pread64 / pwrite64
                let f = self.fd(p, a[0] as i32)?;
                if matches!(f.borrow().kind, FKind::PipeR(_) | FKind::PipeW(_) | FKind::Tty) {
                    return err(ESPIPE);
                }
                if nr == 17 {
                    self.do_read(p, tid, nr, a, &f, a[1], a[2], Some(a[3]))
                } else {
                    self.do_write(p, tid, nr, a, &f, a[1], a[2], Some(a[3]))
                }
            }
            46 | 47 => self.msg_socket(p, tid, nr, a),
            19 | 20 => {
                // readv / writev
                let f = self.fd(p, a[0] as i32)?;
                if a[2] > 1024 {
                    return err(EINVAL);
                }
                let mut total = 0i64;
                for i in 0..a[2] {
                    let base = p.mem.read_u64(a[1] + i * 16)?;
                    let len = p.mem.read_u64(a[1] + i * 16 + 8)?;
                    let r = if nr == 19 { self.do_read(p, tid, nr, a, &f, base, len, None)? } else { self.do_write(p, tid, nr, a, &f, base, len, None)? };
                    match r {
                        Sys::Ret(n) if n >= 0 => {
                            total += n;
                            if (n as u64) < len {
                                break;
                            }
                        }
                        other if total == 0 => return Ok(other),
                        _ => break,
                    }
                }
                ok(total)
            }
            2 | 257 | 437 => {
                // open / openat
                let (dirfd, pp, fl, mode) = if nr == 2 { (AT_FDCWD, a[0], a[1], a[2]) } else { (a[0] as i32, a[1], a[2], a[3]) };
                let path = p.mem.read_cstr(pp, 4096)?;
                let fl = fl as u32;
                let abs = self.path_for(p, dirfd, &path, fl & 0o400000 == 0)?;
                let f = self.open_path(p, &abs, fl, mode as u32)?;
                let fd = self.alloc_fd(p, 0)?;
                p.fds.insert(fd, FdEnt { file: f, cloexec: fl & 0o2000000 != 0 });
                ok(fd as i64)
            }
            3 => {
                let e = p.fds.remove(&(a[0] as i32)).ok_or(EBADF)?;
                self.release(e.file);
                ok(0)
            }
            8 => {
                let f = self.fd(p, a[0] as i32)?;
                let size = match &f.borrow().kind {
                    FKind::Reg(path) => Some(self.fs.lstat(path)?.size),
                    FKind::Gen(d) => Some(d.len() as u64),
                    FKind::Dir(_) => Some(0),
                    FKind::Null | FKind::Zero | FKind::Random => return ok(0),
                    _ => None,
                };
                let size = size.ok_or(ESPIPE)? as i64;
                let mut fb = f.borrow_mut();
                let base = match a[2] {
                    0 => 0,
                    1 => fb.off as i64,
                    2 => size,
                    _ => return err(EINVAL),
                };
                let n = base.checked_add(a[1] as i64).filter(|&n| n >= 0).ok_or(EINVAL)?;
                fb.off = n as u64;
                ok(n)
            }
            4 | 5 | 6 | 262 | 332 => {
                // stat / fstat / lstat / newfstatat / statx
                let (st, out) = match nr {
                    5 => {
                        let f = self.fd(p, a[0] as i32)?;
                        let k = match &f.borrow().kind {
                            FKind::Reg(x) | FKind::Dir(x) => Ok(x.clone()),
                            FKind::Gen(d) => Err(Stat { ino: 7, kind: Kind::File, mode: 0o444, size: d.len() as u64, nlink: 1, uid: 0, gid: 0, mtime: 0 }),
                            FKind::PipeR(_) | FKind::PipeW(_) | FKind::Sock(_) | FKind::EventFd(_) | FKind::Epoll(_) => Err(Stat { ino: 13, kind: Kind::Fifo, mode: 0o600, size: 0, nlink: 1, uid: 0, gid: 0, mtime: 0 }),
                            _ => Err(Stat { ino: 11, kind: Kind::Char, mode: 0o620, size: 0, nlink: 1, uid: 0, gid: 0, mtime: 0 }),
                        };
                        (match k {
                            Ok(path) => self.fs.lstat(&path)?,
                            Err(s) => s,
                        }, a[1])
                    }
                    4 | 6 => {
                        let path = p.mem.read_cstr(a[0], 4096)?;
                        let abs = self.path_for(p, AT_FDCWD, &path, nr == 4)?;
                        (self.stat_abs(p, &abs)?, a[1])
                    }
                    _ => {
                        let (pp, fl, out) = if nr == 262 { (a[1], a[3], a[2]) } else { (a[1], a[2], a[4]) };
                        let path = p.mem.read_cstr(pp, 4096)?;
                        let empty = fl & 0x1000 != 0;
                        let st = if path.is_empty() && empty {
                            let f = self.fd(p, a[0] as i32)?;
                            let path = match &f.borrow().kind {
                                FKind::Reg(x) | FKind::Dir(x) => x.clone(),
                                _ => return err(ENOSYS),
                            };
                            self.fs.lstat(&path)?
                        } else {
                            let abs = self.path_for(p, a[0] as i32, &path, fl & 0x100 == 0)?;
                            self.stat_abs(p, &abs)?
                        };
                        (st, out)
                    }
                };
                if nr == 332 {
                    self.write_statx(&mut p.mem, out, &st)?;
                } else {
                    self.write_stat(&mut p.mem, out, &st)?;
                }
                ok(0)
            }
            9 => {
                // mmap
                let (addr, len, prot, flags, fd, off) = (a[0], a[1], a[2] as u32, a[3] as u32, a[4] as i32, a[5]);
                if len == 0 || off % PAGE != 0 || prot & !7 != 0 {
                    return err(EINVAL);
                }
                if flags & 3 == 0 || flags & 3 == 3 {
                    return err(EINVAL);
                }
                let mut init = Vec::new();
                let mut name = String::new();
                let mut file_backing: Option<(String, u64)> = None;
                if flags & 0x20 == 0 {
                    let f = self.fd(p, fd)?;
                    let want = len.min(MEM_CAP) as usize;
                    init = vec![0u8; want];
                    let fb = f.borrow();
                    let n = match &fb.kind {
                        FKind::Reg(path) => {
                            name = path.clone();
                            file_backing = Some((path.clone(), off));
                            let path = path.clone();
                            drop(fb);
                            self.fs.read(&path, off, &mut init)?
                        }
                        FKind::Gen(d) => {
                            let o = (off as usize).min(d.len());
                            let n = want.min(d.len() - o);
                            init[..n].copy_from_slice(&d[o..o + n]);
                            n
                        }
                        _ => return err(ENODEV),
                    };
                    init.truncate(n);
                }
                if flags & 0x10 != 0 || flags & 0x10_0000 != 0 {
                    if addr % PAGE != 0 || addr == 0 || addr.checked_add(ceil(len)).is_none_or(|e| e > USER_TOP) {
                        return err(EINVAL);
                    }
                    if flags & 0x10_0000 != 0 && !p.mem.is_free(addr, addr + ceil(len)) {
                        return err(EEXIST);
                    }
                    p.mem.map_fixed(addr, len, prot, &init, &name)?;
                    if flags & 1 != 0 {
                        p.mem.make_shared(addr, len);
                        if let Some((path, o)) = file_backing.take() {
                            p.fmaps.push((addr, len, path, o));
                        }
                    }
                    ok(addr as i64)
                } else {
                    // Honour the hint if it is free (glibc ld.so relies on this for libs).
                    if addr != 0 && addr % PAGE == 0 && addr.checked_add(ceil(len)).is_some_and(|e| e <= USER_TOP) && p.mem.is_free(addr, addr + ceil(len)) {
                        p.mem.map_fixed(addr, len, prot, &init, &name)?;
                        if flags & 1 != 0 {
                            p.mem.make_shared(addr, len);
                            if let Some((path, o)) = file_backing.take() {
                                p.fmaps.push((addr, len, path, o));
                            }
                        }
                        return ok(addr as i64);
                    }
                    let at = p.mem.map_anywhere(len, prot, &init, &name)?;
                    if flags & 1 != 0 {
                        p.mem.make_shared(at, len);
                    }
                    if let (true, Some((path, o))) = (flags & 1 != 0, file_backing.take()) {
                        p.fmaps.push((at, len, path, o));
                    }
                    ok(at as i64)
                }
            }
            26 => {
                // msync(addr, len, flags)
                self.sync_maps(p, a[0], a[1]);
                ok(0)
            }
            10 => {
                if a[0] % PAGE != 0 || a[2] & !7 != 0 {
                    return err(EINVAL);
                }
                p.mem.protect(a[0], a[1], a[2] as u32)?;
                ok(0)
            }
            11 => {
                if a[1] == 0 {
                    return err(EINVAL);
                }
                self.sync_maps(p, a[0], a[1]);
                let (lo, hi) = (a[0], a[0].saturating_add(ceil(a[1])));
                let old = core::mem::take(&mut p.fmaps);
                for (st, l, path, off) in old {
                    if st + l <= lo || st >= hi {
                        p.fmaps.push((st, l, path, off));
                        continue;
                    }
                    if st < lo {
                        p.fmaps.push((st, lo - st, path.clone(), off));
                    }
                    if st + l > hi {
                        p.fmaps.push((hi, st + l - hi, path, off + (hi - st)));
                    }
                }
                p.mem.unmap(a[0], a[1])?;
                ok(0)
            }
            12 => ok(if a[0] == 0 { p.mem.brk as i64 } else { p.mem.set_brk(a[0]) as i64 }),
            25 => {
                // mremap(old, oldlen, newlen, flags)
                let (old, ol, nl) = (a[0], ceil(a[1]), ceil(a[2]));
                if old % PAGE != 0 || nl == 0 || ol == 0 {
                    return err(EINVAL);
                }
                if nl <= ol {
                    p.mem.unmap(old + nl, ol - nl)?;
                    return ok(old as i64);
                }
                let prot = p.mem.prot_at(old).ok_or(EFAULT)?;
                let data = p.mem.read_vec(old, ol as usize)?;
                if p.mem.is_free(old + ol, old + nl) {
                    p.mem.map_fixed(old + ol, nl - ol, prot, &[], "")?;
                    return ok(old as i64);
                }
                if a[3] & 1 == 0 {
                    return err(ENOMEM);
                }
                let n = p.mem.map_anywhere(nl, prot, &data, "")?;
                p.mem.unmap(old, ol)?;
                ok(n as i64)
            }
            28 | 149 | 150 | 151 | 152 | 153 => ok(0), // madvise, mlock*, munlock*
            13 => {
                // rt_sigaction(sig, act, oact, size)
                let sig = a[0] as usize;
                if a[3] != 8 || sig == 0 || sig > 64 || (a[1] != 0 && (sig == 9 || sig == 19)) {
                    return err(EINVAL);
                }
                let old = p.acts[sig];
                if a[2] != 0 {
                    for (i, v) in [old.handler, old.flags, old.restorer, old.mask].iter().enumerate() {
                        p.mem.write_u64(a[2] + i as u64 * 8, *v)?;
                    }
                }
                if a[1] != 0 {
                    p.acts[sig] = SigAct { handler: p.mem.read_u64(a[1])?, flags: p.mem.read_u64(a[1] + 8)?, restorer: p.mem.read_u64(a[1] + 16)?, mask: p.mem.read_u64(a[1] + 24)? };
                }
                ok(0)
            }
            14 => {
                // rt_sigprocmask(how, set, old, size)
                if a[3] != 8 {
                    return err(EINVAL);
                }
                let t = self.threads.get_mut(&tid).unwrap();
                let old = t.sigmask;
                if a[1] != 0 {
                    let set = p.mem.read_u64(a[1])? & !((1 << 8) | (1 << 18));
                    t.sigmask = match a[0] {
                        0 => old | set,
                        1 => old & !set,
                        2 => set,
                        _ => return err(EINVAL),
                    };
                }
                if a[2] != 0 {
                    p.mem.write_u64(a[2], old)?;
                }
                ok(0)
            }
            15 => {
                // rt_sigreturn
                let t = self.threads.get_mut(&tid).unwrap();
                let uc = t.regs.rsp; // after `ret` popped pretcode
                let mc = uc + 40;
                let mut v = [0u64; 18];
                for (i, s) in v.iter_mut().enumerate() {
                    *s = p.mem.read_u64(mc + i as u64 * 8)?;
                }
                let mask = p.mem.read_u64(mc + 256)?;
                let r = &mut t.regs;
                [r.r8, r.r9, r.r10, r.r11, r.r12, r.r13, r.r14, r.r15, r.rdi, r.rsi, r.rbp, r.rbx, r.rdx, r.rax, r.rcx, r.rsp, r.rip, r.rflags] = v;
                r.rflags = (r.rflags & 0xDD5) | 0x202;
                t.sigmask = mask & !((1 << 8) | (1 << 18));
                Ok(Sys::Restored)
            }
            131 => {
                // sigaltstack(ss, old)
                let t = self.threads.get_mut(&tid).unwrap();
                if a[1] != 0 {
                    p.mem.write_u64(a[1], t.altstack.0)?;
                    p.mem.write_u32(a[1] + 8, if t.altstack.1 == 0 { 2 } else { 0 })?;
                    p.mem.write_u64(a[1] + 16, t.altstack.1)?;
                }
                if a[0] != 0 {
                    let (sp, fl, sz) = (p.mem.read_u64(a[0])?, p.mem.read_u32(a[0] + 8)?, p.mem.read_u64(a[0] + 16)?);
                    if fl & 2 != 0 {
                        t.altstack = (0, 0);
                    } else if sz < 2048 {
                        return err(12);
                    } else {
                        t.altstack = (sp, sz);
                    }
                }
                ok(0)
            }
            62 | 200 | 234 => {
                // kill(pid,sig) / tkill(tid,sig) / tgkill(tgid,tid,sig)
                let (pid, t, sig) = match nr {
                    62 => (a[0] as i32, None, a[1] as u32),
                    200 => (self.threads.get(&(a[0] as u32)).map_or(0, |t| t.pid) as i32, Some(a[0] as u32), a[1] as u32),
                    _ => (a[0] as i32, Some(a[1] as u32), a[2] as u32),
                };
                if sig > 64 {
                    return err(EINVAL);
                }
                let pid = if pid <= 0 { p.pid } else { pid as u32 };
                if sig == 0 {
                    return if pid == p.pid || self.procs.contains_key(&pid) { ok(0) } else { err(ESRCH) };
                }
                if pid == p.pid || self.procs.contains_key(&pid) {
                    self.deferred.push((pid, t, sig));
                    ok(0)
                } else {
                    err(ESRCH)
                }
            }
            16 => {
                // ioctl
                let f = self.fd(p, a[0] as i32)?;
                let tty = matches!(f.borrow().kind, FKind::Tty);
                match a[1] {
                    0x5401 if tty => {
                        // TCGETS: sane cooked defaults
                        let mut t = [0u8; 36];
                        t[0..4].copy_from_slice(&0x500u32.to_le_bytes());
                        t[4..8].copy_from_slice(&5u32.to_le_bytes());
                        t[8..12].copy_from_slice(&0xbfu32.to_le_bytes());
                        t[12..16].copy_from_slice(&0x8a3bu32.to_le_bytes());
                        p.mem.write(a[2], &t)?;
                        ok(0)
                    }
                    0x5402..=0x5404 if tty => ok(0),
                    0x5413 if tty => {
                        p.mem.write(a[2], &[24, 0, 80, 0, 0, 0, 0, 0])?;
                        ok(0)
                    }
                    0x541B => {
                        let n = match &f.borrow().kind {
                            FKind::PipeR(i) => self.pipes[*i].buf.len(),
                            FKind::Tty => self.console_in.len(),
                            FKind::Sock(i) => {
                                let (i, udp, tcp) = (*i, self.socks[*i].udp, self.socks[*i].tcp);
                                match (udp, tcp) {
                                    (Some(h), _) => self.stack_mut().and_then(|n| n.udp_front_len(h)).unwrap_or(0),
                                    (_, Some(_)) => 1,
                                    _ => self.socks[i].rx.len(),
                                }
                            }
                            _ => 0,
                        };
                        p.mem.write_u32(a[2], n as u32)?;
                        ok(0)
                    }
                    _ => err(ENOTTY),
                }
            }
            21 | 269 | 439 => {
                // access / faccessat / faccessat2
                let (dirfd, pp) = if nr == 21 { (AT_FDCWD, a[0]) } else { (a[0] as i32, a[1]) };
                let path = p.mem.read_cstr(pp, 4096)?;
                let abs = self.path_for(p, dirfd, &path, true)?;
                self.stat_abs(p, &abs)?;
                ok(0)
            }
            22 | 293 => {
                // pipe / pipe2
                self.pipes.push(Pipe { buf: VecDeque::new(), readers: 0, writers: 0 });
                let i = self.pipes.len() - 1;
                let fl = if nr == 293 { a[1] as u32 } else { 0 };
                let r = self.new_fd(FKind::PipeR(i), fl & 0o4000);
                let w = self.new_fd(FKind::PipeW(i), (fl & 0o4000) | 1);
                let (fr, fw) = (self.alloc_fd(p, 0)?, 0);
                p.fds.insert(fr, FdEnt { file: r, cloexec: fl & 0o2000000 != 0 });
                let fw = { let _ = fw; self.alloc_fd(p, 0)? };
                p.fds.insert(fw, FdEnt { file: w, cloexec: fl & 0o2000000 != 0 });
                p.mem.write_u32(a[0], fr as u32)?;
                p.mem.write_u32(a[0] + 4, fw as u32)?;
                ok(0)
            }
            32 | 33 | 292 | 72 => {
                // dup / dup2 / dup3 / fcntl
                if nr == 72 {
                    return match a[1] {
                        0 | 1030 => {
                            let f = self.fd(p, a[0] as i32)?;
                            let n = self.alloc_fd(p, a[2] as i32)?;
                            p.fds.insert(n, FdEnt { file: f, cloexec: a[1] == 1030 });
                            ok(n as i64)
                        }
                        1 => ok(p.fds.get(&(a[0] as i32)).ok_or(EBADF)?.cloexec as i64),
                        2 => {
                            p.fds.get_mut(&(a[0] as i32)).ok_or(EBADF)?.cloexec = a[2] & 1 != 0;
                            ok(0)
                        }
                        3 => ok(self.fd(p, a[0] as i32)?.borrow().flags as i64),
                        4 => {
                            let f = self.fd(p, a[0] as i32)?;
                            let mut fb = f.borrow_mut();
                            fb.flags = (fb.flags & !0o6000) | (a[2] as u32 & 0o6000);
                            ok(0)
                        }
                        _ => err(EINVAL),
                    };
                }
                let f = self.fd(p, a[0] as i32)?;
                let new = if nr == 32 {
                    self.alloc_fd(p, 0)?
                } else {
                    let n = a[1] as i32;
                    if !(0..1024).contains(&n) {
                        return err(EBADF);
                    }
                    if n == a[0] as i32 {
                        return if nr == 33 { ok(n as i64) } else { err(EINVAL) };
                    }
                    if let Some(old) = p.fds.remove(&n) {
                        self.release(old.file);
                    }
                    n
                };
                p.fds.insert(new, FdEnt { file: f, cloexec: nr == 292 && a[2] & 0o2000000 != 0 });
                ok(new as i64)
            }
            217 | 78 => {
                // getdents64 / getdents (64-bit layout used for both)
                let f = self.fd(p, a[0] as i32)?;
                let path = match &f.borrow().kind {
                    FKind::Dir(d) => d.clone(),
                    _ => return err(ENOTDIR),
                };
                let mut ents = vec![(".".to_string(), 1u64, 4u8), ("..".to_string(), 1, 4)];
                for e in self.fs.list(&path)? {
                    ents.push((e.name, e.ino, match e.kind { Kind::Dir => 4, Kind::Symlink => 10, _ => 8 }));
                }
                let mut idx = f.borrow().off as usize;
                let mut out = Vec::new();
                while idx < ents.len() {
                    let (n, ino, ty) = &ents[idx];
                    let reclen = (19 + n.len() + 1).div_ceil(8) * 8;
                    if out.len() + reclen > a[2] as usize {
                        if out.is_empty() {
                            return err(EINVAL);
                        }
                        break;
                    }
                    let mut r = vec![0u8; reclen];
                    r[0..8].copy_from_slice(&ino.to_le_bytes());
                    r[8..16].copy_from_slice(&((idx + 1) as u64).to_le_bytes());
                    r[16..18].copy_from_slice(&(reclen as u16).to_le_bytes());
                    r[18] = *ty;
                    r[19..19 + n.len()].copy_from_slice(n.as_bytes());
                    out.extend(r);
                    idx += 1;
                }
                f.borrow_mut().off = idx as u64;
                p.mem.write(a[1], &out)?;
                ok(out.len() as i64)
            }
            79 => {
                let c = p.cwd.clone();
                if c.len() + 1 > a[1] as usize {
                    return err(ERANGE);
                }
                let mut b = c.into_bytes();
                b.push(0);
                p.mem.write(a[0], &b)?;
                ok(b.len() as i64)
            }
            80 | 81 => {
                let abs = if nr == 80 {
                    let path = p.mem.read_cstr(a[0], 4096)?;
                    self.path_for(p, AT_FDCWD, &path, true)?
                } else {
                    match &self.fd(p, a[0] as i32)?.borrow().kind {
                        FKind::Dir(d) => d.clone(),
                        _ => return err(ENOTDIR),
                    }
                };
                if self.fs.lstat(&abs)?.kind != Kind::Dir {
                    return err(ENOTDIR);
                }
                p.cwd = abs;
                ok(0)
            }
            89 | 267 => {
                // readlink / readlinkat
                let (dirfd, pp, buf, sz) = if nr == 89 { (AT_FDCWD, a[0], a[1], a[2]) } else { (a[0] as i32, a[1], a[2], a[3]) };
                let path = p.mem.read_cstr(pp, 4096)?;
                let abs = self.path_for(p, dirfd, &path, false)?;
                let t = if abs == "/proc/self/exe" { p.exe.clone() } else { self.fs.readlink(&abs)? };
                let n = t.len().min(sz as usize);
                p.mem.write(buf, &t.as_bytes()[..n])?;
                ok(n as i64)
            }
            83 | 258 | 84 | 87 | 263 | 82 | 264 | 88 => {
                // mkdir/mkdirat rmdir unlink/unlinkat rename/renameat symlink chmod(ignored)
                match nr {
                    83 | 258 => {
                        let (d, pp, m) = if nr == 83 { (AT_FDCWD, a[0], a[1]) } else { (a[0] as i32, a[1], a[2]) };
                        let path = p.mem.read_cstr(pp, 4096)?;
                        let abs = self.path_for(p, d, &path, false)?;
                        self.fs.mkdir(&abs, m as u32 & !p.umask)?;
                    }
                    84 | 87 | 263 => {
                        let (d, pp, dir) = match nr {
                            84 => (AT_FDCWD, a[0], true),
                            87 => (AT_FDCWD, a[0], false),
                            _ => (a[0] as i32, a[1], a[2] & 0x200 != 0),
                        };
                        let path = p.mem.read_cstr(pp, 4096)?;
                        let abs = self.path_for(p, d, &path, false)?;
                        self.fs.remove(&abs, dir)?;
                    }
                    82 | 264 => {
                        let (d1, p1, d2, p2) = if nr == 82 { (AT_FDCWD, a[0], AT_FDCWD, a[1]) } else { (a[0] as i32, a[1], a[2] as i32, a[3]) };
                        let (s1, s2) = (p.mem.read_cstr(p1, 4096)?, p.mem.read_cstr(p2, 4096)?);
                        let (f, t) = (self.path_for(p, d1, &s1, false)?, self.path_for(p, d2, &s2, false)?);
                        self.fs.rename(&f, &t)?;
                    }
                    88 => {
                        let (t, l) = (p.mem.read_cstr(a[0], 4096)?, p.mem.read_cstr(a[1], 4096)?);
                        let abs = self.path_for(p, AT_FDCWD, &l, false)?;
                        self.fs.symlink(&t, &abs)?;
                    }
                    _ => {}
                }
                ok(0)
            }
            77 | 76 => {
                // ftruncate / truncate
                let path = if nr == 77 {
                    match &self.fd(p, a[0] as i32)?.borrow().kind {
                        FKind::Reg(x) => x.clone(),
                        _ => return err(EINVAL),
                    }
                } else {
                    let s = p.mem.read_cstr(a[0], 4096)?;
                    self.path_for(p, AT_FDCWD, &s, true)?
                };
                self.fs.truncate(&path, a[1])?;
                ok(0)
            }
            74 | 75 | 73 | 90 | 91 | 92 | 93 | 94 | 268 | 306 | 266 => ok(0), // fsync, flock, chmod/chown family
            319 => {
                let _name = p.mem.read_cstr(a[0], 256)?;
                let path = format!("/tmp/.loos-memfd-{}-{}", p.pid, self.next_id);
                self.next_id = self.next_id.wrapping_add(1);
                self.fs.create(&path, 0o600)?;
                let fd = self.alloc_fd(p, 0)?;
                p.fds.insert(fd, FdEnt { file: self.new_fd(FKind::Reg(path), a[1] as u32), cloexec: false });
                ok(fd as i64)
            }
            // identity
            39 | 186 => ok(if nr == 39 { p.pid } else { tid } as i64),
            110 => ok(p.ppid as i64),
            102 | 104 | 107 | 108 | 105 | 106 | 113 | 114 => ok(0),
            111 | 121 | 112 | 124 => ok(p.pid as i64),
            95 => {
                let old = p.umask;
                p.umask = a[0] as u32 & 0o777;
                ok(old as i64)
            }
            63 => {
                // uname
                let mut b = vec![0u8; 390];
                for (i, s) in ["Linux", "loos", "6.1.0-loos", "#1 SMP LoOS", "x86_64", "(none)"].iter().enumerate() {
                    b[i * 65..i * 65 + s.len()].copy_from_slice(s.as_bytes());
                }
                p.mem.write(a[0], &b)?;
                ok(0)
            }
            99 => {
                let mut b = vec![0u8; 112];
                b[0..8].copy_from_slice(&(self.now_ns / 1_000_000_000).to_le_bytes());
                b[32..40].copy_from_slice(&(4u64 << 30).to_le_bytes());
                b[40..48].copy_from_slice(&(2u64 << 30).to_le_bytes());
                b[80..82].copy_from_slice(&(self.procs.len() as u16).to_le_bytes());
                b[104..108].copy_from_slice(&1u32.to_le_bytes());
                p.mem.write(a[0], &b)?;
                ok(0)
            }
            97 | 98 => {
                // getrlimit / getrusage
                if nr == 97 {
                    let (cur, max) = rlimit(a[0] as u32);
                    p.mem.write_u64(a[1], cur)?;
                    p.mem.write_u64(a[1] + 8, max)?;
                } else {
                    p.mem.write(a[1], &[0u8; 144])?;
                }
                ok(0)
            }
            302 => {
                // prlimit64(pid, res, new, old)
                if a[3] != 0 {
                    let (cur, max) = rlimit(a[1] as u32);
                    p.mem.write_u64(a[3], cur)?;
                    p.mem.write_u64(a[3] + 8, max)?;
                }
                ok(0)
            }
            228 | 229 => {
                // clock_gettime / clock_getres
                if a[0] > 11 {
                    return err(EINVAL);
                }
                let ns = if nr == 229 { 1 } else if a[0] == 0 { self.epoch_base_s * 1_000_000_000 + self.now_ns } else { self.now_ns };
                p.mem.write_u64(a[1], ns / 1_000_000_000)?;
                p.mem.write_u64(a[1] + 8, ns % 1_000_000_000)?;
                ok(0)
            }
            96 => {
                let ns = self.epoch_base_s * 1_000_000_000 + self.now_ns;
                if a[0] != 0 {
                    p.mem.write_u64(a[0], ns / 1_000_000_000)?;
                    p.mem.write_u64(a[0] + 8, ns % 1_000_000_000 / 1000)?;
                }
                ok(0)
            }
            201 => {
                let s = self.epoch_base_s + self.now_ns / 1_000_000_000;
                if a[0] != 0 {
                    p.mem.write_u64(a[0], s)?;
                }
                ok(s as i64)
            }
            35 | 230 => {
                // nanosleep / clock_nanosleep
                let (req, abs) = if nr == 35 { (a[0], false) } else { (a[2], a[1] & 1 != 0) };
                let ns = Self::timespec(&mut p.mem, req)?;
                let until = if abs {
                    let base = if a[0] == 0 { self.epoch_base_s * 1_000_000_000 } else { 0 };
                    ns.saturating_sub(base)
                } else {
                    self.now_ns.saturating_add(ns)
                };
                if until <= self.now_ns {
                    return ok(0);
                }
                Ok(self.block(tid, Wait::Sleep, nr, a, Some(until), false))
            }
            24 => ok(0), // sched_yield
            204 => {
                // sched_getaffinity
                if a[1] < 8 {
                    return err(EINVAL);
                }
                p.mem.write_u64(a[2], 1)?;
                ok(8)
            }
            318 => {
                let n = a[1].min(1 << 20) as usize;
                let mut b = vec![0u8; n];
                for c in b.chunks_mut(8) {
                    let r = self.rand_u64().to_le_bytes();
                    c.copy_from_slice(&r[..c.len()]);
                }
                p.mem.write(a[0], &b)?;
                ok(n as i64)
            }
            158 => {
                // arch_prctl
                let t = self.threads.get_mut(&tid).unwrap();
                match a[0] {
                    0x1002 => t.regs.fs_base = a[1],
                    0x1001 => t.regs.gs_base = a[1],
                    0x1003 => p.mem.write_u64(a[1], t.regs.fs_base)?,
                    0x1004 => p.mem.write_u64(a[1], t.regs.gs_base)?,
                    _ => return err(EINVAL),
                }
                ok(0)
            }
            218 => {
                self.threads.get_mut(&tid).unwrap().clear_tid = a[0];
                ok(tid as i64)
            }
            273 => {
                self.threads.get_mut(&tid).unwrap().robust = a[0];
                ok(0)
            }
            334 | 435 => err(ENOSYS), // rseq, clone3 → glibc falls back to clone
            202 => self.sys_futex(p, tid, nr, a),
            56 => self.sys_clone(p, tid, a),
            57 | 58 => self.sys_clone(p, tid, [17, 0, 0, 0, 0, 0]),
            59 => self.sys_execve(p, tid, a),
            60 => self.sys_exit(p, tid, a[0] as i32, false),
            231 => self.sys_exit(p, tid, a[0] as i32, true),
            61 => self.sys_wait4(p, tid, nr, a),
            41..=55 | 288 => self.sys_socket(p, tid, nr, a),
            7 | 23 | 270 | 271 | 213 | 291 | 233 | 232 | 281 | 284 | 290 | 118 | 120 | 307 => self.sys_io(p, tid, nr, a),
            _ => err(ENOSYS),
        }
    }

    fn sock_key(mem: &mut UserMem, addr: u64, len: u64) -> Result<String, i64> {
        if len < 2 || len > 128 {
            return err(EINVAL);
        }
        let b = mem.read_vec(addr, len as usize)?;
        match u16::from_le_bytes([b[0], b[1]]) {
            1 => {
                let raw = &b[2..];
                let end = raw.iter().position(|&c| c == 0 && raw[0] != 0).unwrap_or(raw.len());
                Ok(format!("unix:{}", String::from_utf8_lossy(&raw[..end])))
            }
            2 if len >= 8 => {
                Ok(format!("inet:{}.{}.{}.{}:{}", b[4], b[5], b[6], b[7], u16::from_be_bytes([b[2], b[3]])))
            }
            _ => err(97),
        }
    }

    fn inet_key(key: &str) -> Result<(Ipv4Addr, u16), i64> {
        let rest = key.strip_prefix("inet:").ok_or(EAFNOSUPPORT)?;
        let (ip, port) = rest.rsplit_once(':').ok_or(EINVAL)?;
        let mut oct = [0u8; 4];
        let mut n = 0;
        for part in ip.split('.') {
            if n == 4 { return err(EINVAL); }
            oct[n] = part.parse().map_err(|_| EINVAL)?;
            n += 1;
        }
        if n != 4 { return err(EINVAL); }
        Ok((Ipv4Addr::from(oct), port.parse().map_err(|_| EINVAL)?))
    }

    fn put_sockaddr(mem: &mut UserMem, key: &str, addr: u64, lenp: u64) -> Result<(), i64> {
        if addr == 0 || lenp == 0 {
            return Ok(());
        }
        let mut b = vec![0u8; 16];
        if let Some(rest) = key.strip_prefix("inet:") {
            let (ip, port) = rest.rsplit_once(':').ok_or(EINVAL)?;
            let mut oct = [0u8; 4];
            for (i, v) in ip.split('.').enumerate() {
                if i >= 4 { return err(EINVAL); }
                oct[i] = v.parse().map_err(|_| EINVAL)?;
            }
            b[0] = 2;
            b[2..4].copy_from_slice(&port.parse::<u16>().map_err(|_| EINVAL)?.to_be_bytes());
            b[4..8].copy_from_slice(&oct);
        } else {
            b = vec![0u8; 2 + key.len().saturating_sub(5) + 1];
            b[0] = 1;
            let path = key.strip_prefix("unix:").unwrap_or("");
            b[2..2 + path.len()].copy_from_slice(path.as_bytes());
        }
        let cap = mem.read_u32(lenp)? as usize;
        mem.write(addr, &b[..b.len().min(cap)])?;
        mem.write_u32(lenp, b.len() as u32)
    }

    fn msg_socket(&mut self, p: &mut Process, tid: Pid, nr: u64, a: [u64; 6]) -> Result<Sys, i64> {
        let f = self.fd(p, a[0] as i32)?;
        let si = match f.borrow().kind {
            FKind::Sock(i) => i,
            _ => return err(ENOTSOCK),
        };
        let msg = a[1];
        if self.socks[si].tcp.is_some() || self.socks[si].datagram {
            // Network sockets: first iovec only (what resolvers and HTTP clients use), optional peer address.
            let iov = p.mem.read_u64(msg + 16)?;
            let (base, len) = (p.mem.read_u64(iov)?, p.mem.read_u64(iov + 8)?);
            let (name, namelen) = (p.mem.read_u64(msg)?, msg + 8);
            return if nr == 46 {
                let nl = p.mem.read_u32(namelen)? as u64;
                if self.socks[si].datagram && name != 0 {
                    self.sys_socket(p, tid, 44, [a[0], base, len, 0, name, nl])
                } else {
                    self.do_write(p, tid, nr, a, &f, base, len, None)
                }
            } else if self.socks[si].datagram {
                self.sys_socket(p, tid, 45, [a[0], base, len, 0, name, namelen])
            } else {
                self.do_read(p, tid, nr, a, &f, base, len, None)
            };
        }
        let iov = p.mem.read_u64(msg + 16)?;
        let iovlen = p.mem.read_u64(msg + 24)?;
        if iovlen == 0 || iovlen > 1024 {
            return err(EINVAL);
        }
        let base = p.mem.read_u64(iov)?;
        let len = p.mem.read_u64(iov + 8)?.min(1 << 20) as usize;
        if nr == 46 {
            let data = p.mem.read_vec(base, len)?;
            let control = p.mem.read_u64(msg + 32)?;
            let control_len = p.mem.read_u64(msg + 40)?.min(1 << 20) as usize;
            let mut rights = Vec::new();
            if control != 0 && control_len != 0 {
                let c = p.mem.read_vec(control, control_len)?;
                let clen = c.get(..8).map(|b| u64::from_ne_bytes(b.try_into().unwrap()) as usize).unwrap_or(0);
                let level = c.get(8..12).map(|b| i32::from_ne_bytes(b.try_into().unwrap())).unwrap_or(0);
                let ty = c.get(12..16).map(|b| i32::from_ne_bytes(b.try_into().unwrap())).unwrap_or(0);
                if clen >= 20 && level == 1 && ty == 1 {
                    let n = (clen - 16) / 4;
                    for j in 0..n {
                        rights.push(self.fd(p, i32::from_ne_bytes(c[16 + j * 4..20 + j * 4].try_into().unwrap()))?);
                    }
                }
            }
            let pe = self.socks[si].peer.ok_or(ENOTCONN)?;
            if self.socks[pe].wayland.is_some() {
                let mut shared = Vec::new();
                for r in rights {
                    let path = match &r.borrow().kind {
                        FKind::Reg(path) => path.clone(),
                        _ => return err(EINVAL),
                    };
                    let st = self.fs.lstat(&path)?;
                    let mut bytes = vec![0u8; st.size.min(64 << 20) as usize];
                    let n = self.fs.read(&path, 0, &mut bytes)?;
                    bytes.truncate(n);
                    shared.push(SharedFd { data: bytes });
                }
                let events = {
                    let server = self.socks[pe].wayland.as_mut().unwrap();
                    server.receive_with_fds(&data, &shared).map_err(|_| EPROTO)?;
                    server.take_events()
                };
                self.socks[si].rx.extend(events);
            } else {
                self.socks[pe].rx.extend(&data);
                self.socks[pe].rights.push_back(rights);
            }
            let wake = if self.socks[pe].wayland.is_some() { si } else { pe };
            self.wake_where(|t| matches!(t.state, TState::Blocked { wait: Wait::Sock(q), .. } if q == wake));
            return Ok(Sys::Ret(data.len() as i64));
        }

        if self.socks[si].rx.is_empty() {
            if self.socks[si].peer_closed || self.socks[si].peer.is_none() {
                return Ok(Sys::Ret(0));
            }
            if f.borrow().flags & 0o4000 != 0 {
                return err(EAGAIN);
            }
            return Ok(self.block(tid, Wait::Sock(si), nr, a, None, true));
        }
        let n = len.min(self.socks[si].rx.len());
        let mut bytes = vec![0u8; n];
        for b in &mut bytes {
            *b = self.socks[si].rx.pop_front().unwrap();
        }
        p.mem.write(base, &bytes)?;
        let control = p.mem.read_u64(msg + 32)?;
        let control_len = p.mem.read_u64(msg + 40)? as usize;
        if let Some(rights) = self.socks[si].rights.pop_front() {
            if control == 0 || control_len < 16 + rights.len() * 4 {
                return err(EMSGSIZE);
            }
            let clen = 16 + rights.len() * 4;
            let mut c = vec![0u8; (clen + 3) & !3];
            c[..8].copy_from_slice(&(clen as u64).to_ne_bytes());
            c[8..12].copy_from_slice(&1i32.to_ne_bytes());
            c[12..16].copy_from_slice(&1i32.to_ne_bytes());
            for (j, r) in rights.into_iter().enumerate() {
                let fd = self.alloc_fd(p, 0)?;
                p.fds.insert(fd, FdEnt { file: r, cloexec: false });
                c[16 + j * 4..20 + j * 4].copy_from_slice(&(fd as i32).to_ne_bytes());
            }
            p.mem.write(control, &c)?;
            p.mem.write_u64(msg + 40, c.len() as u64)?;
        }
        Ok(Sys::Ret(n as i64))
    }

    /// `inet:ip:port` of the local (or remote) end of a network socket, from the stack.
    fn net_sock_key(&mut self, i: usize, peer: bool) -> Option<String> {
        let (tcp, udp, udp_peer, datagram) = (self.socks[i].tcp, self.socks[i].udp, self.socks[i].udp_peer, self.socks[i].datagram);
        let net = self.stack_mut()?;
        let (ip, port) = if let Some(h) = tcp {
            let (l, r) = net.tcp_addrs(h)?;
            if peer { r } else { l }
        } else if datagram {
            if peer {
                let (ip, port) = udp_peer?;
                (IpAddr::V4(ip), port)
            } else {
                let dst = IpAddr::V4(udp_peer.map_or(Ipv4Addr::new(8, 8, 8, 8), |p| p.0));
                (net.source_for(dst).unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED)), udp.and_then(|h| net.udp_port(h)).unwrap_or(0))
            }
        } else {
            return None;
        };
        match ip {
            IpAddr::V4(v) => Some(format!("inet:{}.{}.{}.{}:{}", v.octets()[0], v.octets()[1], v.octets()[2], v.octets()[3], port)),
            IpAddr::V6(_) => None,
        }
    }

    /// Gives a datagram socket its stack handle (an ephemeral port) the first time it is used.
    fn ensure_udp(&mut self, i: usize) -> Result<(), i64> {
        if self.socks[i].udp.is_none() {
            let Some(net) = self.stack_mut() else { return err(100) };
            self.socks[i].udp = Some(net.udp_bind(0).map_err(|_| 98i64)?);
        }
        Ok(())
    }

    /// Starts (or retries) a real TCP connection through the network stack.
    #[allow(clippy::too_many_arguments)]
    fn tcp_connect(&mut self, tid: Pid, nr: u64, a: [u64; 6], i: usize, ip: Ipv4Addr, port: u16, nonblock: bool) -> Result<Sys, i64> {
        let Some(net) = self.stack_mut() else { return err(101) }; // ENETUNREACH
        let h = net.tcp_connect(IpAddr::V4(ip), port).map_err(|_| 101i64)?;
        self.socks[i].tcp = Some(h);
        if nonblock {
            return err(EINPROGRESS);
        }
        Ok(self.block(tid, Wait::Sock(i), nr, a, None, true))
    }

    fn tcp_connect_state(&mut self, tid: Pid, nr: u64, a: [u64; 6], i: usize, h: usize, nonblock: bool) -> Result<Sys, i64> {
        let Some(net) = self.stack_mut() else { return err(101) };
        match net.tcp_state(h) {
            Some(State::Established | State::CloseWait) => Ok(Sys::Ret(0)),
            Some(State::Closed) | None => err(ECONNREFUSED),
            _ if net.tcp_failure(h).is_some() => err(ECONNREFUSED),
            _ if nonblock => err(EALREADY),
            _ => Ok(self.block(tid, Wait::Sock(i), nr, a, None, true)),
        }
    }

    fn sys_socket(&mut self, p: &mut Process, tid: Pid, nr: u64, a: [u64; 6]) -> Result<Sys, i64> {
        let sid = |this: &Self, p: &Process, fd: u64| -> Result<(usize, FileRef), i64> {
            let f = this.fd(p, fd as i32)?;
            let i = match f.borrow().kind {
                FKind::Sock(i) => i,
                _ => return err(ENOTSOCK),
            };
            Ok((i, f))
        };
        let new_sock = |this: &mut Self, p: &mut Process, i: usize, cloexec: bool, nb: bool| -> Result<i64, i64> {
            let f = this.new_fd(FKind::Sock(i), if nb { 0o4002 } else { 2 });
            let fd = this.alloc_fd(p, 0)?;
            p.fds.insert(fd, FdEnt { file: f, cloexec });
            Ok(fd as i64)
        };
        match nr {
            41 => {
                let (dom, ty) = (a[0], a[1] & 0xf);
                if dom != 1 && dom != 2 {
                    return err(97);
                }
                if ty != 1 && ty != 2 {
                    return err(93); // EPROTONOSUPPORT
                }
                if ty == 2 && self.stack_mut().is_none() {
                    return err(93); // EPROTONOSUPPORT until a LoOS stack is attached
                }
                self.socks.push(Sock::default());
                let i = self.socks.len() - 1;
                self.socks[i].datagram = ty == 2;
                Ok(Sys::Ret(new_sock(self, p, i, a[1] & 0o2000000 != 0, a[1] & 0o4000 != 0)?))
            }
            53 => {
                if a[0] != 1 || a[1] & 0xf != 1 {
                    return err(97);
                }
                self.socks.push(Sock::default());
                self.socks.push(Sock::default());
                let (x, y) = (self.socks.len() - 2, self.socks.len() - 1);
                self.socks[x].peer = Some(y);
                self.socks[y].peer = Some(x);
                let cx = a[1] & 0o2000000 != 0;
                let fx = new_sock(self, p, x, cx, false)?;
                let fy = new_sock(self, p, y, cx, false)?;
                p.mem.write_u32(a[3], fx as u32)?;
                p.mem.write_u32(a[3] + 4, fy as u32)?;
                Ok(Sys::Ret(0))
            }
            49 => {
                let (i, _) = sid(self, p, a[0])?;
                let key = Self::sock_key(&mut p.mem, a[1], a[2])?;
                if key.starts_with("inet:") {
                    let (ip, _) = Self::inet_key(&key)?;
                    if !ip.is_loopback() && self.stack_mut().is_none() {
                        return err(101);
                    }
                }
                if self.socks.iter().any(|s| s.key.as_deref() == Some(key.as_str()) && !s.closed) {
                    return err(98); // EADDRINUSE
                }
                if self.socks[i].datagram {
                    let (_, port) = Self::inet_key(&key)?;
                    let Some(net) = self.stack_mut() else { return err(100) };
                    self.socks[i].udp = Some(net.udp_bind(port).map_err(|_| 98)?);
                }
                self.socks[i].key = Some(key);
                Ok(Sys::Ret(0))
            }
            50 => {
                let (i, _) = sid(self, p, a[0])?;
                if self.socks[i].key.is_none() {
                    return err(EINVAL);
                }
                self.socks[i].listening = true;
                Ok(Sys::Ret(0))
            }
            42 => {
                let (i, f) = sid(self, p, a[0])?;
                if let Some(h) = self.socks[i].tcp {
                    return self.tcp_connect_state(tid, nr, a, i, h, f.borrow().flags & 0o4000 != 0);
                }
                if self.socks[i].peer.is_some() {
                    return err(106); // EISCONN
                }
                let key = Self::sock_key(&mut p.mem, a[1], a[2])?;
                if key.starts_with("inet:") {
                    let (ip, port) = Self::inet_key(&key)?;
                    if !ip.is_loopback() {
                        if self.socks[i].datagram {
                            self.ensure_udp(i)?;
                            self.socks[i].udp_peer = Some((ip, port));
                            return Ok(Sys::Ret(0));
                        }
                        return self.tcp_connect(tid, nr, a, i, ip, port, f.borrow().flags & 0o4000 != 0);
                    }
                }
                let Some(l) = self.socks.iter().position(|s| s.listening && !s.closed && s.key.as_deref() == Some(key.as_str())) else {
                    return err(111); // ECONNREFUSED
                };
                if self.socks[l].backlog.len() >= 128 {
                    return err(111);
                }
                self.socks.push(Sock::default());
                let srv = self.socks.len() - 1;
                self.socks[srv].key = Some(key.clone());
                self.socks[srv].peer = Some(i);
                self.socks[i].peer = Some(srv);
                if key == "unix:/run/user/1000/wayland-0" {
                    self.socks[srv].wayland = Some(WaylandServer::new());
                }
                self.socks[l].backlog.push_back(srv);
                self.wake_where(|t| matches!(t.state, TState::Blocked { wait: Wait::Sock(q), .. } if q == l));
                Ok(Sys::Ret(0))
            }
            43 | 288 => {
                let (i, f) = sid(self, p, a[0])?;
                if !self.socks[i].listening {
                    return err(EINVAL);
                }
                let Some(srv) = self.socks[i].backlog.pop_front() else {
                    if f.borrow().flags & 0o4000 != 0 {
                        return err(EAGAIN);
                    }
                    return Ok(self.block(tid, Wait::Sock(i), nr, a, None, true));
                };
                if a[1] != 0 {
                    if let Some(k) = self.socks[i].key.clone() {
                        Self::put_sockaddr(&mut p.mem, &k, a[1], a[2])?;
                    }
                }
                let fl = if nr == 288 { a[3] } else { 0 };
                Ok(Sys::Ret(new_sock(self, p, srv, fl & 0o2000000 != 0, fl & 0o4000 != 0)?))
            }
            44 | 45 => {
                // sendto / recvfrom: stream sockets plus LoOS-backed AF_INET datagrams
                let f = self.fd(p, a[0] as i32)?;
                let si = match f.borrow().kind { FKind::Sock(i) => i, _ => return err(ENOTSOCK) };
                if self.socks[si].datagram {
                    self.ensure_udp(si)?;
                    let h = self.socks[si].udp.unwrap();
                    if nr == 44 {
                        let (dst, port) = if a[4] != 0 {
                            Self::inet_key(&Self::sock_key(&mut p.mem, a[4], a[5])?)?
                        } else {
                            self.socks[si].udp_peer.ok_or(EDESTADDRREQ)?
                        };
                        let data = p.mem.read_vec(a[1], a[2].min(1 << 20) as usize)?;
                        let Some(net) = self.stack_mut() else { return err(100) };
                        net.udp_send(h, IpAddr::V4(dst), port, &data).map_err(|_| 101)?;
                        return Ok(Sys::Ret(data.len() as i64));
                    }
                    let Some(net) = self.stack_mut() else { return err(100) };
                    let Some((from, port, data)) = net.udp_recv(h) else {
                        if f.borrow().flags & 0o4000 != 0 { return err(EAGAIN); }
                        return Ok(self.block(tid, Wait::Sock(si), nr, a, None, true));
                    };
                    let n = data.len().min(a[2] as usize);
                    p.mem.write(a[1], &data[..n])?;
                    if nr == 45 && a[4] != 0 {
                        if let IpAddr::V4(ip) = from {
                            let key = format!("inet:{}.{}.{}.{}:{}", ip.octets()[0], ip.octets()[1], ip.octets()[2], ip.octets()[3], port);
                            Self::put_sockaddr(&mut p.mem, &key, a[4], a[5])?;
                        }
                    }
                    return Ok(Sys::Ret(n as i64));
                }
                if nr == 44 { self.do_write(p, tid, nr, a, &f, a[1], a[2], None) } else { self.do_read(p, tid, nr, a, &f, a[1], a[2], None) }
            }
            48 => {
                let (i, _) = sid(self, p, a[0])?;
                if let Some(h) = self.socks[i].tcp {
                    if a[1] != 0 {
                        if let Some(net) = self.stack_mut() {
                            let _ = net.tcp_close(h);
                        }
                    }
                    return Ok(Sys::Ret(0));
                }
                if let Some(pe) = self.socks[i].peer {
                    self.socks[pe].peer_closed = true;
                    self.wake_where(|t| matches!(t.state, TState::Blocked { wait: Wait::Sock(q), .. } if q == pe));
                }
                Ok(Sys::Ret(0))
            }
            51 | 52 => {
                let (i, _) = sid(self, p, a[0])?;
                if let Some(k) = self.net_sock_key(i, nr == 52) {
                    Self::put_sockaddr(&mut p.mem, &k, a[1], a[2])?;
                    return Ok(Sys::Ret(0));
                }
                if nr == 52 && (self.socks[i].tcp.is_some() || self.socks[i].datagram) {
                    return err(ENOTCONN);
                }
                let k = if nr == 51 { self.socks[i].key.clone() } else { self.socks[i].peer.and_then(|pe| self.socks[pe].key.clone()) };
                match k {
                    Some(k) => Self::put_sockaddr(&mut p.mem, &k, a[1], a[2])?,
                    None if nr == 52 => return err(ENOTCONN),
                    None => {}
                }
                Ok(Sys::Ret(0))
            }
            54 => Ok(Sys::Ret(0)),
            55 => {
                if a[3] != 0 && a[4] != 0 {
                    let (i, _) = sid(self, p, a[0])?;
                    let mut v = 0u32;
                    if a[1] == 1 && a[2] == 4 {
                        // SO_ERROR: the outcome of a non-blocking connect
                        v = self.socks[i].tcp.map_or(0, |h| match self.stack_mut() {
                            Some(net) if net.tcp_failure(h).is_some() || net.tcp_state(h) == Some(State::Closed) => ECONNREFUSED as u32,
                            _ => 0,
                        });
                    }
                    p.mem.write_u32(a[3], v)?;
                }
                Ok(Sys::Ret(0))
            }
            _ => err(ENOSYS),
        }
    }

    /// Write file-backed `MAP_SHARED` pages overlapping [addr, addr+len) back to their files
    /// (never extends a file: bytes past its current size are dropped, as the kernel does).
    fn sync_maps(&mut self, p: &mut Process, addr: u64, len: u64) {
        let end = addr.saturating_add(len);
        let maps: Vec<(u64, u64, String, u64)> = p.fmaps.iter().filter(|m| m.0 < end && m.0 + m.1 > addr).cloned().collect();
        for (start, l, path, off) in maps {
            let Ok(st) = self.fs.lstat(&path) else { continue };
            let n = l.min(st.size.saturating_sub(off));
            if n == 0 {
                continue;
            }
            if let Ok(data) = p.mem.read_vec(start, n as usize) {
                let _ = self.fs.write(&path, off, &data);
            }
        }
    }

    fn sys_exit(&mut self, p: &mut Process, tid: Pid, code: i32, group: bool) -> Result<Sys, i64> {
        if group || p.threads.len() == 1 {
            self.sync_maps(p, 0, u64::MAX);
        }
        let status = (code & 0xff) << 8;
        if group || p.threads.len() == 1 {
            // exit_process re-inserts the zombie; take our own copy out of the way.
            let mut dead = core::mem::replace(p, Process { pid: p.pid, ppid: p.ppid, mem: UserMem::new(), fds: BTreeMap::new(), cwd: String::new(), exe: String::new(), argv: vec![], acts: vec![], threads: vec![], exit: None, umask: 0, fmaps: Vec::new() });
            let pid = dead.pid;
            dead.exit = None;
            self.procs.insert(pid, dead);
            self.exit_process(pid, status);
            return Ok(Sys::ProcessExit(status));
        }
        let th = self.threads.remove(&tid).ok_or(ESRCH)?;
        p.threads.retain(|&t| t != tid);
        if th.clear_tid != 0 && p.mem.write_u32(th.clear_tid, 0).is_ok() {
            self.futex_wake(p.pid, th.clear_tid, 1);
        }
        Ok(Sys::ThreadExit)
    }

    fn sys_wait4(&mut self, p: &mut Process, tid: Pid, nr: u64, a: [u64; 6]) -> Result<Sys, i64> {
        let want = a[0] as i32;
        let kids: Vec<(Pid, Option<i32>)> = self.procs.values().filter(|c| c.ppid == p.pid && (want <= 0 || c.pid == want as u32)).map(|c| (c.pid, c.exit)).collect();
        if kids.is_empty() {
            return err(ECHILD);
        }
        if let Some((pid, Some(st))) = kids.iter().find(|k| k.1.is_some()).copied() {
            if a[1] != 0 {
                p.mem.write_u32(a[1], st as u32)?;
            }
            self.procs.remove(&pid);
            return Ok(Sys::Ret(pid as i64));
        }
        if a[2] & 1 != 0 {
            return Ok(Sys::Ret(0));
        }
        Ok(self.block(tid, Wait::Child, nr, a, None, true))
    }

    fn sys_futex(&mut self, p: &mut Process, tid: Pid, nr: u64, a: [u64; 6]) -> Result<Sys, i64> {
        let (uaddr, op, val) = (a[0], a[1] as u32, a[2] as u32);
        if uaddr % 4 != 0 {
            return err(EINVAL);
        }
        match op & 127 {
            0 | 9 => {
                let cur = p.mem.read_u32(uaddr)?;
                if cur != val {
                    return err(EAGAIN);
                }
                let deadline = if a[3] == 0 {
                    None
                } else {
                    let ns = Self::timespec(&mut p.mem, a[3])?;
                    Some(if op & 127 == 9 { ns.saturating_sub(if op & 256 != 0 { self.epoch_base_s * 1_000_000_000 } else { 0 }) } else { self.now_ns.saturating_add(ns) })
                };
                Ok(self.block(tid, Wait::Futex(p.pid, uaddr), nr, a, deadline, false))
            }
            1 | 10 => Ok(Sys::Ret(self.futex_wake(p.pid, uaddr, val as usize) as i64)),
            3 | 4 => {
                let (uaddr2, nreq) = (a[4], a[3] as usize);
                if op & 127 == 4 && p.mem.read_u32(uaddr)? != a[5] as u32 {
                    return err(EAGAIN);
                }
                let woken = self.futex_wake(p.pid, uaddr, val as usize);
                let mut moved = 0;
                let ids: Vec<Pid> = self.threads.values().filter(|t| matches!(t.state, TState::Blocked { wait: Wait::Futex(pp, ad), .. } if pp == p.pid && ad == uaddr)).map(|t| t.tid).collect();
                for t in ids.into_iter().take(nreq) {
                    if let TState::Blocked { wait, .. } = &mut self.threads.get_mut(&t).unwrap().state {
                        *wait = Wait::Futex(p.pid, uaddr2);
                        moved += 1;
                    }
                }
                Ok(Sys::Ret((woken + moved) as i64))
            }
            5 => {
                // FUTEX_WAKE_OP: *uaddr2 op= arg; wake val at uaddr; if cmp wake val2 at uaddr2
                let enc = a[5] as u32;
                let (opc, cmp, oparg, cmparg) = (enc >> 28 & 7, enc >> 24 & 15, (enc >> 12 & 0xfff) as u32, (enc & 0xfff) as u32);
                let oparg = if enc >> 28 & 8 != 0 { 1u32 << (oparg & 31) } else { oparg };
                let old = p.mem.read_u32(a[4])?;
                let new = match opc {
                    0 => oparg,
                    1 => old.wrapping_add(oparg),
                    2 => old | oparg,
                    3 => old & !oparg,
                    4 => old ^ oparg,
                    _ => return err(ENOSYS),
                };
                p.mem.write_u32(a[4], new)?;
                let mut n = self.futex_wake(p.pid, uaddr, val as usize);
                let hit = match cmp {
                    0 => old == cmparg,
                    1 => old != cmparg,
                    2 => (old as i32) < cmparg as i32,
                    3 => old as i32 <= cmparg as i32,
                    4 => old as i32 > cmparg as i32,
                    5 => old as i32 >= cmparg as i32,
                    _ => false,
                };
                if hit {
                    n += self.futex_wake(p.pid, a[4], a[3] as usize);
                }
                Ok(Sys::Ret(n as i64))
            }
            _ => err(ENOSYS),
        }
    }

    fn sys_clone(&mut self, p: &mut Process, tid: Pid, a: [u64; 6]) -> Result<Sys, i64> {
        let (flags, stack, ptid, ctid, tls) = (a[0], a[1], a[2], a[3], a[4]);
        let parent_regs = self.threads[&tid].regs;
        let mut regs = parent_regs;
        regs.rax = 0;
        if stack != 0 {
            regs.rsp = stack;
        }
        if flags & 0x8_0000 != 0 {
            regs.fs_base = tls;
        }
        let new = self.alloc_id();
        let mask = self.threads[&tid].sigmask;
        if flags & 0x1_0000 != 0 {
            // thread: needs VM|SIGHAND|FS|FILES semantics
            if flags & 0x100 == 0 || flags & 0x800 == 0 {
                return err(EINVAL);
            }
            if p.threads.len() >= 512 {
                return err(EAGAIN);
            }
            if flags & 0x10_0000 != 0 {
                p.mem.write_u32(ptid, new)?;
            }
            if flags & 0x100_0000 != 0 {
                p.mem.write_u32(ctid, new)?;
            }
            let clear = if flags & 0x20_0000 != 0 { ctid } else { 0 };
            p.threads.push(new);
            self.threads.insert(new, Thread { tid: new, pid: p.pid, regs, state: TState::Runnable, sigmask: mask, pending: 0, altstack: (0, 0), clear_tid: clear, robust: 0, resume_deadline: None });
            return Ok(Sys::Ret(new as i64));
        }
        // fork / vfork / CLONE_VM|CLONE_VFORK (posix_spawn): copy semantics
        if self.procs.len() >= 4096 {
            return err(EAGAIN);
        }
        let vm_shared = flags & 0x100 != 0;
        let mut child = p.clone();
        if vm_shared {
            child.mem = p.mem.share_all();
        }
        child.pid = new;
        child.ppid = p.pid;
        child.threads = vec![new];
        child.exit = None;
        for f in child.fds.values() {
            let _ = f; // shared open-file descriptions: Rc clone already bumped refs
        }
        if flags & 0x10_0000 != 0 {
            p.mem.write_u32(ptid, new)?;
            child.mem.write_u32(ptid, new).ok();
        }
        if flags & 0x100_0000 != 0 {
            child.mem.write_u32(ctid, new).ok();
        }
        let clear = if flags & 0x20_0000 != 0 { ctid } else { 0 };
        self.procs.insert(new, child);
        self.threads.insert(new, Thread { tid: new, pid: new, regs, state: TState::Runnable, sigmask: mask, pending: 0, altstack: (0, 0), clear_tid: clear, robust: 0, resume_deadline: None });
        Ok(Sys::Ret(new as i64))
    }

    fn sys_execve(&mut self, p: &mut Process, tid: Pid, a: [u64; 6]) -> Result<Sys, i64> {
        let path = p.mem.read_cstr(a[0], 4096)?;
        let argv = Self::read_strv(&mut p.mem, a[1])?;
        let envp = Self::read_strv(&mut p.mem, a[2])?;
        let abs = self.path_for(p, AT_FDCWD, &path, true)?;
        let st = self.fs.lstat(&abs)?;
        if st.kind != Kind::File {
            return err(EACCES);
        }
        if st.mode & 0o111 == 0 {
            return err(EACCES);
        }
        let mut mem = UserMem::new();
        let seed = self.rand_u64();
        let argv = if argv.is_empty() { vec![path.clone()] } else { argv };
        let l = loader::load(&mut self.fs, &mut mem, &abs, &argv, &envp, seed)?;
        // Point of no return.
        p.mem = mem;
        p.exe = l.exe;
        p.argv = l.argv;
        for s in 1..65 {
            if p.acts[s].handler > 1 {
                p.acts[s] = SigAct::default();
            }
        }
        let closing: Vec<i32> = p.fds.iter().filter(|(_, f)| f.cloexec).map(|(&k, _)| k).collect();
        for k in closing {
            if let Some(f) = p.fds.remove(&k) {
                self.release(f.file);
            }
        }
        let others: Vec<Pid> = p.threads.iter().copied().filter(|&t| t != tid).collect();
        for t in others {
            self.threads.remove(&t);
        }
        p.threads = vec![tid];
        let t = self.threads.get_mut(&tid).unwrap();
        t.regs = Regs { rip: l.entry, rsp: l.sp, rflags: 0x202, ..Regs::default() };
        t.altstack = (0, 0);
        t.clear_tid = 0;
        t.pending = 0;
        Ok(Sys::Exec)
    }
}

fn rlimit(res: u32) -> (u64, u64) {
    match res {
        3 => (STACK_MAX, u64::MAX),
        7 => (1024, 4096),
        _ => (u64::MAX, u64::MAX),
    }
}

include!("syscalls_generated.rs");
