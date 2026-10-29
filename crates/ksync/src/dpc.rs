//! Deferred procedure calls and system work queues (state only).
//!
//! A [`DpcQueue`] is per-CPU and drained at `DISPATCH_LEVEL` when the
//! interrupt that queued work returns. A DPC object may sit in the queue at
//! most once: re-queuing an inserted DPC is a no-op, as in NT. High-importance
//! DPCs go to the head. [`WorkQueue`] is the passive-level counterpart: items
//! are run by worker threads, highest priority class first, FIFO within one.

use alloc::collections::VecDeque;
use alloc::vec::Vec;

/// DPC queue position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Importance {
    /// Appended to the tail; delivery may be batched.
    Low,
    /// Appended to the tail.
    Medium,
    /// Inserted at the head.
    High,
}

/// Identifies a DPC object (owner-allocated, e.g. per device).
pub type DpcId = u64;

/// A queued DPC with its call arguments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dpc {
    /// The DPC object.
    pub id: DpcId,
    /// Routine entry point.
    pub routine: u64,
    /// `SystemArgument1/2`.
    pub args: [u64; 2],
}

/// One processor's DPC queue.
#[derive(Default)]
pub struct DpcQueue {
    q: VecDeque<Dpc>,
}

impl DpcQueue {
    /// Creates an empty queue.
    pub fn new() -> Self {
        Self::default()
    }

    /// `KeInsertQueueDpc`: returns `false` if `dpc.id` is already queued.
    pub fn insert(&mut self, dpc: Dpc, imp: Importance) -> bool {
        if self.q.iter().any(|d| d.id == dpc.id) {
            return false;
        }
        match imp {
            Importance::High => self.q.push_front(dpc),
            Importance::Low | Importance::Medium => self.q.push_back(dpc),
        }
        true
    }

    /// `KeRemoveQueueDpc`: returns whether it was queued.
    pub fn remove(&mut self, id: DpcId) -> bool {
        match self.q.iter().position(|d| d.id == id) {
            Some(i) => self.q.remove(i).is_some(),
            None => false,
        }
    }

    /// Whether a software interrupt should be requested to drain the queue.
    pub fn pending(&self) -> bool {
        !self.q.is_empty()
    }

    /// Takes every queued DPC in run order. DPCs queued by routines while the
    /// batch runs land in the queue for the next drain.
    pub fn drain(&mut self) -> Vec<Dpc> {
        self.q.drain(..).collect()
    }
}

/// Work queue priority class (NT `WORK_QUEUE_TYPE`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum WorkClass {
    /// Long-running, may be starved.
    Delayed = 0,
    /// Time-sensitive.
    Critical = 1,
    /// Must not be delayed by anything.
    HyperCritical = 2,
}

/// Passive-level work items of type `T`.
pub struct WorkQueue<T> {
    q: [VecDeque<T>; 3],
}

impl<T> Default for WorkQueue<T> {
    fn default() -> Self {
        Self { q: [VecDeque::new(), VecDeque::new(), VecDeque::new()] }
    }
}

impl<T> WorkQueue<T> {
    /// Creates an empty work queue.
    pub fn new() -> Self {
        Self::default()
    }

    /// `ExQueueWorkItem`.
    pub fn push(&mut self, class: WorkClass, item: T) {
        self.q[class as usize].push_back(item);
    }

    /// Next item for a worker thread, highest class first.
    pub fn pop(&mut self) -> Option<(WorkClass, T)> {
        [WorkClass::HyperCritical, WorkClass::Critical, WorkClass::Delayed]
            .into_iter()
            .find_map(|c| self.q[c as usize].pop_front().map(|t| (c, t)))
    }

    /// Items waiting in all classes.
    pub fn len(&self) -> usize {
        self.q.iter().map(VecDeque::len).sum()
    }

    /// Whether no items are waiting.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}
