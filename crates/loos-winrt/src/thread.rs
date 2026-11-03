//! Cooperative threads and kernel objects (events, mutexes, semaphores, critical sections). The kernel switches threads
//! only at blocking syscalls: a call that cannot complete returns [`Ret::Yield`] and is retried when the thread runs again.

use crate::{Process, Ret};
use alloc::vec::Vec;

/// Syscall number the thread-exit stub uses.
pub const THREAD_EXIT: u32 = 0x0fe;
/// First syscall number of this table.
pub const THR_BASE: u32 = 0x600;
const OBJ_BASE: u64 = 0x20000;
const INFINITE: u64 = 0xffff_ffff;
const WAIT_TIMEOUT: u64 = 0x102;

/// One thread of the process.
pub struct Thread {
    /// Thread id.
    pub tid: u64,
    /// Thread environment block (user GS base).
    pub teb: u64,
    /// Start routine and its argument (new threads only).
    pub entry: u64,
    /// Argument for `entry`.
    pub param: u64,
    /// Initial stack pointer (new threads only).
    pub rsp: u64,
    /// Exit code once finished.
    pub done: Option<u32>,
    pub(crate) deadline: Option<u64>,
    pub(crate) yielded: bool,
    pub(crate) tls: Vec<u64>,
    pub(crate) last_error: u32,
}

impl Thread {
    pub(crate) fn main(tid: u64, teb: u64) -> Thread {
        Thread { tid, teb, entry: 0, param: 0, rsp: 0, done: None, deadline: None, yielded: false, tls: Vec::new(), last_error: 0 }
    }
}

pub(crate) enum Obj {
    Thread(usize),
    Event { signaled: bool, manual: bool },
    Mutex { owner: Option<usize>, count: u32 },
    Sem { count: i32, max: i32 },
}

const NAMES: &[&str] = &[
    "CreateThread", "ExitThread", "GetCurrentThreadId", "CreateEventW", "CreateEventA", "SetEvent", "ResetEvent", "CreateMutexW",
    "CreateMutexA", "ReleaseMutex", "CreateSemaphoreW", "CreateSemaphoreA", "ReleaseSemaphore", "WaitForSingleObject",
    "WaitForMultipleObjects", "Sleep", "SwitchToThread", "GetExitCodeThread", "TlsAlloc", "TlsFree", "TlsGetValue", "TlsSetValue",
    "FlsAlloc", "FlsFree", "FlsGetValue", "FlsSetValue", "EnterCriticalSection", "LeaveCriticalSection", "TryEnterCriticalSection",
    "InitializeCriticalSection", "InitializeCriticalSectionEx", "InitializeCriticalSectionAndSpinCount", "DeleteCriticalSection",
    "SleepEx", "GetLastError", "SetLastError", "ResumeThread", "SuspendThread", "SetThreadPriority", "CloseHandleObj",
];

/// Syscall number of a thread/synchronisation import.
pub fn lookup(name: &str) -> Option<u32> {
    NAMES.iter().position(|n| *n == name).map(|i| THR_BASE + i as u32)
}

fn flip(t: &mut Thread) -> bool {
    let y = t.yielded;
    t.yielded = !y;
    y
}

enum Wait {
    Got,
    Pending,
}

impl Process {
    pub(crate) fn cur_thread(&mut self) -> &mut Thread {
        let i = self.cur;
        &mut self.threads[i]
    }

    /// Thread index to create a kernel frame for (new threads queue here).
    pub fn take_spawn(&mut self) -> Option<usize> {
        if self.spawned.is_empty() { None } else { Some(self.spawned.remove(0)) }
    }

    /// True while some thread is sleeping on a timeout (the scheduler must keep time moving).
    pub fn has_timed_waiters(&self) -> bool {
        self.threads.iter().any(|t| t.done.is_none() && t.deadline.is_some())
    }

    /// Number of threads that have not exited.
    pub fn live_threads(&self) -> usize {
        self.threads.iter().filter(|t| t.done.is_none()).count()
    }

    fn new_obj(&mut self, o: Obj) -> u64 {
        self.objs.push(Some(o));
        OBJ_BASE + (self.objs.len() as u64 - 1) * 4
    }

    fn obj_idx(&self, h: u64) -> Option<usize> {
        let i = h.checked_sub(OBJ_BASE)? as usize / 4;
        self.objs.get(i)?.as_ref().map(|_| i)
    }

    fn try_acquire(&mut self, h: u64) -> Option<Wait> {
        let i = self.obj_idx(h)?;
        let cur = self.cur;
        let mut o = self.objs[i].take()?;
        let r = match &mut o {
            Obj::Thread(t) => {
                if self.threads[*t].done.is_some() { Wait::Got } else { Wait::Pending }
            }
            Obj::Event { signaled, manual } => {
                if *signaled {
                    if !*manual {
                        *signaled = false;
                    }
                    Wait::Got
                } else {
                    Wait::Pending
                }
            }
            Obj::Mutex { owner, count } => {
                if owner.is_none_or(|x| x == cur) {
                    *owner = Some(cur);
                    *count += 1;
                    Wait::Got
                } else {
                    Wait::Pending
                }
            }
            Obj::Sem { count, .. } => {
                if *count > 0 {
                    *count -= 1;
                    Wait::Got
                } else {
                    Wait::Pending
                }
            }
        };
        self.objs[i] = Some(o);
        Some(r)
    }

    /// Wait for `hs` (any or all). `Ok(index)` when satisfied, `Err(true)` timed out, `Err(false)` keep waiting.
    fn wait(&mut self, hs: &[u64], all: bool, ms: u64, now: u64) -> Result<u64, bool> {
        let got = if all {
            // Probe without consuming: all must be ready; acquire only then.
            let ready = hs.iter().all(|&h| match self.obj_idx(h) {
                Some(i) => match &self.objs[i] {
                    Some(Obj::Thread(t)) => self.threads[*t].done.is_some(),
                    Some(Obj::Event { signaled, .. }) => *signaled,
                    Some(Obj::Mutex { owner, .. }) => owner.is_none_or(|x| x == self.cur),
                    Some(Obj::Sem { count, .. }) => *count > 0,
                    None => true,
                },
                None => true,
            });
            if ready {
                for &h in hs {
                    self.try_acquire(h);
                }
                Some(0)
            } else {
                None
            }
        } else {
            hs.iter().enumerate().find_map(|(k, &h)| match self.try_acquire(h) {
                Some(Wait::Pending) => None,
                _ => Some(k as u64), // signalled, or not a waitable object (returns at once)
            })
        };
        if let Some(k) = got {
            self.cur_thread().deadline = None;
            return Ok(k);
        }
        if ms == INFINITE {
            return Err(false);
        }
        let t = self.cur_thread();
        let d = *t.deadline.get_or_insert(now + ms);
        if now >= d {
            t.deadline = None;
            Err(true)
        } else {
            Err(false)
        }
    }

    /// Calls in this table. `None` for numbers outside it.
    pub(crate) fn thread_syscall(&mut self, nr: u32, a: [u64; 4], rsp: u64, now_ms: u64) -> Option<Ret> {
        if nr == THREAD_EXIT {
            let code = a[0] as u32;
            let i = self.cur;
            self.threads[i].done = Some(code);
            return Some(Ret::ThreadExit(code));
        }
        let name = *NAMES.get(nr.checked_sub(THR_BASE)? as usize)?;
        let v = |x: u64| Some(Ret::Value(x));
        match name {
            "CreateThread" => {
                let stack = if a[1] == 0 { 1 << 20 } else { a[1].clamp(1 << 16, 16 << 20) };
                let base = self.mem.map_anywhere(stack, 3, &[], "tstack").ok()?;
                let top = base + stack;
                let teb = self.mem.map_anywhere(0x3000, 3, &[], "teb").ok()?;
                let tid = 0x1000 + 4 * self.threads.len() as u64;
                let mut tb = alloc::vec![0u8; 0x3000];
                let put = |b: &mut Vec<u8>, o: usize, v: u64| b[o..o + 8].copy_from_slice(&v.to_le_bytes());
                put(&mut tb, 0x08, top);
                put(&mut tb, 0x10, base);
                put(&mut tb, 0x30, teb);
                put(&mut tb, 0x40, self.pid);
                put(&mut tb, 0x48, tid);
                put(&mut tb, 0x60, self.teb + 0x1000);
                self.mem.write(teb, &tb).ok()?;
                let sp = top - 0x28;
                self.mem.write(sp, &self.thread_exit_stub.to_le_bytes()).ok()?;
                let mut t = Thread::main(tid, teb);
                (t.entry, t.param, t.rsp) = (a[2], a[3], sp);
                self.threads.push(t);
                let idx = self.threads.len() - 1;
                self.spawned.push(idx);
                let tid_out = self.stack_arg(rsp, 6);
                if tid_out != 0 {
                    let _ = self.mem.write(tid_out, &(tid as u32).to_le_bytes());
                }
                v(self.new_obj(Obj::Thread(idx)))
            }
            "ExitThread" => self.thread_syscall(THREAD_EXIT, a, rsp, now_ms),
            "GetCurrentThreadId" => v(self.threads[self.cur].tid),
            "CreateEventW" | "CreateEventA" => v(self.new_obj(Obj::Event { signaled: a[2] != 0, manual: a[1] != 0 })),
            "SetEvent" | "ResetEvent" => {
                if let Some(i) = self.obj_idx(a[0]) {
                    if let Some(Obj::Event { signaled, .. }) = &mut self.objs[i] {
                        *signaled = name == "SetEvent";
                    }
                }
                v(1)
            }
            "CreateMutexW" | "CreateMutexA" => {
                let h = self.new_obj(Obj::Mutex { owner: None, count: 0 });
                if a[1] != 0 {
                    self.try_acquire(h);
                }
                v(h)
            }
            "ReleaseMutex" => {
                let cur = self.cur;
                if let Some(i) = self.obj_idx(a[0]) {
                    if let Some(Obj::Mutex { owner, count }) = &mut self.objs[i] {
                        if *owner == Some(cur) {
                            *count -= 1;
                            if *count == 0 {
                                *owner = None;
                            }
                            return v(1);
                        }
                    }
                }
                self.set_last_error(288);
                v(0)
            }
            "CreateSemaphoreW" | "CreateSemaphoreA" => v(self.new_obj(Obj::Sem { count: a[1] as i32, max: a[2] as i32 })),
            "ReleaseSemaphore" => {
                if let Some(i) = self.obj_idx(a[0]) {
                    if let Some(Obj::Sem { count, max }) = &mut self.objs[i] {
                        let prev = *count;
                        if prev + a[1] as i32 > *max {
                            return v(0);
                        }
                        *count += a[1] as i32;
                        if a[2] != 0 {
                            let _ = self.mem.write(a[2], &prev.to_le_bytes());
                        }
                        return v(1);
                    }
                }
                v(0)
            }
            "WaitForSingleObject" => match self.wait(&[a[0]], false, a[1] & 0xffff_ffff, now_ms) {
                Ok(_) => v(0),
                Err(true) => v(WAIT_TIMEOUT),
                Err(false) => Some(Ret::Yield),
            },
            "WaitForMultipleObjects" => {
                let hs: Vec<u64> = (0..a[0].min(64)).map(|i| self.mem.read_u64(a[1] + i * 8).unwrap_or(0)).collect();
                let ms = a[3] & 0xffff_ffff;
                match self.wait(&hs, a[2] != 0, ms, now_ms) {
                    Ok(k) => v(k),
                    Err(true) => v(WAIT_TIMEOUT),
                    Err(false) => Some(Ret::Yield),
                }
            }
            "Sleep" | "SleepEx" => {
                let ms = a[0] & 0xffff_ffff;
                if ms == INFINITE {
                    return Some(Ret::Park);
                }
                let t = self.cur_thread();
                if ms == 0 {
                    return if flip(t) { v(0) } else { Some(Ret::Yield) };
                }
                let d = *t.deadline.get_or_insert(now_ms + ms);
                if now_ms >= d {
                    t.deadline = None;
                    v(0)
                } else {
                    Some(Ret::Yield)
                }
            }
            "SwitchToThread" => {
                let t = self.cur_thread();
                if flip(t) { v(0) } else { Some(Ret::Yield) }
            }
            "GetExitCodeThread" => {
                let code = self.obj_idx(a[0]).and_then(|i| match &self.objs[i] {
                    Some(Obj::Thread(t)) => Some(self.threads[*t].done.map_or(259, u64::from)),
                    _ => None,
                });
                if let Some(c) = code {
                    let _ = self.mem.write(a[1], &(c as u32).to_le_bytes());
                }
                v(u64::from(code.is_some()))
            }
            "TlsAlloc" | "FlsAlloc" => {
                self.tls_count += 1;
                v(u64::from(self.tls_count - 1))
            }
            "TlsFree" | "FlsFree" => v(1),
            "TlsGetValue" | "FlsGetValue" => v(self.cur_thread().tls.get(a[0] as usize).copied().unwrap_or(0)),
            "TlsSetValue" | "FlsSetValue" => {
                let t = self.cur_thread();
                if t.tls.len() <= a[0] as usize {
                    t.tls.resize(a[0] as usize + 1, 0);
                }
                t.tls[a[0] as usize] = a[1];
                v(1)
            }
            "InitializeCriticalSection" | "InitializeCriticalSectionEx" | "InitializeCriticalSectionAndSpinCount" => {
                let _ = self.mem.write(a[0], &[0u8; 40]);
                v(1)
            }
            "DeleteCriticalSection" => v(0),
            "EnterCriticalSection" | "TryEnterCriticalSection" => {
                // CRITICAL_SECTION: RecursionCount @12 (i32), OwningThread @16.
                let me = self.threads[self.cur].tid;
                let owner = self.mem.read_u64(a[0] + 16).unwrap_or(0);
                if owner == 0 || owner == me {
                    let rec = self.mem.read_u32(a[0] + 12).unwrap_or(0);
                    let _ = self.mem.write(a[0] + 12, &(rec + 1).to_le_bytes());
                    let _ = self.mem.write(a[0] + 16, &me.to_le_bytes());
                    return v(1);
                }
                if name == "TryEnterCriticalSection" { v(0) } else { Some(Ret::Yield) }
            }
            "LeaveCriticalSection" => {
                let rec = self.mem.read_u32(a[0] + 12).unwrap_or(1).saturating_sub(1);
                let _ = self.mem.write(a[0] + 12, &rec.to_le_bytes());
                if rec == 0 {
                    let _ = self.mem.write(a[0] + 16, &0u64.to_le_bytes());
                }
                v(0)
            }
            "GetLastError" => v(u64::from(self.last_error())),
            "SetLastError" => {
                self.set_last_error(a[0] as u32);
                v(0)
            }
            "ResumeThread" | "SuspendThread" | "SetThreadPriority" => v(0),
            _ => None,
        }
    }

    /// `CloseHandle` for kernel objects. Returns false if `h` is not one.
    pub(crate) fn close_obj(&mut self, h: u64) -> bool {
        match self.obj_idx(h) {
            Some(i) => {
                self.objs[i] = None;
                true
            }
            None => false,
        }
    }
}
