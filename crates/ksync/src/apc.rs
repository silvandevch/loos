//! Asynchronous procedure call queues (NT `KAPC` semantics, state only).
//!
//! Each thread has a kernel and a user queue. Kernel APCs run whenever the
//! thread is at passive level and has not disabled them; user APCs run only
//! when the thread enters an alertable wait or calls `test_alert`. The
//! scheduler owns delivery: this module decides *what* may run and tells the
//! caller when an alertable waiter must be woken via
//! [`crate::Dispatcher::alert`].

use alloc::collections::{BTreeMap, VecDeque};

use crate::dispatcher::ThreadId;

/// Processor mode an APC targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Runs in kernel mode on the target thread.
    Kernel,
    /// Runs in user mode at the next alertable point.
    User,
}

/// A queued call: routine address plus up to three arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Apc {
    /// Routine entry point.
    pub routine: u64,
    /// Arguments passed to the routine.
    pub args: [u64; 3],
}

/// What the caller must do after [`ApcQueues::queue`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QueueResult {
    /// Queued; nothing else needed.
    Queued,
    /// Queued; the thread is in an alertable wait and must be alerted.
    Alert,
    /// The thread is terminating and no longer accepts APCs.
    Rejected,
}

#[derive(Default)]
struct ThreadApcs {
    kernel: VecDeque<Apc>,
    user: VecDeque<Apc>,
    kernel_disable: u32,
    alertable_wait: bool,
    user_pending_alert: bool,
    terminating: bool,
}

/// Per-thread APC state for every thread.
#[derive(Default)]
pub struct ApcQueues {
    threads: BTreeMap<ThreadId, ThreadApcs>,
}

impl ApcQueues {
    /// Creates empty queues.
    pub fn new() -> Self {
        Self::default()
    }

    /// Queues `apc` to `thread` in `mode`.
    pub fn queue(&mut self, thread: ThreadId, mode: Mode, apc: Apc) -> QueueResult {
        let t = self.threads.entry(thread).or_default();
        if t.terminating {
            return QueueResult::Rejected;
        }
        match mode {
            Mode::Kernel => {
                t.kernel.push_back(apc);
                QueueResult::Queued
            }
            Mode::User => {
                t.user.push_back(apc);
                t.user_pending_alert = true;
                if t.alertable_wait {
                    t.alertable_wait = false;
                    QueueResult::Alert
                } else {
                    QueueResult::Queued
                }
            }
        }
    }

    /// `KeEnterCriticalRegion`-style nesting: disables kernel APC delivery.
    pub fn disable_kernel(&mut self, thread: ThreadId) {
        let t = self.threads.entry(thread).or_default();
        t.kernel_disable = t.kernel_disable.saturating_add(1);
    }

    /// Undoes one [`ApcQueues::disable_kernel`].
    pub fn enable_kernel(&mut self, thread: ThreadId) {
        if let Some(t) = self.threads.get_mut(&thread) {
            t.kernel_disable = t.kernel_disable.saturating_sub(1);
        }
    }

    /// Next kernel APC that may run now, if delivery is enabled.
    pub fn next_kernel(&mut self, thread: ThreadId) -> Option<Apc> {
        let t = self.threads.get_mut(&thread)?;
        if t.kernel_disable > 0 {
            return None;
        }
        t.kernel.pop_front()
    }

    /// Called before an alertable wait. Returns `true` when user APCs are
    /// already pending, in which case the wait must not block and the caller
    /// completes it as alerted. Otherwise records the alertable wait.
    pub fn begin_alertable_wait(&mut self, thread: ThreadId) -> bool {
        let t = self.threads.entry(thread).or_default();
        if !t.user.is_empty() {
            return true;
        }
        t.alertable_wait = true;
        false
    }

    /// Called when any wait of `thread` ends.
    pub fn end_wait(&mut self, thread: ThreadId) {
        if let Some(t) = self.threads.get_mut(&thread) {
            t.alertable_wait = false;
        }
    }

    /// `NtTestAlert`: whether user APCs are pending; clears the alert flag.
    pub fn test_alert(&mut self, thread: ThreadId) -> bool {
        self.threads.get_mut(&thread).is_some_and(|t| core::mem::replace(&mut t.user_pending_alert, false) || !t.user.is_empty())
    }

    /// Takes every pending user APC in FIFO order (delivered on return to user mode).
    pub fn drain_user(&mut self, thread: ThreadId) -> VecDeque<Apc> {
        match self.threads.get_mut(&thread) {
            Some(t) => {
                t.user_pending_alert = false;
                core::mem::take(&mut t.user)
            }
            None => VecDeque::new(),
        }
    }

    /// Thread termination: rejects new APCs and discards pending user APCs.
    /// Returns kernel APCs still owed (they run during rundown).
    pub fn rundown(&mut self, thread: ThreadId) -> VecDeque<Apc> {
        let t = self.threads.entry(thread).or_default();
        t.terminating = true;
        t.user.clear();
        t.alertable_wait = false;
        core::mem::take(&mut t.kernel)
    }

    /// Forgets a dead thread entirely.
    pub fn remove(&mut self, thread: ThreadId) {
        self.threads.remove(&thread);
    }
}
