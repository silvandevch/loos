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
use crate::wayland::{Frame as WaylandFrame, Server as WaylandServer, SharedFd, KEYMAP_PATH};
use core::cell::RefCell;
use alloc::collections::{BTreeMap, VecDeque};
use alloc::rc::Rc;
use core::net::{IpAddr, Ipv4Addr};
use loos_net::stack::Stack;
use loos_net::tcp::State;

/// Process/thread id (one shared id space, as in Linux).
pub type Pid = u32;
/// Tag in a futex key's pid half marking a shared-memory futex (real pids stay far below it).
const SHARED_FUTEX: Pid = 0x8000_0000;

/// Base of the `liblooswin` bridge syscall range ("LO" << 16).
pub const BRIDGE_BASE: u64 = 0x4C4F_0000;
const AT_FDCWD: i32 = -100;
/// Where unlinked-but-open files live until their last descriptor closes.
const UNLINKED: &str = "/.loos-unlinked/";

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
    /// `CLONE_VFORK` parent: until child (pid) execs or exits; resumes returning the child pid.
    VFork(Pid),
    /// A timerfd read waiting for its expiry.
    Timer(usize),
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
    /// The last syscalls this thread made, oldest first (reported when the process crashes).
    pub recent: alloc::collections::VecDeque<(u64, [u64; 3], Sys)>,
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
    /// x87/SSE register state (FXSAVE image), saved and restored by the kernel around each run.
    pub fx: Box<[u8; 512]>,
    /// CPU exception behind the signal being delivered: (vector, error code, cr2).
    pub fault: Option<(u64, u64, u64)>,
    /// `prctl(PR_SET_NAME)` name (empty: the program's file name).
    pub name: String,
}

/// Initial FPU state: x87 control word 0x37f, MXCSR 0x1f80 (all exceptions masked).
pub fn fx_default() -> Box<[u8; 512]> {
    let mut fx = Box::new([0u8; 512]);
    fx[0] = 0x7f;
    fx[1] = 0x03;
    fx[24..28].copy_from_slice(&0x1f80u32.to_le_bytes());
    fx[28..32].copy_from_slice(&0xffffu32.to_le_bytes()); // MXCSR_MASK
    fx
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
    /// timerfd: index into `timers`.
    TimerFd(usize),
}

/// A timerfd: next expiry on the monotonic clock (`now_ns` domain; 0 = disarmed) and the period.
#[derive(Clone, Copy, Debug, Default)]
pub struct Timer {
    next: u64,
    interval: u64,
    realtime: bool,
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
    /// Passed fds by the stream position of the first byte of the message that carried them.
    rights: VecDeque<(u64, Vec<FileRef>)>,
    /// Bytes ever queued / consumed on `rx` (stream positions for `rights`).
    rx_tail: u64,
    rx_head: u64,
    wayland: Option<WaylandServer>,
    /// Local SOCK_SEQPACKET / SOCK_DGRAM: `rx` holds whole messages whose lengths are in `msgs`.
    packet: bool,
    msgs: VecDeque<usize>,
}

struct Pipe {
    buf: VecDeque<u8>,
    readers: usize,
    writers: usize,
    /// Bumped whenever bytes go in or out (edge-triggered epoll).
    gen: u64,
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
    /// Page cache of files mapped `MAP_SHARED` (or handed to the compositor): every mapping of a
    /// path aliases these pages, so a client's drawing into a memfd is live for the compositor.
    page_cache: BTreeMap<String, SharedBuf>,
    /// memfd seals by file (files without an entry are sealed against new seals).
    seals: BTreeMap<String, u32>,
    /// Unlinked files kept for open descriptors: hidden path -> open-file objects still referring to it.
    unlinked_open: BTreeMap<String, usize>,
    /// The vDSO time page shared (read-only) by every process; see [`Self::set_clock_page`].
    vvar: SharedBuf,
    /// Output size advertised to Wayland clients.
    pub wayland_screen: (u32, u32),
    /// Client area of a maximized Linux window (screen minus LoOS window frame).
    pub wayland_work_area: (u32, u32),
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
    /// Diagnostics for the system log (never mixed into program output).
    pub diag: String,
    /// Log every syscall to the console (strace-style).
    pub trace: bool,
    /// When set, trace only these syscall numbers.
    pub trace_only: Option<Vec<u64>>,
    /// Last I/O fingerprint pollers were checked against, and when they were last swept.
    io_seen: u64,
    poll_sweep_ns: u64,
    /// Diagnostics: per process, the last fd poll/epoll reported ready ("fd N <what> ev X").
    pub last_ready: BTreeMap<Pid, String>,
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
    /// timerfds.
    pub(crate) timers: Vec<Timer>,
    /// epoll sets: (fd, events, data, activity seen at the last report for EPOLLET).
    /// epoll sets: (fd, events, data, last activity reported, the open file registered). A
    /// registration belongs to the open file, as on Linux: once that file is closed it is gone, even
    /// if the fd number is reused (its data would point at the old file's freed watcher).
    epolls: Vec<Vec<(i32, u32, u64, u64, alloc::rc::Weak<RefCell<OpenFile>>)>>,
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
            page_cache: BTreeMap::new(),
            seals: BTreeMap::new(),
            unlinked_open: BTreeMap::new(),
            wayland_screen: (1024, 768),
            wayland_work_area: (1024, 720),
            vvar: {
                let mut pb = PageBuf::default();
                pb.write_at(0, &[1; 8]); // allocate the page now: its address must never change
                Rc::new(RefCell::new(pb))
            },
            next_id: 100,
            now_ns: 0,
            epoch_base_s: EPOCH_BASE_S,
            missing: BTreeMap::new(),
            console_out: Vec::new(),
            diag: String::new(),
            trace: false,
            trace_only: None,
            last_ready: BTreeMap::new(),
            io_seen: 0,
            poll_sweep_ns: 0,
            stdin_eof: false,
            console_in: VecDeque::new(),
            bridge: None,
            rng: 0x9E37_79B9_7F4A_7C15,
            wakeq: Vec::new(),
            deferred: Vec::new(),
            net: None,
            eventfds: Vec::new(),
            timers: Vec::new(),
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

    /// The shared page cache for `path`, created from the file's current bytes. Entries no mapping
    /// or compositor pool uses any more are dropped.
    fn cached_file(&mut self, path: &str) -> Result<SharedBuf, i64> {
        self.page_cache.retain(|_, b| Rc::strong_count(b) > 1);
        if let Some(b) = self.page_cache.get(path) {
            return Ok(b.clone());
        }
        let st = self.fs.lstat(path)?;
        let mut bytes = vec![0u8; st.size.min(1 << 30) as usize];
        let n = self.fs.read(path, 0, &mut bytes)?;
        let mut pb = PageBuf::default();
        pb.write_at(0, &bytes[..n]);
        pb.grow_to(st.size as usize);
        let b = Rc::new(RefCell::new(pb));
        self.page_cache.insert(path.into(), b.clone());
        Ok(b)
    }

    /// Activity counter of an open file: moves whenever its readiness may have changed
    /// (data in or out, counters, timers). Edge-triggered epoll reports only when it moved.
    pub(crate) fn activity(&self, f: &FileRef) -> u64 {
        match f.borrow().kind {
            FKind::Sock(i) => {
                let s = &self.socks[i];
                let drained = s.peer.map_or(0, |pe| self.socks[pe].rx_head);
                s.rx_tail.wrapping_add(drained).wrapping_add(s.backlog.len() as u64).wrapping_add(s.peer_closed as u64)
            }
            FKind::PipeR(i) | FKind::PipeW(i) => self.pipes[i].gen.wrapping_add(self.pipes[i].writers as u64 * 7 + self.pipes[i].readers as u64 * 13),
            FKind::EventFd(i) => self.eventfds[i].0,
            FKind::TimerFd(i) => self.timers[i].next ^ (self.now_ns >= self.timers[i].next) as u64,
            _ => 0,
        }
    }

    /// Queue a message for local socket `i`; `fds` travel with its first byte (SCM_RIGHTS).
    fn sock_push(&mut self, i: usize, data: &[u8], fds: Vec<FileRef>) {
        let s = &mut self.socks[i];
        if !fds.is_empty() {
            s.rights.push_back((s.rx_tail, fds));
        }
        s.rx.extend(data);
        s.rx_tail += data.len() as u64;
        if s.packet {
            s.msgs.push_back(data.len());
        }
    }

    /// Take up to `max` received bytes from local socket `i`, plus the fds sent with them. Like Linux,
    /// a read never runs into a later message that carries fds, so every fd set arrives with exactly
    /// the bytes it was sent with. Packet sockets return one whole message (excess is discarded).
    fn sock_pop(&mut self, i: usize, max: usize) -> (Vec<u8>, Vec<FileRef>) {
        let s = &mut self.socks[i];
        let start = s.rx_head;
        let fds = match s.rights.front() {
            Some((pos, _)) if *pos <= start => s.rights.pop_front().unwrap().1,
            _ => Vec::new(),
        };
        let (take, keep) = if s.packet {
            let l = s.msgs.pop_front().unwrap_or(s.rx.len());
            (l, l.min(max))
        } else {
            let limit = s.rights.front().map_or(usize::MAX, |(pos, _)| (pos - start) as usize);
            let n = max.min(s.rx.len()).min(limit);
            (n, n)
        };
        let take = take.min(s.rx.len());
        let mut out: Vec<u8> = s.rx.drain(..take).collect();
        out.truncate(keep);
        s.rx_head += take as u64;
        (out, fds)
    }

    fn wake_pollers(&mut self) {
        self.wake_where(|t| matches!(t.state, TState::Blocked { wait: Wait::Poll, .. }));
    }

    /// Takes frames committed by Wayland clients, keyed by connection, and lets those clients
    /// draw their next frame (`wl_surface.frame` callbacks fire once a frame was presented).
    /// With `fire_callbacks_now` false the caller reports presentation later through
    /// [`Self::wayland_presented`] (callbacks then pace clients at the display refresh).
    pub fn take_wayland_frames(&mut self, fire_callbacks_now: bool) -> Vec<(usize, WaylandFrame)> {
        let time_ms = (self.now_ns / 1_000_000) as u32;
        let mut out = Vec::new();
        for i in 0..self.socks.len() {
            if let Some(server) = self.socks[i].wayland.as_mut() {
                let frames = server.take_frames();
                if !frames.is_empty() && fire_callbacks_now {
                    server.frames_presented(time_ms);
                }
                out.extend(frames.into_iter().map(|f| (i, f)));
                self.wayland_flush(i);
            }
        }
        out
    }

    /// The screen showing the latest Wayland frames was presented: fire every pending
    /// `wl_surface.frame` callback, so clients draw their next frame at the display's pace.
    pub fn wayland_presented(&mut self) {
        // Clients compare callback times with their own CLOCK_MONOTONIC (GTK, Firefox's vsync).
        let time_ms = (self.now_ns / 1_000_000) as u32;
        for i in 0..self.socks.len() {
            if let Some(server) = self.socks[i].wayland.as_mut() {
                server.frames_presented(time_ms);
                self.wayland_flush(i);
            }
        }
    }

    /// Surfaces whose windows should close, keyed by connection (role destroyed or client gone).
    pub fn take_wayland_closed(&mut self) -> Vec<(usize, u32)> {
        let mut out = Vec::new();
        for (i, s) in self.socks.iter_mut().enumerate() {
            if let Some(server) = s.wayland.as_mut() {
                out.extend(server.take_closed().into_iter().map(|c| (i, c)));
            }
        }
        out
    }

    /// True while connection `conn` still has its client.
    pub fn wayland_alive(&self, conn: usize) -> bool {
        self.socks.get(conn).is_some_and(|s| s.wayland.is_some() && s.peer.is_some() && !s.peer_closed)
    }

    /// Deliver compositor input/requests to connection `conn` (pointer, keys, close).
    pub fn wayland_input(&mut self, conn: usize, f: impl FnOnce(&mut WaylandServer)) {
        if let Some(server) = self.socks.get_mut(conn).and_then(|s| s.wayland.as_mut()) {
            f(server);
            self.wayland_flush(conn);
        }
    }

    /// Move a server socket's pending events (and the fds they carry) to its client.
    fn wayland_flush(&mut self, srv: usize) {
        let Some(server) = self.socks[srv].wayland.as_mut() else { return };
        let events = server.take_events();
        let paths = server.take_out_fds();
        let Some(cli) = self.socks[srv].peer else { return };
        if events.is_empty() {
            return;
        }
        let mut files = Vec::new();
        for path in paths {
            let kind = if path == "/dev/null" { FKind::Null } else { FKind::Reg(path) };
            files.push(self.new_fd(kind, 0));
        }
        self.sock_push(cli, &events, files);
        self.wake_where(|t| matches!(t.state, TState::Blocked { wait: Wait::Sock(q), .. } if q == cli));
        self.wake_pollers();
    }

    /// Feed client bytes (and passed fds) to the compositor connection `srv`.
    fn wayland_feed(&mut self, srv: usize, data: &[u8], rights: Vec<FileRef>) -> Result<(), i64> {
        let mut shared = Vec::new();
        for r in rights {
            let path = match &r.borrow().kind {
                FKind::Reg(path) => path.clone(),
                _ => return err(EINVAL),
            };
            shared.push(SharedFd { buf: self.cached_file(&path)? });
        }
        let r = self.socks[srv].wayland.as_mut().unwrap().receive_with_fds(data, &shared);
        self.wayland_flush(srv);
        r.map_err(|_| EPROTO)
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
    /// Live processes (zombies excluded) and threads, for status reports.
    pub fn counts(&self) -> (usize, usize) {
        (self.procs.values().filter(|p| p.exit.is_none()).count(), self.threads.len())
    }
    /// All thread ids.
    pub fn thread_ids(&self) -> Vec<Pid> {
        self.threads.keys().copied().collect()
    }
    /// Earliest blocking deadline (ns) of any thread, if one exists.
    pub fn next_deadline(&self) -> Option<u64> {
        let timers = self.timers.iter().filter(|t| t.next > self.now_ns).map(|t| t.next);
        self.threads.values().filter_map(|t| if let TState::Blocked { deadline, .. } = &t.state { *deadline } else { None }).chain(timers).min()
    }

    /// Expirations of timerfd `i` up to now; consumes them (re-arming periodic timers).
    fn timer_take(&mut self, i: usize) -> u64 {
        let now = self.now_ns;
        let t = &mut self.timers[i];
        if t.next == 0 || now < t.next {
            return 0;
        }
        if t.interval == 0 {
            t.next = 0;
            return 1;
        }
        let n = 1 + (now - t.next) / t.interval;
        t.next += n * t.interval;
        n
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

    /// Unlink a file. Open descriptors keep an unlinked file alive (Linux semantics programs rely on,
    /// e.g. shared memory that is created, opened, unlinked, then resized through the fd): such a file
    /// moves to a hidden path that its descriptors follow, and goes away with the last of them.
    fn unlink(&mut self, p: &mut Process, abs: &str) -> Result<(), i64> {
        let open: Vec<FileRef> = p
            .fds
            .values()
            .chain(self.procs.values().flat_map(|q| q.fds.values()))
            .map(|e| e.file.clone())
            .filter(|f| matches!(&f.borrow().kind, FKind::Reg(x) if x == abs))
            .collect();
        if open.is_empty() && !self.page_cache.contains_key(abs) {
            return self.fs.remove(abs, false);
        }
        self.fs.lstat(abs)?;
        let hidden = format!("{UNLINKED}{}", self.next_id);
        self.next_id = self.next_id.wrapping_add(1);
        if self.fs.lstat(UNLINKED.trim_end_matches('/')).is_err() {
            self.fs.mkdir(UNLINKED.trim_end_matches('/'), 0o700)?;
        }
        self.fs.rename(abs, &hidden)?;
        // Distinct open-file objects (dup'd and inherited descriptors share one).
        let mut objs: Vec<FileRef> = Vec::new();
        for f in open {
            if !objs.iter().any(|o| Rc::ptr_eq(o, &f)) {
                f.borrow_mut().kind = FKind::Reg(hidden.clone());
                objs.push(f);
            }
        }
        self.unlinked_open.insert(hidden.clone(), objs.len());
        if let Some(b) = self.page_cache.remove(abs) {
            self.page_cache.insert(hidden.clone(), b);
        }
        if let Some(sl) = self.seals.remove(abs) {
            self.seals.insert(hidden.clone(), sl);
        }
        for q in self.procs.values_mut().map(|q| &mut q.fmaps).chain(core::iter::once(&mut p.fmaps)) {
            for m in q.iter_mut().filter(|m| m.2 == abs) {
                m.2 = hidden.clone();
            }
        }
        Ok(())
    }

    /// Map the shared vDSO time page into a fresh program image (only when there is a vDSO).
    fn map_clock_page(&self, mem: &mut UserMem) -> Result<(), i64> {
        if crate::vdso::memory_image().is_some() {
            mem.map_shared_buf(Some(crate::vdso::VVAR_ADDR), PAGE, PROT_READ, self.vvar.clone(), 0, "[vvar]")?;
        }
        Ok(())
    }

    /// Refresh the vDSO time page before user code runs: the monotonic clock is `now_ns` at TSC
    /// value `tsc`, and `mult` converts TSC ticks to nanoseconds (32.32 fixed point). The kernel must
    /// advance `now_ns` by the same TSC-derived amount afterwards, so the clock never runs backwards.
    pub fn set_clock_page(&mut self, tsc: u64, mult: u64) {
        let mut page = [0u8; 40];
        for (k, v) in [self.now_ns, tsc, mult, self.epoch_base_s].iter().enumerate() {
            page[k * 8..k * 8 + 8].copy_from_slice(&v.to_le_bytes());
        }
        self.vvar.borrow_mut().write_at(0, &page);
    }

    /// Delete unlinked files that no descriptor and no mapping uses any more.
    fn reap_unlinked(&mut self) {
        let dead: Vec<String> = self
            .unlinked_open
            .iter()
            .filter(|(p, &n)| n == 0 && !self.page_cache.get(*p).is_some_and(|b| Rc::strong_count(b) > 1))
            .map(|(p, _)| p.clone())
            .collect();
        for path in dead {
            self.unlinked_open.remove(&path);
            let _ = self.fs.remove(&path, false);
            self.page_cache.remove(&path);
            self.seals.remove(&path);
        }
    }

    fn release(&mut self, f: FileRef) {
        self.release_inner(f);
        self.reap_unlinked();
    }

    fn release_inner(&mut self, f: FileRef) {
        if Rc::strong_count(&f) == 1 {
            // Last descriptor of an unlinked file: now it really goes.
            if let FKind::Reg(path) = &f.borrow().kind {
                if let Some(n) = self.unlinked_open.get_mut(path) {
                    *n = n.saturating_sub(1);
                }
            }
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

    /// A `CLONE_VFORK` child exec'd or exited: its parent may run again.
    fn vfork_done(&mut self, child: Pid) {
        let parents: Vec<Pid> = self.threads.values().filter(|t| matches!(t.state, TState::Blocked { wait: Wait::VFork(c), .. } if c == child)).map(|t| t.pid).collect();
        for pid in parents {
            if let Some(p) = self.procs.get_mut(&pid) {
                p.mem.end_vfork();
            }
        }
        self.wake_where(|t| matches!(t.state, TState::Blocked { wait: Wait::VFork(c), .. } if c == child));
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
        let regs = Regs { rip: l.entry, rsp: l.sp, rflags: 0x202, ..Regs::default() };
        self.spawn_image(mem, regs, l.exe, argv)
    }

    /// Create a process from an address space another loader built (e.g. LoOS Prism's Mach-O
    /// loader), with fds 0-2 on the console and one thread starting at `regs`.
    pub fn spawn_image(&mut self, mut mem: UserMem, regs: Regs, exe: String, argv: &[String]) -> Result<Pid, i64> {
        self.map_clock_page(&mut mem)?;
        let pid = self.alloc_id();
        let tty = self.new_fd(FKind::Tty, 2);
        let mut fds = BTreeMap::new();
        for i in 0..3 {
            fds.insert(i, FdEnt { file: tty.clone(), cloexec: false });
        }
        let mut acts = vec![SigAct::default(); 65];
        acts[0] = SigAct::default();
        self.procs.insert(
            pid,
            Process { pid, ppid: 0, mem, fds, cwd: "/".into(), exe, argv: argv.to_vec(), acts, threads: vec![pid], exit: None, umask: 0o022, fmaps: Vec::new() },
        );
        self.threads.insert(pid, Thread { tid: pid, pid, regs, state: TState::Runnable, sigmask: 0, pending: 0, altstack: (0, 0), clear_tid: 0, robust: 0, resume_deadline: None, fx: fx_default(), fault: None, name: String::new(), recent: Default::default() });
        Ok(pid)
    }

    /// Advance the clock: fires sleep/futex timeouts.
    pub fn advance(&mut self, ns: u64) {
        let before = self.now_ns;
        self.now_ns += ns;
        let now = self.now_ns;
        // timerfds that expired in this step: their readers and pollers re-check.
        // Safety net: sweep pollers every 10 ms even without a visible I/O change.
        if now.saturating_sub(self.poll_sweep_ns) >= 10_000_000 {
            self.poll_sweep_ns = now;
            self.wake_pollers();
            self.drain();
        }
        let fired: Vec<usize> = (0..self.timers.len()).filter(|&i| self.timers[i].next != 0 && self.timers[i].next > before && self.timers[i].next <= now).collect();
        if !fired.is_empty() {
            self.wake_where(|t| matches!(t.state, TState::Blocked { wait: Wait::Poll, .. } | TState::Blocked { wait: Wait::Timer(_), .. }));
            self.drain();
        }
        let due: Vec<(Pid, Wait)> = self
            .threads
            .values()
            .filter_map(|t| match &t.state {
                TState::Blocked { wait, deadline: Some(d), .. } if *d <= now => Some((t.tid, *wait)),
                _ => None,
            })
            .collect();
        let mut restart = false;
        for (tid, w) in due {
            if matches!(w, Wait::Timer(_)) {
                self.wakeq.push(tid); // re-run the read: it now returns the expiration count
                restart = true;
                continue;
            }
            let t = self.threads.get_mut(&tid).unwrap();
            t.state = TState::Runnable;
            t.regs.rax = if matches!(w, Wait::Sleep | Wait::Poll) { 0 } else { (-ETIMEDOUT) as u64 };
        }
        if restart {
            self.drain();
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
                    None => t.regs.rax = if let Wait::VFork(c) = wait { c as u64 } else { 0 },
                }
            }
        }
        let d = core::mem::take(&mut self.deferred);
        for (pid, tid, sig) in d {
            self.send_signal(pid, tid, sig);
        }
    }

    /// A CPU exception in user mode: queue the matching signal with the fault details
    /// (si_addr, cr2, trapno and error code in the signal frame).
    pub fn fault(&mut self, tid: Pid, vector: u64, error: u64, cr2: u64) {
        let sig = match vector {
            0 | 16 | 19 => 8,
            1 | 3 => 5,
            6 => 4,
            17 => 7,
            _ => 11,
        };
        let Some(t) = self.threads.get_mut(&tid) else { return };
        t.fault = Some((vector, error, cr2));
        let pid = t.pid;
        self.send_signal(pid, Some(tid), sig);
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
        // [pretcode][ucontext 8..432][siginfo 432..560][pad][fpstate 584..1096, 16-aligned]
        sp = (sp.wrapping_sub(1104)) & !15;
        let frame = sp.wrapping_sub(8);
        let mut buf = vec![0u8; 1096];
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
        // sigcontext tail: cs/gs/fs/ss, err, trapno, oldmask, cr2, fpstate pointer
        let fault = t.fault.take();
        let (vector, error, cr2) = fault.unwrap_or((0, 0, 0));
        put(&mut buf, mc + 18 * 8, 0x002b_0000_0000_0033);
        put(&mut buf, mc + 19 * 8, error);
        put(&mut buf, mc + 20 * 8, vector);
        put(&mut buf, mc + 21 * 8, t.sigmask);
        put(&mut buf, mc + 22 * 8, cr2);
        put(&mut buf, mc + 23 * 8, frame + 584);
        put(&mut buf, 8 + 296 - 8 + 8, t.sigmask); // uc_sigmask @ mcontext+256
        // siginfo @432: si_signo, si_errno, si_code, then si_addr for faults
        buf[432..436].copy_from_slice(&sig.to_le_bytes());
        if fault.is_some() {
            let code: u32 = match (sig, vector) {
                (11, _) => if error & 1 != 0 { 2 } else { 1 }, // SEGV_ACCERR / SEGV_MAPERR
                (8, 0) => 1,                                    // FPE_INTDIV
                (8, _) => 7,                                    // FPE_FLTINV
                (4, _) => 2,                                    // ILL_ILLOPN
                _ => 128,                                       // SI_KERNEL
            };
            buf[440..444].copy_from_slice(&code.to_le_bytes());
            put(&mut buf, 448, if sig == 11 { cr2 } else { r.rip });
        }
        buf[584..1096].copy_from_slice(&t.fx[..]);
        let ok = p.mem.write(frame, &buf).is_ok();
        if !ok {
            self.procs.insert(pid, p);
            self.terminate_process(pid, 11);
            return Delivery::Killed(11);
        }
        t.sigmask |= act.mask | if act.flags & 0x4000_0000 != 0 { 0 } else { 1 << (sig - 1) };
        if act.flags & 0x8000_0000 != 0 {
            p.acts[sig as usize] = SigAct::default(); // SA_RESETHAND: one-shot handler
        }
        t.fx = fx_default(); // handlers start with a clean FPU state, as on Linux
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
        p.mem = UserMem::new(); // a zombie keeps no memory (and releases what it shared with a vfork parent)
        self.vfork_done(pid);
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
        if nr == 231 && a[0] & 0xff != 0 {
            // A program giving up: say who it was and what it last did (the console is the only witness).
            let who = self.threads.get(&tid).and_then(|t| self.procs.get(&t.pid)).map(|p| p.argv.join(" ")).unwrap_or_default();
            self.diag += &format!("lin: [{tid}] exits with status {}: {}\n{}", a[0] & 0xff, who.chars().take(200).collect::<String>(), self.recent_syscalls(tid));
        }
        let r = self.syscall_inner(tid, nr, a);
        if let Some(t) = self.threads.get_mut(&tid) {
            if t.recent.len() == 24 {
                t.recent.pop_front();
            }
            t.recent.push_back((nr, [a[0], a[1], a[2]], r));
        }
        if self.trace && self.trace_only.as_ref().is_none_or(|only| only.contains(&nr)) {
            let name = crate::syscalls_generated::syscall_name(nr);
            let line = format!("[{tid}] {name}#{nr}({:#x}, {:#x}, {:#x}, {:#x}) = {r:?}\n", a[0], a[1], a[2], a[3]);
            self.console_out.extend(line.as_bytes());
        }
        self.drain();
        // Re-check poll/epoll sleepers only when I/O state changed: re-running every sleeper's
        // scan after every syscall made each syscall cost dozens.
        let fp = self.io_fingerprint();
        if fp != self.io_seen {
            self.io_seen = fp;
            self.wake_pollers();
            self.drain();
        }
        r
    }

    /// The thread's last syscalls, one per line, for a crash report.
    pub fn recent_syscalls(&self, tid: Pid) -> String {
        let Some(t) = self.threads.get(&tid) else { return String::new() };
        let mut out = String::new();
        for (nr, a, r) in &t.recent {
            let name = crate::syscalls_generated::syscall_name(*nr);
            out += &format!("lin:   [{tid}] {name}#{nr}({:#x}, {:#x}, {:#x}) = {r:?}\n", a[0], a[1], a[2]);
        }
        out
    }

    /// Cheap digest of everything poll/epoll readiness depends on (sockets, pipes, counters, timers).
    fn io_fingerprint(&self) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut mix = |v: u64| h = (h ^ v).wrapping_mul(0x100_0000_01b3);
        for s in &self.socks {
            mix(s.rx_tail);
            mix(s.rx_head);
            mix(s.backlog.len() as u64 | (s.peer_closed as u64) << 32 | (s.closed as u64) << 33 | (s.peer.is_some() as u64) << 34);
        }
        for p in &self.pipes {
            mix(p.gen ^ (p.readers as u64) << 40 ^ (p.writers as u64) << 48);
        }
        for e in &self.eventfds {
            mix(e.0);
        }
        for t in &self.timers {
            mix(t.next);
        }
        mix(self.socks.len() as u64 ^ (self.pipes.len() as u64) << 20);
        h
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
            "/proc/self/status" => format!("Name:\t{}\nState:\tR (running)\nTgid:\t{}\nPid:\t{}\nPPid:\t{}\nUid:\t1000\t1000\t1000\t1000\nGid:\t1000\t1000\t1000\t1000\nThreads:\t{}\nVmSize:\t{} kB\n", p.exe.rsplit('/').next().unwrap_or(""), p.pid, p.pid, p.ppid, p.threads.len(), p.mem.mapped() / 1024),
            "/proc/cpuinfo" => "processor\t: 0\nvendor_id\t: LoOS\nmodel name\t: x86-64 (LoOS native)\ncpu MHz\t\t: 2000.000\nflags\t\t: fpu sse sse2 syscall nx lm\n\n".into(),
            "/proc/meminfo" => "MemTotal:        4194304 kB\nMemFree:         2097152 kB\nMemAvailable:    2097152 kB\n".into(),
            "/proc/version" => "Linux version 6.1.0-loos (loos@loos) #1 SMP LoOS\n".into(),
            "/proc/self/stat" => {
                // All 52 fields, as proc(5) lists them; libuv reads rss (24) and vsize (23).
                let (comm, vsize, rss) = (p.exe.rsplit('/').next().unwrap_or(""), p.mem.mapped(), p.mem.resident_pages());
                let mut f = format!("{} ({comm}) R {} {} {} 0 -1 4194304 0 0 0 0 0 0 0 0 20 0 {} 0 0 {vsize} {rss} 18446744073709551615", p.pid, p.ppid, p.pid, p.pid, p.threads.len());
                f += &" 0".repeat(52 - 25);
                f + "\n"
            }
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
        if let Some(f) = Self::proc_fd(p, path) {
            return match &f?.borrow().kind {
                FKind::Reg(target) | FKind::Dir(target) => self.fs.lstat(target),
                _ => Ok(Stat { ino: 13, kind: Kind::Fifo, mode: 0o600, size: 0, nlink: 1, uid: 0, gid: 0, mtime: 0 }),
            };
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

    /// The open file behind `/proc/self/fd/N` (also `/proc/<own pid>/fd/N`, `/proc/thread-self/fd/N`).
    fn proc_fd(p: &Process, abs: &str) -> Option<Result<FileRef, i64>> {
        let rest = abs.strip_prefix("/proc/")?;
        let (who, n) = rest.split_once("/fd/")?;
        if who != "self" && who != "thread-self" && who.parse::<Pid>().ok() != Some(p.pid) {
            return None;
        }
        Some(n.parse::<i32>().ok().and_then(|n| p.fds.get(&n)).map(|e| e.file.clone()).ok_or(ENOENT))
    }

    /// What `readlink("/proc/self/fd/N")` shows.
    pub(crate) fn fd_link(f: &FileRef) -> String {
        match &f.borrow().kind {
            FKind::Reg(path) | FKind::Dir(path) => path.clone(),
            FKind::Sock(i) => format!("socket:[{}]", 10000 + i),
            FKind::PipeR(i) | FKind::PipeW(i) => format!("pipe:[{}]", 20000 + i),
            FKind::Null => "/dev/null".into(),
            FKind::Tty => "/dev/tty".into(),
            FKind::EventFd(_) => "anon_inode:[eventfd]".into(),
            FKind::TimerFd(_) => "anon_inode:[timerfd]".into(),
            FKind::Epoll(_) => "anon_inode:[eventpoll]".into(),
            _ => "anon_inode:[loos]".into(),
        }
    }

    fn open_path(&mut self, p: &mut Process, abs: &str, flags: u32, mode: u32) -> Result<FileRef, i64> {
        // /proc/self/fd/N: a fresh open of the same file (new access mode, e.g. a read-only reopen);
        // pipes, sockets and devices are shared as they are.
        if let Some(f) = Self::proc_fd(p, abs) {
            let f = f?;
            let target = match &f.borrow().kind {
                FKind::Reg(path) => Some(path.clone()),
                _ => None,
            };
            return match target {
                Some(path) => {
                    let f = self.open_path(p, &path, flags & !0o300, mode)?; // no O_CREAT/O_EXCL on the link
                    if let Some(n) = self.unlinked_open.get_mut(&path) {
                        *n += 1; // another open of an unlinked file keeps it alive too
                    }
                    Ok(f)
                }
                None => Ok(f),
            };
        }
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
        // Regular files fill the whole request, as on Linux (up to MAX_RW_COUNT); programs such as
        // Electron's asar reader rely on it. Streams return what they have, at most 1 MiB at a time.
        let reg = match &f.borrow().kind {
            FKind::Reg(path) => Some((path.clone(), at.unwrap_or(f.borrow().off))),
            _ => None,
        };
        let count = match reg {
            Some((path, off)) => {
                // A mapped file's newest bytes (and length) may still be in the page cache.
                let cached = self.page_cache.get(&path).map_or(0, |b| b.borrow().len() as u64);
                let size = self.fs.lstat(&path).map_or(0, |s| s.size).max(cached);
                count.min(size.saturating_sub(off).max(1 << 20)).min(0x7fff_f000)
            }
            None => count.min(1 << 20),
        } as usize;
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
            let (got, fds) = self.sock_pop(i, count);
            for f in fds {
                self.release(f); // read() has nowhere to put passed fds: they are closed, as on Linux
            }
            p.mem.write(buf, &got)?;
            return Ok(Sys::Ret(got.len() as i64));
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
                    if let Some(b) = self.page_cache.get(&path) {
                        b.borrow().read_at(off as usize, &mut data[..n]); // mapped: the cache is newest
                    }
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
                        pipe.gen += 1;
                    }
                    self.wake_where(|t| matches!(t.state, TState::Blocked { wait: Wait::Pipe(q), .. } if q == i));
                    fb = f.borrow_mut();
                    n
                }
                FKind::TimerFd(i) => {
                    let i = *i;
                    drop(fb);
                    if count < 8 {
                        return err(EINVAL);
                    }
                    let n = self.timer_take(i);
                    if n == 0 {
                        if flags & 0o4000 != 0 {
                            return err(EAGAIN);
                        }
                        let at = self.timers[i].next;
                        return Ok(self.block(tid, Wait::Timer(i), nr, a, (at != 0).then_some(at), true));
                    }
                    p.mem.write(buf, &n.to_le_bytes())?;
                    return Ok(Sys::Ret(8));
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
            let n = if self.socks[pe].packet { data.len() } else { room.min(data.len()) };
            if self.socks[pe].wayland.is_some() {
                self.wayland_feed(pe, &data[..n], Vec::new())?;
            } else {
                self.sock_push(pe, &data[..n], Vec::new());
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
                if let Some(&sl) = self.seals.get(&path) {
                    if sl & 0x18 != 0 || (sl & 4 != 0 && o + data.len() as u64 > self.fs.lstat(&path)?.size) {
                        return err(EPERM); // write / future-write / grow seals
                    }
                }
                let n = self.fs.write(&path, o, &data)?;
                if let Some(b) = self.page_cache.get(&path) {
                    b.borrow_mut().write_at(o as usize, &data[..n]);
                }
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
                self.pipes[i].gen += 1;
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
            46 | 47 => self.dontwait(p, a[0], a[2], |s, p| s.msg_socket(p, tid, nr, a)),
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
                if flags & 0x20 == 0 && flags & 1 != 0 {
                    // MAP_SHARED of a regular file: alias its page cache.
                    let (path, acc) = match &self.fd(p, fd)?.borrow() {
                        o => (if let FKind::Reg(path) = &o.kind { Some(path.clone()) } else { None }, o.flags & 3),
                    };
                    if let Some(path) = path {
                        if prot & PROT_WRITE != 0 {
                            if acc == 0 {
                                return err(EACCES); // shared writable map needs a writable fd
                            }
                            if self.seals.get(&path).is_some_and(|s| s & 0x18 != 0) {
                                return err(EPERM); // F_SEAL_WRITE / F_SEAL_FUTURE_WRITE
                            }
                        }
                        let fixed = flags & 0x10 != 0 || flags & 0x10_0000 != 0;
                        let at = if fixed {
                            if addr % PAGE != 0 || addr == 0 || addr.checked_add(ceil(len)).is_none_or(|e| e > USER_TOP) {
                                return err(EINVAL);
                            }
                            if flags & 0x10_0000 != 0 && !p.mem.is_free(addr, addr + ceil(len)) {
                                return err(EEXIST);
                            }
                            Some(addr)
                        } else if addr != 0 && addr % PAGE == 0 && addr.checked_add(ceil(len)).is_some_and(|e| e <= USER_TOP) && p.mem.is_free(addr, addr + ceil(len)) {
                            Some(addr)
                        } else {
                            None
                        };
                        let buf = self.cached_file(&path)?;
                        let at = p.mem.map_shared_buf(at, len, prot, buf, off as usize, &path)?;
                        p.fmaps.push((at, len, path, off));
                        return ok(at as i64);
                    }
                }
                let mut init = Vec::new();
                let mut name = String::new();
                let mut file_backing: Option<(String, u64)> = None;
                if flags & 0x20 == 0 {
                    let f = self.fd(p, fd)?;
                    let fb = f.borrow();
                    // Only the file's bytes are read; the rest of the mapping stays demand-zero.
                    let avail = match &fb.kind {
                        FKind::Reg(path) => self.fs.lstat(path)?.size.saturating_sub(off),
                        FKind::Gen(d) => (d.len() as u64).saturating_sub(off),
                        _ => 0,
                    };
                    let want = len.min(avail) as usize;
                    init = vec![0u8; want];
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
                self.reap_unlinked();
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
                if let Some((buf, off, name)) = p.mem.shared_at(old) {
                    // A shared mapping stays an alias of the same pages (a memfd the compositor reads,
                    // shared memory between processes): grow or move it, never copy it.
                    let at = if p.mem.is_free(old + ol, old + nl) {
                        p.mem.unmap(old, ol)?;
                        p.mem.map_shared_buf(Some(old), nl, prot, buf, off, &name)?
                    } else if a[3] & 1 != 0 {
                        let n = p.mem.map_shared_buf(None, nl, prot, buf, off, &name)?;
                        p.mem.unmap(old, ol)?;
                        n
                    } else {
                        return err(ENOMEM);
                    };
                    for m in p.fmaps.iter_mut().filter(|m| m.0 == old) {
                        m.0 = at;
                        m.1 = nl;
                    }
                    return ok(at as i64);
                }
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
            28 => {
                // MADV_DONTNEED (4) and MADV_REMOVE (9) zero private pages; the rest are hints.
                if a[0] % PAGE != 0 {
                    return err(EINVAL);
                }
                if a[2] == 4 || a[2] == 9 {
                    p.mem.discard(a[0], a[1]);
                }
                ok(0)
            }
            149 | 150 | 151 | 152 | 153 => ok(0), // mlock*, munlock*
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
                let fpstate = p.mem.read_u64(mc + 23 * 8)?;
                if fpstate != 0 {
                    let mut fx = [0u8; 512];
                    p.mem.read(fpstate, &mut fx)?;
                    // Reserved MXCSR bits would make FXRSTOR fault in the kernel: keep the defined ones.
                    let mxcsr = u32::from_le_bytes(fx[24..28].try_into().unwrap()) & 0xffff;
                    fx[24..28].copy_from_slice(&mxcsr.to_le_bytes());
                    t.fx = Box::new(fx);
                }
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
                    0x5421 => {
                        // FIONBIO: same as toggling O_NONBLOCK via fcntl
                        let on = p.mem.read_u32(a[2])? != 0;
                        let mut b = f.borrow_mut();
                        b.flags = if on { b.flags | 0o4000 } else { b.flags & !0o4000 };
                        ok(0)
                    }
                    0x5450 | 0x5451 => {
                        // FIONCLEX / FIOCLEX
                        p.fds.get_mut(&(a[0] as i32)).ok_or(EBADF)?.cloexec = a[1] == 0x5451;
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
                self.pipes.push(Pipe { buf: VecDeque::new(), readers: 0, writers: 0, gen: 0 });
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
                        // Record locks: one process owns the files it locks here, so every lock is granted.
                        6 | 7 | 37 | 38 => {
                            self.fd(p, a[0] as i32)?;
                            ok(0)
                        }
                        5 | 36 => {
                            self.fd(p, a[0] as i32)?;
                            p.mem.write(a[2], &2u16.to_le_bytes())?; // l_type = F_UNLCK: no conflicting lock
                            ok(0)
                        }
                        8 | 9 | 15 | 16 => ok(0), // F_SETOWN / F_GETOWN (+_EX): no SIGIO delivery
                        1031 => ok(65536),       // F_SETPIPE_SZ
                        1032 => ok(65536),       // F_GETPIPE_SZ
                        // File seals (memfd): recorded per file; F_SEAL_SEAL (1) forbids adding more.
                        1033 | 1034 => {
                            let path = match &self.fd(p, a[0] as i32)?.borrow().kind {
                                FKind::Reg(x) => x.clone(),
                                _ => return err(EINVAL),
                            };
                            let Some(seals) = self.seals.get(&path).copied() else { return err(EINVAL) }; // only memfds have seals
                            if a[1] == 1034 {
                                return ok(seals as i64);
                            }
                            if seals & 1 != 0 {
                                return err(EPERM);
                            }
                            self.seals.insert(path, seals | a[2] as u32 & 0x1f);
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
                let t = if abs == "/proc/self/exe" {
                    p.exe.clone()
                } else if let Some(f) = Self::proc_fd(p, &abs) {
                    Self::fd_link(&f?)
                } else {
                    self.fs.readlink(&abs)?
                };
                let n = t.len().min(sz as usize);
                p.mem.write(buf, &t.as_bytes()[..n])?;
                ok(n as i64)
            }
            86 | 265 => {
                // link / linkat(olddirfd, old, newdirfd, new, flags)
                let (d1, p1, d2, p2, fl) = if nr == 86 { (AT_FDCWD, a[0], AT_FDCWD, a[1], 0) } else { (a[0] as i32, a[1], a[2] as i32, a[3], a[4]) };
                let (s1, s2) = (p.mem.read_cstr(p1, 4096)?, p.mem.read_cstr(p2, 4096)?);
                let (f, t) = (self.path_for(p, d1, &s1, fl & 0x400 != 0)?, self.path_for(p, d2, &s2, false)?);
                self.fs.link(&f, &t)?;
                ok(0)
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
                        if dir {
                            self.fs.remove(&abs, true)?;
                        } else {
                            self.unlink(p, &abs)?;
                        }
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
            // sched_get_priority_max / _min: SCHED_FIFO and SCHED_RR use 1..99, the others only 0.
            // capget(header, data): every process runs as root with all capabilities (v3: two 32-bit sets).
            125 => {
                p.mem.write_u32(a[0], 0x2008_0522)?; // _LINUX_CAPABILITY_VERSION_3
                if a[1] != 0 {
                    let set = [u32::MAX, u32::MAX, 0, 0x1ff, 0x1ff, 0];
                    for (k, word) in [set[0], set[1], set[2], set[3], set[4], set[5]].iter().enumerate() {
                        p.mem.write_u32(a[1] + k as u64 * 4, *word)?;
                    }
                }
                ok(0)
            }
            126 => ok(0), // capset: accepted
            // getpriority returns 20 - nice (every process runs at nice 0); setpriority is accepted.
            140 => ok(20),
            141 => ok(0),
            // sched_setparam / sched_setscheduler accepted; every thread is SCHED_OTHER, priority 0.
            142 | 144 => ok(0),
            143 => {
                p.mem.write(a[1], &0u32.to_le_bytes())?;
                ok(0)
            }
            145 => ok(0),
            146 | 147 => match a[0] {
                1 | 2 => ok(if nr == 146 { 99 } else { 1 }),
                0 | 3 | 5 | 6 => ok(0),
                _ => err(EINVAL),
            },
            285 => {
                // fallocate(fd, mode, offset, len): mode 0 grows the file; punching/zeroing ranges is accepted as-is.
                let path = match &self.fd(p, a[0] as i32)?.borrow().kind {
                    FKind::Reg(x) => x.clone(),
                    _ => return err(ENODEV),
                };
                let end = a[2].checked_add(a[3]).ok_or(EFBIG)?;
                if a[1] == 0 && end > self.fs.lstat(&path)?.size {
                    self.fs.truncate(&path, end)?;
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
                if let Some(&sl) = self.seals.get(&path) {
                    let size = self.fs.lstat(&path)?.size;
                    if (sl & 2 != 0 && a[1] < size) || (sl & 4 != 0 && a[1] > size) {
                        return err(EPERM); // F_SEAL_SHRINK / F_SEAL_GROW
                    }
                }
                self.fs.truncate(&path, a[1])?;
                ok(0)
            }
            74 | 75 | 73 | 90 | 91 | 92 | 93 | 94 | 268 | 306 | 266 => ok(0), // fsync, flock, chmod/chown family
            319 => {
                // Flags: CLOEXEC, ALLOW_SEALING, HUGETLB (with a size in bits 26..32), NOEXEC_SEAL, EXEC.
                let f = a[1] as u32;
                if f & !(0x1f | 0x3f << 26) != 0 || (f & 0x3f << 26 != 0 && f & 4 == 0) {
                    return err(EINVAL);
                }
                let _name = p.mem.read_cstr(a[0], 256)?;
                let path = format!("/tmp/.loos-memfd-{}-{}", p.pid, self.next_id);
                self.next_id = self.next_id.wrapping_add(1);
                self.fs.create(&path, 0o600)?;
                // MFD_ALLOW_SEALING starts unsealed; otherwise F_SEAL_SEAL forbids adding any.
                self.seals.insert(path.clone(), if a[1] & 2 != 0 { 0 } else { 1 });
                let fd = self.alloc_fd(p, 0)?;
                // MFD_CLOEXEC (1) is the close-on-exec flag; the file itself is read-write.
                p.fds.insert(fd, FdEnt { file: self.new_fd(FKind::Reg(path), 2), cloexec: a[1] & 1 != 0 });
                ok(fd as i64)
            }
            // identity
            39 | 186 => ok(if nr == 39 { p.pid } else { tid } as i64),
            110 => ok(p.ppid as i64),
            102 | 104 | 107 | 108 => ok(crate::fs::USER_ID as i64), // get[e]uid / get[e]gid
            105 | 106 | 113 | 114 => ok(0), // set*id: already that user
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
            57 => self.sys_clone(p, tid, [17, 0, 0, 0, 0, 0]),
            // vfork: copied address space (safe even when the child scribbles on the shared stack), parent waits for exec/exit.
            58 => self.sys_clone(p, tid, [0x4000 | 17, 0, 0, 0, 0, 0]),
            59 => {
                let r = self.sys_execve(p, tid, a);
                if let Err(e) = r {
                    let path = p.mem.read_cstr(a[0], 4096).unwrap_or_default();
                    self.diag += &format!("lin: [{tid}] execve {path} failed: errno {e}\n");
                    if e == EFAULT {
                        let (argv, envp) = (Self::read_strv(&mut p.mem, a[1]).map(|v| v.len()), Self::read_strv(&mut p.mem, a[2]).map(|v| v.len()));
                        self.diag += &format!("lin:   argv {argv:?} envp {envp:?}\n{}", self.recent_syscalls(tid));
                    }
                }
                r
            }
            60 => self.sys_exit(p, tid, a[0] as i32, false),
            231 => self.sys_exit(p, tid, a[0] as i32, true),
            61 => self.sys_wait4(p, tid, nr, a),
            247 => self.sys_waitid(p, tid, nr, a),
            137 | 138 => {
                // statfs / fstatfs: one tmpfs-like filesystem backs the whole tree.
                if nr == 137 {
                    let path = p.mem.read_cstr(a[0], 4096)?;
                    let abs = self.path_for(p, AT_FDCWD, &path, true)?;
                    self.stat_abs(p, &abs)?;
                } else {
                    self.fd(p, a[0] as i32)?;
                }
                let mut b = [0u8; 120];
                for (o, v) in [(0usize, 0x0102_1994u64), (8, 4096), (16, 1 << 20), (24, 1 << 19), (32, 1 << 19), (40, 1 << 20), (48, 1 << 19), (64, 255), (72, 4096)] {
                    b[o..o + 8].copy_from_slice(&v.to_le_bytes());
                }
                p.mem.write(a[1], &b)?;
                Ok(Sys::Ret(0))
            }
            221 => Ok(Sys::Ret(0)), // fadvise64: advisory
            157 => match a[0] {
                // prctl: PDEATHSIG, DUMPABLE, KEEPCAPS, NO_NEW_PRIVS, CHILD_SUBREAPER accepted; names kept per thread elsewhere.
                // PR_SET_VMA (0x53564d41): naming anonymous mappings is cosmetic.
                1 | 4 | 8 | 36 | 38 | 39 | 0x53564d41 => Ok(Sys::Ret(0)),
                2 | 3 | 37 => Ok(Sys::Ret(if a[0] == 3 { 1 } else { 0 })),
                15 => {
                    // Linux copies at most 16 bytes and truncates longer names silently.
                    let mut raw = Vec::new();
                    for i in 0..15 {
                        let mut c = [0u8];
                        p.mem.read(a[1] + i, &mut c)?;
                        if c[0] == 0 {
                            break;
                        }
                        raw.push(c[0]);
                    }
                    let name = String::from_utf8_lossy(&raw).into_owned();
                    if let Some(t) = self.threads.get_mut(&tid) {
                        t.name = name;
                    }
                    Ok(Sys::Ret(0))
                }
                16 => {
                    let own = self.threads.get(&tid).map(|t| t.name.clone()).unwrap_or_default();
                    let n = if own.is_empty() { p.exe.rsplit('/').next().unwrap_or("") } else { own.as_str() }.as_bytes();
                    let mut b = [0u8; 16];
                    let k = n.len().min(15);
                    b[..k].copy_from_slice(&n[..k]);
                    p.mem.write(a[1], &b)?;
                    Ok(Sys::Ret(0))
                }
                _ => err(EINVAL),
            },
            436 => {
                // close_range(first, last, flags): CLOSE_RANGE_CLOEXEC (4) marks instead of closing.
                let (lo, hi) = (a[0] as i64, (a[1] as u32) as i64);
                let fds: Vec<i32> = p.fds.keys().copied().filter(|&f| (f as i64) >= lo && (f as i64) <= hi).collect();
                for f in fds {
                    if a[2] & 4 != 0 {
                        p.fds.get_mut(&f).unwrap().cloexec = true;
                    } else if let Some(e) = p.fds.remove(&f) {
                        self.release(e.file);
                    }
                }
                ok(0)
            }
            44 | 45 => self.dontwait(p, a[0], a[3], |s, p| s.sys_socket(p, tid, nr, a)),
            41..=55 | 288 => self.sys_socket(p, tid, nr, a),
            7 | 23 | 270 | 271 | 213 | 291 | 233 | 232 | 281 | 284 | 290 | 118 | 120 | 307 | 253..=255 | 294 | 283 | 286 | 287 => self.sys_io(p, tid, nr, a),
            280 => {
                // utimensat(dirfd, path, times, flags): timestamps are not stored; the target must exist.
                if a[1] == 0 {
                    self.fd(p, a[0] as i32)?;
                } else {
                    let path = p.mem.read_cstr(a[1], 4096)?;
                    let abs = self.path_for(p, a[0] as i32, &path, a[3] & 0x100 == 0)?;
                    self.stat_abs(p, &abs)?;
                }
                ok(0)
            }
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

    /// Run a send/recv call with `MSG_DONTWAIT` (0x40) in `msg_flags` as if the fd were O_NONBLOCK.
    fn dontwait(&mut self, p: &mut Process, fd: u64, msg_flags: u64, call: impl FnOnce(&mut Self, &mut Process) -> Result<Sys, i64>) -> Result<Sys, i64> {
        let f = match self.fd(p, fd as i32) {
            Ok(f) if msg_flags & 0x40 != 0 && f.borrow().flags & 0o4000 == 0 => f,
            _ => return call(self, p),
        };
        f.borrow_mut().flags |= 0o4000;
        let r = call(self, p);
        f.borrow_mut().flags &= !0o4000;
        r
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
        // Local sockets: the whole iovec array (header + payload buffers are common, e.g. Firefox IPC).
        let iov = p.mem.read_u64(msg + 16)?;
        let iovlen = p.mem.read_u64(msg + 24)?;
        if iovlen > 1024 {
            return err(EINVAL);
        }
        let mut iovs = Vec::with_capacity(iovlen as usize);
        for k in 0..iovlen {
            iovs.push((p.mem.read_u64(iov + k * 16)?, p.mem.read_u64(iov + k * 16 + 8)? as usize));
        }
        let len = iovs.iter().map(|v| v.1).sum::<usize>().min(1 << 22);
        if nr == 46 {
            let mut data = Vec::with_capacity(len);
            for &(b, l) in &iovs {
                let l = l.min(len - data.len());
                data.extend(p.mem.read_vec(b, l)?);
            }
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
                self.wayland_feed(pe, &data, rights)?;
            } else {
                self.sock_push(pe, &data, rights);
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
        let (bytes, fds) = self.sock_pop(si, len);
        let n = bytes.len();
        let mut done = 0;
        for &(b, l) in &iovs {
            if done == n {
                break;
            }
            let k = l.min(n - done);
            p.mem.write(b, &bytes[done..done + k])?;
            done += k;
        }
        let control = p.mem.read_u64(msg + 32)?;
        let control_len = p.mem.read_u64(msg + 40)? as usize;
        let mut flags = 0u32;
        let mut used = 0usize;
        if !fds.is_empty() {
            // As many fds as fit in the control buffer; the rest are closed (MSG_CTRUNC).
            let fit = if control == 0 { 0 } else { control_len.saturating_sub(16) / 4 };
            let given = fds.len().min(fit);
            if given < fds.len() {
                flags |= 8;
            }
            let mut it = fds.into_iter();
            if given > 0 {
                let clen = 16 + given * 4;
                let mut c = vec![0u8; (clen + 7) & !7];
                c[..8].copy_from_slice(&(clen as u64).to_ne_bytes());
                c[8..12].copy_from_slice(&1i32.to_ne_bytes());
                c[12..16].copy_from_slice(&1i32.to_ne_bytes());
                for j in 0..given {
                    let fd = self.alloc_fd(p, 0)?;
                    p.fds.insert(fd, FdEnt { file: it.next().unwrap(), cloexec: a[2] & 0x4000_0000 != 0 });
                    c[16 + j * 4..20 + j * 4].copy_from_slice(&(fd as i32).to_ne_bytes());
                }
                used = c.len().min(control_len);
                p.mem.write(control, &c[..used])?;
            }
            for f in it {
                self.release(f);
            }
        }
        p.mem.write_u64(msg + 40, used as u64)?;
        p.mem.write_u32(msg + 48, flags)?;
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
                if dom == 1 && (ty == 2 || ty == 5) {
                    // local datagram / sequenced-packet socket
                    self.socks.push(Sock { packet: true, ..Sock::default() });
                    let i = self.socks.len() - 1;
                    return Ok(Sys::Ret(new_sock(self, p, i, a[1] & 0o2000000 != 0, a[1] & 0o4000 != 0)?));
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
                let ty = a[1] & 0xf;
                if a[0] != 1 {
                    return err(97);
                }
                if !matches!(ty, 1 | 2 | 5) {
                    return err(93); // EPROTONOSUPPORT
                }
                let packet = ty != 1;
                self.socks.push(Sock { packet, ..Sock::default() });
                self.socks.push(Sock { packet, ..Sock::default() });
                let (x, y) = (self.socks.len() - 2, self.socks.len() - 1);
                self.socks[x].peer = Some(y);
                self.socks[y].peer = Some(x);
                let (cx, nb) = (a[1] & 0o2000000 != 0, a[1] & 0o4000 != 0);
                let fx = new_sock(self, p, x, cx, nb)?;
                let fy = new_sock(self, p, y, cx, nb)?;
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
                let packet = self.socks[l].packet;
                self.socks.push(Sock { packet, ..Sock::default() });
                let srv = self.socks.len() - 1;
                self.socks[srv].key = Some(key.clone());
                self.socks[srv].peer = Some(i);
                self.socks[i].peer = Some(srv);
                if key == "unix:/run/user/1000/wayland-0" {
                    let mut w = WaylandServer::new();
                    w.screen = self.wayland_screen;
                    w.work_area = self.wayland_work_area;
                    w.keymap_size = self.fs.lstat(KEYMAP_PATH).map_or(0, |st| st.size as u32);
                    self.socks[srv].wayland = Some(w);
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
                // getsockopt(fd, level, name, optval, optlen)
                let (i, _) = sid(self, p, a[0])?;
                if a[3] == 0 || a[4] == 0 {
                    return Ok(Sys::Ret(0));
                }
                let inet = self.socks[i].tcp.is_some() || self.socks[i].udp.is_some() || self.socks[i].key.as_deref().is_some_and(|k| !k.starts_with("unix:"));
                let mut out: Vec<u8> = match (a[1], a[2]) {
                    (1, 4) => {
                        // SO_ERROR: the outcome of a non-blocking connect
                        let v = self.socks[i].tcp.map_or(0, |h| match self.stack_mut() {
                            Some(net) if net.tcp_failure(h).is_some() || net.tcp_state(h) == Some(State::Closed) => ECONNREFUSED as u32,
                            _ => 0,
                        });
                        v.to_ne_bytes().to_vec()
                    }
                    (1, 3) => (if self.socks[i].datagram { 2u32 } else { 1 }).to_ne_bytes().to_vec(), // SO_TYPE
                    (1, 7) | (1, 8) => 212_992u32.to_ne_bytes().to_vec(),                               // SO_SNDBUF / SO_RCVBUF
                    (1, 30) => (self.socks[i].listening as u32).to_ne_bytes().to_vec(),                 // SO_ACCEPTCONN
                    (1, 39) => (if inet { 2u32 } else { 1 }).to_ne_bytes().to_vec(),                    // SO_DOMAIN
                    (1, 17) => {
                        // SO_PEERCRED: struct ucred { pid, uid, gid } (every process here runs as root)
                        let mut c = p.pid.to_ne_bytes().to_vec();
                        c.extend_from_slice(&[0; 8]);
                        c
                    }
                    _ => 0u32.to_ne_bytes().to_vec(),
                };
                let cap = p.mem.read_u32(a[4])? as usize;
                out.truncate(cap);
                p.mem.write(a[3], &out)?;
                p.mem.write_u32(a[4], out.len() as u32)?;
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

    /// waitid(idtype, id, siginfo, options): P_ALL / P_PID, WEXITED with WNOHANG / WNOWAIT.
    fn sys_waitid(&mut self, p: &mut Process, tid: Pid, nr: u64, a: [u64; 6]) -> Result<Sys, i64> {
        let (idtype, id, info, opts) = (a[0], a[1] as u32, a[2], a[3]);
        if opts & 4 == 0 || idtype > 1 {
            return err(EINVAL);
        }
        let kids: Vec<(Pid, Option<i32>)> = self.procs.values().filter(|c| c.ppid == p.pid && (idtype == 0 || c.pid == id)).map(|c| (c.pid, c.exit)).collect();
        if kids.is_empty() {
            return err(ECHILD);
        }
        let mut si = [0u8; 128];
        if let Some((pid, Some(st))) = kids.iter().find(|k| k.1.is_some()).copied() {
            let (code, status) = if st & 0x7f == 0 { (1u32, (st >> 8) & 0xff) } else { (2, st & 0x7f) };
            si[0..4].copy_from_slice(&17u32.to_le_bytes());
            si[8..12].copy_from_slice(&code.to_le_bytes());
            si[16..20].copy_from_slice(&pid.to_le_bytes());
            si[24..28].copy_from_slice(&(status as u32).to_le_bytes());
            if info != 0 {
                p.mem.write(info, &si)?;
            }
            if opts & 0x0100_0000 == 0 {
                self.procs.remove(&pid);
            }
            return Ok(Sys::Ret(0));
        }
        if opts & 1 != 0 {
            if info != 0 {
                p.mem.write(info, &si)?;
            }
            return Ok(Sys::Ret(0));
        }
        Ok(self.block(tid, Wait::Child, nr, a, None, true))
    }

    /// Futex identity. Private futexes (`FUTEX_PRIVATE_FLAG`) are per address space; a shared one
    /// in a `MAP_SHARED` mapping is the memory word itself, whichever process maps it where
    /// (process-shared semaphores and mutexes between e.g. Firefox's processes).
    fn futex_key(p: &UserMem, op: u32, addr: u64, pid: Pid) -> (Pid, u64) {
        if op & 128 == 0 {
            if let Some((buf, off, _)) = p.shared_at(addr) {
                let id = Rc::as_ptr(&buf) as usize as u64;
                return (SHARED_FUTEX | (id >> 4) as Pid, (off as u64) + (addr & (PAGE - 1)));
            }
        }
        (pid, addr)
    }

    fn sys_futex(&mut self, p: &mut Process, tid: Pid, nr: u64, a: [u64; 6]) -> Result<Sys, i64> {
        let (uaddr, op, val) = (a[0], a[1] as u32, a[2] as u32);
        if uaddr % 4 != 0 {
            return err(EINVAL);
        }
        let key = Self::futex_key(&p.mem, op, uaddr, p.pid);
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
                Ok(self.block(tid, Wait::Futex(key.0, key.1), nr, a, deadline, false))
            }
            1 | 10 => Ok(Sys::Ret(self.futex_wake(key.0, key.1, val as usize) as i64)),
            3 | 4 => {
                let (uaddr2, nreq) = (a[4], a[3] as usize);
                if op & 127 == 4 && p.mem.read_u32(uaddr)? != a[5] as u32 {
                    return err(EAGAIN);
                }
                let woken = self.futex_wake(key.0, key.1, val as usize);
                let key2 = Self::futex_key(&p.mem, op, uaddr2, p.pid);
                let mut moved = 0;
                let ids: Vec<Pid> = self.threads.values().filter(|t| matches!(t.state, TState::Blocked { wait: Wait::Futex(pp, ad), .. } if (pp, ad) == key)).map(|t| t.tid).collect();
                for t in ids.into_iter().take(nreq) {
                    if let TState::Blocked { wait, .. } = &mut self.threads.get_mut(&t).unwrap().state {
                        *wait = Wait::Futex(key2.0, key2.1);
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
                let mut n = self.futex_wake(key.0, key.1, val as usize);
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
                    let key2 = Self::futex_key(&p.mem, op, a[4], p.pid);
                    n += self.futex_wake(key2.0, key2.1, a[3] as usize);
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
        let parent_fx = self.threads[&tid].fx.clone(); // children start with the parent's FPU state
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
            self.threads.insert(new, Thread { tid: new, pid: p.pid, regs, state: TState::Runnable, sigmask: mask, pending: 0, altstack: (0, 0), clear_tid: clear, robust: 0, resume_deadline: None, fx: parent_fx.clone(), fault: None, name: String::new(), recent: Default::default() });
            return Ok(Sys::Ret(new as i64));
        }
        // fork / vfork / CLONE_VM|CLONE_VFORK (posix_spawn): copy semantics
        if self.procs.len() >= 4096 {
            return err(EAGAIN);
        }
        let vm_shared = flags & 0x100 != 0;
        let mut child = p.clone();
        child.mem = if vm_shared { p.mem.share_all() } else { p.mem.fork() };
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
        self.threads.insert(new, Thread { tid: new, pid: new, regs, state: TState::Runnable, sigmask: mask, pending: 0, altstack: (0, 0), clear_tid: clear, robust: 0, resume_deadline: None, fx: parent_fx.clone(), fault: None, name: String::new(), recent: Default::default() });
        if flags & 0x4000 != 0 {
            return Ok(self.block(tid, Wait::VFork(new), 0, a, None, false));
        }
        Ok(Sys::Ret(new as i64))
    }

    fn sys_execve(&mut self, p: &mut Process, tid: Pid, a: [u64; 6]) -> Result<Sys, i64> {
        let path = p.mem.read_cstr(a[0], 4096)?;
        let argv = Self::read_strv(&mut p.mem, a[1])?;
        let envp = Self::read_strv(&mut p.mem, a[2])?;
        let abs = match path.as_str() {
            // Chromium relaunches itself for every helper process.
            "/proc/self/exe" => p.exe.clone(),
            _ => self.path_for(p, AT_FDCWD, &path, true)?,
        };
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
        self.map_clock_page(&mut mem)?;
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
        t.fx = fx_default();
        t.altstack = (0, 0);
        t.clear_tid = 0;
        t.pending = 0;
        self.vfork_done(p.pid);
        Ok(Sys::Exec)
    }
}

fn rlimit(res: u32) -> (u64, u64) {
    // A typical desktop Linux's defaults (`ulimit -a`); programs cast some of these to int.
    const INF: u64 = u64::MAX;
    match res {
        3 => (STACK_MAX, INF),
        4 => (0, INF),                // core
        6 | 11 => (63_000, 63_000),   // nproc, sigpending
        7 => (1024, 524_288),         // nofile
        8 => (8 << 20, 8 << 20),      // memlock
        12 => (819_200, 819_200),     // msgqueue
        13 | 14 => (0, 0),            // nice, rtprio
        _ => (INF, INF),
    }
}

include!("syscalls_generated.rs");
