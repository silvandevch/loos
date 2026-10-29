//! Kernel synchronization for LoOS.
//!
//! * [`apc`] – per-thread kernel/user APC queues and alertable-wait rules.
//! * [`dpc`] – per-CPU DPC queues and passive-level work queues.
//! * [`spin`] – interrupt-agnostic ticket spinlock (`no_std`, no allocation).
//! * [`dispatcher`] – NT-style dispatcher objects (events, semaphores, mutants) and the
//!   wait machinery (`wait any` / `wait all`, timeouts, abandonment on thread death).
//!
//! The dispatcher is a pure state machine: it never blocks or touches a scheduler. Callers
//! (the scheduler) feed it `wait`/`signal`/`tick` calls and drain [`dispatcher::Wakeup`]s to
//! make threads runnable again, which keeps the whole thing deterministic and host-testable.
#![no_std]

extern crate alloc;

pub mod apc;
pub mod dispatcher;
pub mod dpc;
pub mod spin;

pub use dispatcher::{Dispatcher, Error, EventKind, ObjId, ThreadId, WaitKind, WaitStart, WaitStatus, Wakeup};
pub use spin::{SpinLock, SpinLockGuard};
