#![allow(missing_docs)]
use loos_ksync::apc::{Apc, ApcQueues, Mode, QueueResult};
use loos_ksync::dpc::{Dpc, DpcQueue, Importance, WorkClass, WorkQueue};
use loos_ksync::{Dispatcher, EventKind, WaitKind, WaitStart, WaitStatus, Wakeup};

const A: Apc = Apc { routine: 1, args: [0; 3] };

#[test]
fn one_shot_timer_signals_waiter() {
    let mut d = Dispatcher::new();
    let t = d.create_timer(EventKind::Notification);
    assert!(!d.set_timer(t, 10, 0).unwrap());
    assert_eq!(d.wait(1, &[t], WaitKind::Any, None).unwrap(), WaitStart::Blocked);
    d.advance(9);
    assert!(d.drain_wakeups().is_empty());
    d.advance(1);
    assert_eq!(d.drain_wakeups(), vec![Wakeup { thread: 1, status: WaitStatus::Signaled(0) }]);
    assert!(d.timer_state(t).unwrap());
    assert!(!d.cancel_timer(t).unwrap());
}

#[test]
fn periodic_sync_timer_rearms_and_autoresets() {
    let mut d = Dispatcher::new();
    let t = d.create_timer(EventKind::Synchronization);
    d.set_timer(t, 5, 5).unwrap();
    d.advance(5);
    assert_eq!(d.wait(1, &[t], WaitKind::Any, Some(0)).unwrap(), WaitStart::Done(WaitStatus::Signaled(0)));
    assert!(!d.timer_state(t).unwrap());
    d.wait(1, &[t], WaitKind::Any, None).unwrap();
    d.advance(23); // fell behind: fires once, next due at 25
    assert_eq!(d.drain_wakeups().len(), 1);
    d.wait(1, &[t], WaitKind::Any, None).unwrap();
    d.advance(1);
    assert!(d.drain_wakeups().is_empty());
    d.advance(1);
    assert_eq!(d.drain_wakeups().len(), 1);
    assert!(d.cancel_timer(t).unwrap());
}

#[test]
fn zero_delay_timer_fires_immediately() {
    let mut d = Dispatcher::new();
    let t = d.create_timer(EventKind::Notification);
    d.set_timer(t, 0, 0).unwrap();
    assert!(d.timer_state(t).unwrap());
}

#[test]
fn user_apc_alerts_only_alertable_wait() {
    let mut d = Dispatcher::new();
    let mut q = ApcQueues::new();
    let e = d.create_event(EventKind::Notification, false);
    d.wait(1, &[e], WaitKind::Any, None).unwrap();
    assert_eq!(q.queue(1, Mode::User, A), QueueResult::Queued);
    assert!(!d.alert(1));
    d.cancel_wait(1);
    d.drain_wakeups();
    assert!(q.begin_alertable_wait(1), "pending user APC must not block");
    assert_eq!(q.drain_user(1).len(), 1);

    assert!(!q.begin_alertable_wait(1));
    d.wait_ex(1, &[e], WaitKind::Any, None, true).unwrap();
    assert_eq!(q.queue(1, Mode::User, A), QueueResult::Alert);
    assert!(d.alert(1));
    assert_eq!(d.drain_wakeups(), vec![Wakeup { thread: 1, status: WaitStatus::Alerted }]);
    q.end_wait(1);
    assert!(q.test_alert(1));
    assert_eq!(q.drain_user(1).len(), 1);
    assert!(!q.test_alert(1));
}

#[test]
fn kernel_apcs_respect_disable_and_rundown() {
    let mut q = ApcQueues::new();
    q.queue(2, Mode::Kernel, A);
    q.disable_kernel(2);
    assert!(q.next_kernel(2).is_none());
    q.enable_kernel(2);
    assert_eq!(q.next_kernel(2), Some(A));
    q.queue(2, Mode::Kernel, A);
    q.queue(2, Mode::User, A);
    assert_eq!(q.rundown(2).len(), 1);
    assert!(q.drain_user(2).is_empty());
    assert_eq!(q.queue(2, Mode::Kernel, A), QueueResult::Rejected);
}

#[test]
fn dpc_dedup_and_importance() {
    let mut q = DpcQueue::new();
    let d = |id| Dpc { id, routine: 0, args: [0; 2] };
    assert!(q.insert(d(1), Importance::Medium));
    assert!(!q.insert(d(1), Importance::High));
    assert!(q.insert(d(2), Importance::Low));
    assert!(q.insert(d(3), Importance::High));
    assert!(q.remove(2));
    assert_eq!(q.drain().iter().map(|x| x.id).collect::<Vec<_>>(), vec![3, 1]);
    assert!(!q.pending());
}

#[test]
fn work_queue_priority_order() {
    let mut w = WorkQueue::new();
    w.push(WorkClass::Delayed, 'a');
    w.push(WorkClass::Critical, 'b');
    w.push(WorkClass::HyperCritical, 'c');
    w.push(WorkClass::Critical, 'd');
    let order: Vec<char> = core::iter::from_fn(|| w.pop().map(|x| x.1)).collect();
    assert_eq!(order, vec!['c', 'b', 'd', 'a']);
    assert!(w.is_empty());
}
