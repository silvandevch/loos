//! NT-style dispatcher objects and waits.

use alloc::vec::Vec;

/// Identifies a thread to the dispatcher (opaque to it).
pub type ThreadId = u64;

/// Handle to a dispatcher object; stale handles are rejected after [`Dispatcher::destroy`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObjId {
    index: u32,
    generation: u32,
}

impl ObjId {
    /// Non-zero 64-bit encoding for C callers (bit 63 is always set, so 0 can mean "no object").
    pub const fn to_raw(self) -> u64 {
        1 << 63 | (self.generation as u64) << 32 | self.index as u64
    }

    /// Inverse of [`ObjId::to_raw`]; `None` for 0 or values without the marker bit.
    pub const fn from_raw(raw: u64) -> Option<ObjId> {
        if raw >> 63 == 0 {
            return None;
        }
        Some(ObjId { index: raw as u32, generation: (raw >> 32) as u32 & 0x7fff_ffff })
    }
}

/// Event reset behaviour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    /// Stays signaled until explicitly reset; releases every waiter.
    Notification,
    /// Resets automatically after releasing a single waiter.
    Synchronization,
}

/// How a multi-object wait is satisfied.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitKind {
    /// Any one object being signaled satisfies the wait.
    Any,
    /// All objects must be signaled simultaneously.
    All,
}

/// Dispatcher failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The handle does not name a live object.
    InvalidHandle,
    /// The object is not of the kind the operation needs.
    WrongType,
    /// A mutex was released by a thread that does not own it.
    NotOwner,
    /// A semaphore release would exceed its maximum count.
    LimitExceeded,
    /// The thread already has an outstanding wait.
    AlreadyWaiting,
    /// The wait list is empty or contains duplicates.
    BadWaitList,
    /// A mutex was acquired recursively too many times.
    RecursionLimit,
    /// Object is still referenced by waiting threads.
    InUse,
}

/// Outcome of a completed wait; indices refer to the wait list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitStatus {
    /// Object `i` satisfied the wait (for `All`, `0`).
    Signaled(usize),
    /// Mutex `i` was acquired but its previous owner died holding it.
    Abandoned(usize),
    /// The timeout elapsed first.
    Timeout,
    /// The wait was cancelled by [`Dispatcher::cancel_wait`].
    Cancelled,
    /// An alertable wait was ended by [`Dispatcher::alert`] (APC delivery).
    Alerted,
}

/// Result of starting a wait.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaitStart {
    /// Completed without blocking.
    Done(WaitStatus),
    /// The thread must block; a [`Wakeup`] will be produced later.
    Blocked,
}

/// A blocked thread that may run again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Wakeup {
    /// The thread to make ready.
    pub thread: ThreadId,
    /// How its wait ended.
    pub status: WaitStatus,
}

const MAX_RECURSION: u32 = 0x7fff_ffff;

enum Kind {
    Event { kind: EventKind, signaled: bool },
    Semaphore { count: u32, max: u32 },
    Mutex { owner: Option<ThreadId>, recursion: u32, abandoned: bool },
    Timer { kind: EventKind, signaled: bool, due: Option<u64>, period: u64 },
}

struct Slot {
    generation: u32,
    obj: Option<Kind>,
}

struct Waiter {
    thread: ThreadId,
    objs: Vec<ObjId>,
    kind: WaitKind,
    deadline: Option<u64>,
    alertable: bool,
}

/// Owns all dispatcher objects and the queue of blocked waiters.
#[derive(Default)]
pub struct Dispatcher {
    slots: Vec<Slot>,
    free: Vec<u32>,
    waiters: Vec<Waiter>,
    ready: Vec<Wakeup>,
    now: u64,
}

impl Dispatcher {
    /// Creates an empty dispatcher at time zero.
    pub fn new() -> Self {
        Self::default()
    }

    /// Current time in ticks.
    pub fn now(&self) -> u64 {
        self.now
    }

    fn alloc(&mut self, k: Kind) -> ObjId {
        if let Some(i) = self.free.pop() {
            let s = &mut self.slots[i as usize];
            s.obj = Some(k);
            return ObjId { index: i, generation: s.generation };
        }
        self.slots.push(Slot { generation: 0, obj: Some(k) });
        ObjId { index: (self.slots.len() - 1) as u32, generation: 0 }
    }

    fn get(&self, id: ObjId) -> Result<&Kind, Error> {
        match self.slots.get(id.index as usize) {
            Some(s) if s.generation == id.generation => s.obj.as_ref().ok_or(Error::InvalidHandle),
            _ => Err(Error::InvalidHandle),
        }
    }

    fn get_mut(&mut self, id: ObjId) -> Result<&mut Kind, Error> {
        match self.slots.get_mut(id.index as usize) {
            Some(s) if s.generation == id.generation => s.obj.as_mut().ok_or(Error::InvalidHandle),
            _ => Err(Error::InvalidHandle),
        }
    }

    /// Creates an event.
    pub fn create_event(&mut self, kind: EventKind, signaled: bool) -> ObjId {
        self.alloc(Kind::Event { kind, signaled })
    }

    /// Creates a counting semaphore. `initial` must not exceed `max` and `max` must be non-zero.
    pub fn create_semaphore(&mut self, initial: u32, max: u32) -> Result<ObjId, Error> {
        if max == 0 || initial > max {
            return Err(Error::LimitExceeded);
        }
        Ok(self.alloc(Kind::Semaphore { count: initial, max }))
    }

    /// Creates a (recursive) mutex, optionally owned by `owner`.
    pub fn create_mutex(&mut self, owner: Option<ThreadId>) -> ObjId {
        self.alloc(Kind::Mutex { owner, recursion: owner.is_some() as u32, abandoned: false })
    }

    /// Destroys an object; fails while threads wait on it.
    pub fn destroy(&mut self, id: ObjId) -> Result<(), Error> {
        self.get(id)?;
        if self.waiters.iter().any(|w| w.objs.contains(&id)) {
            return Err(Error::InUse);
        }
        let s = &mut self.slots[id.index as usize];
        s.obj = None;
        s.generation = s.generation.wrapping_add(1);
        self.free.push(id.index);
        Ok(())
    }

    /// Creates a waitable timer (not signaled, not armed). `kind` picks
    /// manual-reset (`Notification`) or auto-reset (`Synchronization`).
    pub fn create_timer(&mut self, kind: EventKind) -> ObjId {
        self.alloc(Kind::Timer { kind, signaled: false, due: None, period: 0 })
    }

    /// Arms a timer `delay` ticks from now, repeating every `period` ticks when
    /// non-zero. Clears the signal. Returns whether it was already armed.
    pub fn set_timer(&mut self, id: ObjId, delay: u64, period: u64) -> Result<bool, Error> {
        let now = self.now;
        match self.get_mut(id)? {
            Kind::Timer { signaled, due, period: p, .. } => {
                *signaled = false;
                *p = period;
                let was = due.replace(now.saturating_add(delay)).is_some();
                Ok(was)
            }
            _ => Err(Error::WrongType),
        }
        .inspect(|_| if delay == 0 { self.fire_timers() })
    }

    /// Disarms a timer, leaving its signal state. Returns whether it was armed.
    pub fn cancel_timer(&mut self, id: ObjId) -> Result<bool, Error> {
        match self.get_mut(id)? {
            Kind::Timer { due, .. } => Ok(due.take().is_some()),
            _ => Err(Error::WrongType),
        }
    }

    /// Signal state of a timer.
    pub fn timer_state(&self, id: ObjId) -> Result<bool, Error> {
        match self.get(id)? {
            Kind::Timer { signaled, .. } => Ok(*signaled),
            _ => Err(Error::WrongType),
        }
    }

    /// Signals every timer whose due time has passed and re-arms periodic ones.
    /// A periodic timer that fell several periods behind fires once.
    fn fire_timers(&mut self) {
        let now = self.now;
        let mut any = false;
        for s in &mut self.slots {
            if let Some(Kind::Timer { signaled, due: due @ Some(_), period, .. }) = s.obj.as_mut() {
                let d = due.unwrap_or(0);
                if d <= now {
                    *signaled = true;
                    any = true;
                    *due = if *period == 0 { None } else { Some(d + ((now - d) / *period + 1) * *period) };
                }
            }
        }
        if any {
            self.satisfy();
        }
    }

    /// Signals an event, releasing waiters per its kind.
    pub fn set_event(&mut self, id: ObjId) -> Result<bool, Error> {
        match self.get_mut(id)? {
            Kind::Event { signaled, .. } => {
                let prev = core::mem::replace(signaled, true);
                self.satisfy();
                Ok(prev)
            }
            _ => Err(Error::WrongType),
        }
    }

    /// Clears an event; returns whether it was signaled.
    pub fn reset_event(&mut self, id: ObjId) -> Result<bool, Error> {
        match self.get_mut(id)? {
            Kind::Event { signaled, .. } => Ok(core::mem::replace(signaled, false)),
            _ => Err(Error::WrongType),
        }
    }

    /// Releases the currently waiting threads (all for notification events, one for
    /// synchronization events) and leaves the event non-signaled.
    pub fn pulse_event(&mut self, id: ObjId) -> Result<(), Error> {
        self.set_event(id)?;
        self.reset_event(id)?;
        Ok(())
    }

    /// Whether an event is currently signaled.
    pub fn event_state(&self, id: ObjId) -> Result<bool, Error> {
        match self.get(id)? {
            Kind::Event { signaled, .. } => Ok(*signaled),
            _ => Err(Error::WrongType),
        }
    }

    /// Adds `n` to a semaphore, returning the previous count.
    pub fn release_semaphore(&mut self, id: ObjId, n: u32) -> Result<u32, Error> {
        match self.get_mut(id)? {
            Kind::Semaphore { count, max } => {
                if n == 0 || n > *max - *count {
                    return Err(Error::LimitExceeded);
                }
                let prev = *count;
                *count += n;
                self.satisfy();
                Ok(prev)
            }
            _ => Err(Error::WrongType),
        }
    }

    /// Current semaphore count.
    pub fn semaphore_count(&self, id: ObjId) -> Result<u32, Error> {
        match self.get(id)? {
            Kind::Semaphore { count, .. } => Ok(*count),
            _ => Err(Error::WrongType),
        }
    }

    /// Releases one level of a mutex held by `thread`; returns the remaining recursion depth.
    pub fn release_mutex(&mut self, id: ObjId, thread: ThreadId) -> Result<u32, Error> {
        match self.get_mut(id)? {
            Kind::Mutex { owner, recursion, .. } => {
                if *owner != Some(thread) {
                    return Err(Error::NotOwner);
                }
                *recursion -= 1;
                if *recursion == 0 {
                    *owner = None;
                }
                let left = *recursion;
                if left == 0 {
                    self.satisfy();
                }
                Ok(left)
            }
            _ => Err(Error::WrongType),
        }
    }

    /// Current owner of a mutex.
    pub fn mutex_owner(&self, id: ObjId) -> Result<Option<ThreadId>, Error> {
        match self.get(id)? {
            Kind::Mutex { owner, .. } => Ok(*owner),
            _ => Err(Error::WrongType),
        }
    }

    fn signaled(k: &Kind, thread: ThreadId) -> bool {
        match k {
            Kind::Event { signaled, .. } | Kind::Timer { signaled, .. } => *signaled,
            Kind::Semaphore { count, .. } => *count > 0,
            Kind::Mutex { owner, recursion, .. } => owner.is_none() || (*owner == Some(thread) && *recursion < MAX_RECURSION),
        }
    }

    /// Takes ownership/count from a signaled object; returns whether it was abandoned.
    fn consume(k: &mut Kind, thread: ThreadId) -> bool {
        match k {
            Kind::Event { kind, signaled } | Kind::Timer { kind, signaled, .. } => {
                if *kind == EventKind::Synchronization {
                    *signaled = false;
                }
                false
            }
            Kind::Semaphore { count, .. } => {
                *count -= 1;
                false
            }
            Kind::Mutex { owner, recursion, abandoned } => {
                *owner = Some(thread);
                *recursion += 1;
                core::mem::replace(abandoned, false)
            }
        }
    }

    /// Tries to satisfy a wait right now, consuming what it takes.
    fn try_satisfy(&mut self, thread: ThreadId, objs: &[ObjId], kind: WaitKind) -> Option<WaitStatus> {
        match kind {
            WaitKind::Any => {
                for (i, &o) in objs.iter().enumerate() {
                    if Self::signaled(self.get(o).ok()?, thread) {
                        let ab = Self::consume(self.get_mut(o).ok()?, thread);
                        return Some(if ab { WaitStatus::Abandoned(i) } else { WaitStatus::Signaled(i) });
                    }
                }
                None
            }
            WaitKind::All => {
                for &o in objs {
                    if !Self::signaled(self.get(o).ok()?, thread) {
                        return None;
                    }
                }
                let mut abandoned = None;
                for (i, &o) in objs.iter().enumerate() {
                    if Self::consume(self.get_mut(o).ok()?, thread) && abandoned.is_none() {
                        abandoned = Some(i);
                    }
                }
                Some(abandoned.map_or(WaitStatus::Signaled(0), WaitStatus::Abandoned))
            }
        }
    }

    /// Starts a wait. `timeout` of `Some(0)` polls; `None` waits forever.
    pub fn wait(&mut self, thread: ThreadId, objs: &[ObjId], kind: WaitKind, timeout: Option<u64>) -> Result<WaitStart, Error> {
        self.wait_ex(thread, objs, kind, timeout, false)
    }

    /// Starts a wait that [`Dispatcher::alert`] may end early when `alertable`.
    /// The caller checks for pending user APCs before calling (see `apc`).
    pub fn wait_ex(&mut self, thread: ThreadId, objs: &[ObjId], kind: WaitKind, timeout: Option<u64>, alertable: bool) -> Result<WaitStart, Error> {
        if objs.is_empty() || objs.iter().enumerate().any(|(i, o)| objs[..i].contains(o)) {
            return Err(Error::BadWaitList);
        }
        for &o in objs {
            self.get(o)?;
        }
        if self.waiters.iter().any(|w| w.thread == thread) {
            return Err(Error::AlreadyWaiting);
        }
        if let Some(st) = self.try_satisfy(thread, objs, kind) {
            return Ok(WaitStart::Done(st));
        }
        if timeout == Some(0) {
            return Ok(WaitStart::Done(WaitStatus::Timeout));
        }
        self.waiters.push(Waiter { thread, objs: objs.to_vec(), kind, deadline: timeout.map(|t| self.now.saturating_add(t)), alertable });
        Ok(WaitStart::Blocked)
    }

    /// Completes waits that have become satisfiable, in FIFO order.
    fn satisfy(&mut self) {
        let mut i = 0;
        while i < self.waiters.len() {
            let (thread, kind) = (self.waiters[i].thread, self.waiters[i].kind);
            let objs = core::mem::take(&mut self.waiters[i].objs);
            match self.try_satisfy(thread, &objs, kind) {
                Some(status) => {
                    self.waiters.remove(i);
                    self.ready.push(Wakeup { thread, status });
                }
                None => {
                    self.waiters[i].objs = objs;
                    i += 1;
                }
            }
        }
    }

    /// Advances time, timing out expired waits.
    pub fn advance(&mut self, ticks: u64) {
        self.now = self.now.saturating_add(ticks);
        self.fire_timers();
        let now = self.now;
        let mut i = 0;
        while i < self.waiters.len() {
            if match self.waiters[i].deadline {
                Some(d) => d <= now,
                None => false,
            } {
                let w = self.waiters.remove(i);
                self.ready.push(Wakeup { thread: w.thread, status: WaitStatus::Timeout });
            } else {
                i += 1;
            }
        }
    }

    /// Aborts `thread`'s wait, if any. Returns whether one was pending.
    pub fn cancel_wait(&mut self, thread: ThreadId) -> bool {
        match self.waiters.iter().position(|w| w.thread == thread) {
            Some(i) => {
                self.waiters.remove(i);
                self.ready.push(Wakeup { thread, status: WaitStatus::Cancelled });
                true
            }
            None => false,
        }
    }

    /// Ends `thread`'s wait with [`WaitStatus::Alerted`] if it is alertable.
    /// Returns whether the thread was woken.
    pub fn alert(&mut self, thread: ThreadId) -> bool {
        match self.waiters.iter().position(|w| w.thread == thread && w.alertable) {
            Some(i) => {
                self.waiters.remove(i);
                self.ready.push(Wakeup { thread, status: WaitStatus::Alerted });
                true
            }
            None => false,
        }
    }

    /// Thread exit: drops its wait silently and abandons every mutex it still owns.
    pub fn thread_exit(&mut self, thread: ThreadId) {
        self.waiters.retain(|w| w.thread != thread);
        for s in &mut self.slots {
            if let Some(Kind::Mutex { owner, recursion, abandoned }) = s.obj.as_mut() {
                if *owner == Some(thread) {
                    *owner = None;
                    *recursion = 0;
                    *abandoned = true;
                }
            }
        }
        self.satisfy();
    }

    /// Takes the wakeups produced since the last call.
    pub fn drain_wakeups(&mut self) -> Vec<Wakeup> {
        core::mem::take(&mut self.ready)
    }

    /// Number of threads currently blocked.
    pub fn blocked_count(&self) -> usize {
        self.waiters.len()
    }
}
