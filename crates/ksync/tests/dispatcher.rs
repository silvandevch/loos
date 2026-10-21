#![allow(missing_docs)]
use loos_ksync::*;

fn drain(d: &mut Dispatcher) -> Vec<Wakeup> {
    d.drain_wakeups()
}

#[test]
fn notification_event_releases_all() {
    let mut d = Dispatcher::new();
    let e = d.create_event(EventKind::Notification, false);
    assert_eq!(d.wait(1, &[e], WaitKind::Any, None), Ok(WaitStart::Blocked));
    assert_eq!(d.wait(2, &[e], WaitKind::Any, None), Ok(WaitStart::Blocked));
    d.set_event(e).unwrap();
    let w = drain(&mut d);
    assert_eq!(w.len(), 2);
    assert!(d.event_state(e).unwrap());
    assert_eq!(d.wait(3, &[e], WaitKind::Any, Some(0)), Ok(WaitStart::Done(WaitStatus::Signaled(0))));
}

#[test]
fn synchronization_event_releases_one_fifo() {
    let mut d = Dispatcher::new();
    let e = d.create_event(EventKind::Synchronization, false);
    d.wait(1, &[e], WaitKind::Any, None).unwrap();
    d.wait(2, &[e], WaitKind::Any, None).unwrap();
    d.set_event(e).unwrap();
    assert_eq!(drain(&mut d), vec![Wakeup { thread: 1, status: WaitStatus::Signaled(0) }]);
    assert!(!d.event_state(e).unwrap());
    d.set_event(e).unwrap();
    assert_eq!(drain(&mut d)[0].thread, 2);
}

#[test]
fn pulse_wakes_waiters_only() {
    let mut d = Dispatcher::new();
    let e = d.create_event(EventKind::Notification, false);
    d.wait(1, &[e], WaitKind::Any, None).unwrap();
    d.pulse_event(e).unwrap();
    assert_eq!(drain(&mut d).len(), 1);
    assert_eq!(d.wait(2, &[e], WaitKind::Any, Some(0)), Ok(WaitStart::Done(WaitStatus::Timeout)));
}

#[test]
fn semaphore_counts_and_limits() {
    let mut d = Dispatcher::new();
    let s = d.create_semaphore(1, 3).unwrap();
    assert_eq!(d.wait(1, &[s], WaitKind::Any, Some(0)), Ok(WaitStart::Done(WaitStatus::Signaled(0))));
    d.wait(2, &[s], WaitKind::Any, None).unwrap();
    d.wait(3, &[s], WaitKind::Any, None).unwrap();
    assert_eq!(d.release_semaphore(s, 2), Ok(0));
    assert_eq!(drain(&mut d).len(), 2);
    assert_eq!(d.semaphore_count(s), Ok(0));
    assert_eq!(d.release_semaphore(s, 4), Err(Error::LimitExceeded));
    assert_eq!(d.release_semaphore(s, 0), Err(Error::LimitExceeded));
    assert_eq!(d.create_semaphore(5, 3), Err(Error::LimitExceeded));
}

#[test]
fn mutex_recursion_ownership_handoff() {
    let mut d = Dispatcher::new();
    let m = d.create_mutex(None);
    assert_eq!(d.wait(1, &[m], WaitKind::Any, Some(0)), Ok(WaitStart::Done(WaitStatus::Signaled(0))));
    assert_eq!(d.wait(1, &[m], WaitKind::Any, Some(0)), Ok(WaitStart::Done(WaitStatus::Signaled(0))));
    assert_eq!(d.wait(2, &[m], WaitKind::Any, None), Ok(WaitStart::Blocked));
    assert_eq!(d.release_mutex(m, 2), Err(Error::NotOwner));
    assert_eq!(d.release_mutex(m, 1), Ok(1));
    assert!(drain(&mut d).is_empty());
    assert_eq!(d.release_mutex(m, 1), Ok(0));
    assert_eq!(drain(&mut d), vec![Wakeup { thread: 2, status: WaitStatus::Signaled(0) }]);
    assert_eq!(d.mutex_owner(m), Ok(Some(2)));
}

#[test]
fn owner_death_abandons_mutex() {
    let mut d = Dispatcher::new();
    let m = d.create_mutex(Some(1));
    d.wait(2, &[m], WaitKind::Any, None).unwrap();
    d.thread_exit(1);
    assert_eq!(drain(&mut d), vec![Wakeup { thread: 2, status: WaitStatus::Abandoned(0) }]);
    d.release_mutex(m, 2).unwrap();
    assert_eq!(d.wait(3, &[m], WaitKind::Any, Some(0)), Ok(WaitStart::Done(WaitStatus::Signaled(0))));
}

#[test]
fn wait_any_picks_first_signaled_and_consumes_only_it() {
    let mut d = Dispatcher::new();
    let a = d.create_event(EventKind::Synchronization, false);
    let b = d.create_event(EventKind::Synchronization, true);
    let c = d.create_event(EventKind::Synchronization, true);
    assert_eq!(d.wait(1, &[a, b, c], WaitKind::Any, Some(0)), Ok(WaitStart::Done(WaitStatus::Signaled(1))));
    assert!(!d.event_state(b).unwrap());
    assert!(d.event_state(c).unwrap());
}

#[test]
fn wait_all_is_atomic() {
    let mut d = Dispatcher::new();
    let a = d.create_event(EventKind::Synchronization, true);
    let b = d.create_event(EventKind::Synchronization, false);
    assert_eq!(d.wait(1, &[a, b], WaitKind::All, None), Ok(WaitStart::Blocked));
    assert!(d.event_state(a).unwrap(), "nothing consumed while the wait is incomplete");
    d.set_event(b).unwrap();
    assert_eq!(drain(&mut d), vec![Wakeup { thread: 1, status: WaitStatus::Signaled(0) }]);
    assert!(!d.event_state(a).unwrap() && !d.event_state(b).unwrap());
}

#[test]
fn timeouts_and_cancel() {
    let mut d = Dispatcher::new();
    let e = d.create_event(EventKind::Notification, false);
    d.wait(1, &[e], WaitKind::Any, Some(10)).unwrap();
    d.wait(2, &[e], WaitKind::Any, Some(20)).unwrap();
    d.wait(3, &[e], WaitKind::Any, None).unwrap();
    d.advance(9);
    assert!(drain(&mut d).is_empty());
    d.advance(1);
    assert_eq!(drain(&mut d), vec![Wakeup { thread: 1, status: WaitStatus::Timeout }]);
    assert!(d.cancel_wait(3));
    assert!(!d.cancel_wait(3));
    assert_eq!(drain(&mut d), vec![Wakeup { thread: 3, status: WaitStatus::Cancelled }]);
    d.advance(100);
    assert_eq!(drain(&mut d), vec![Wakeup { thread: 2, status: WaitStatus::Timeout }]);
    assert_eq!(d.blocked_count(), 0);
}

#[test]
fn validation() {
    let mut d = Dispatcher::new();
    let e = d.create_event(EventKind::Notification, false);
    let s = d.create_semaphore(0, 1).unwrap();
    assert_eq!(d.wait(1, &[], WaitKind::Any, None), Err(Error::BadWaitList));
    assert_eq!(d.wait(1, &[e, e], WaitKind::All, None), Err(Error::BadWaitList));
    assert_eq!(d.set_event(s), Err(Error::WrongType));
    d.wait(1, &[e], WaitKind::Any, None).unwrap();
    assert_eq!(d.wait(1, &[e], WaitKind::Any, None), Err(Error::AlreadyWaiting));
    assert_eq!(d.destroy(e), Err(Error::InUse));
    d.cancel_wait(1);
    assert_eq!(d.destroy(e), Ok(()));
    assert_eq!(d.set_event(e), Err(Error::InvalidHandle), "stale handle");
    let e2 = d.create_event(EventKind::Notification, false);
    assert_ne!(e, e2, "slot reuse bumps the generation");
}

#[test]
fn spinlock_basic_and_threads() {
    use std::sync::Arc;
    let l = SpinLock::new(0u64);
    {
        let _g = l.lock();
        assert!(l.try_lock().is_none() && l.is_locked());
    }
    assert!(!l.is_locked());
    let l = Arc::new(l);
    let hs: Vec<_> = (0..4)
        .map(|_| {
            let l = l.clone();
            std::thread::spawn(move || {
                for _ in 0..20_000 {
                    *l.lock() += 1;
                }
            })
        })
        .collect();
    for h in hs {
        h.join().unwrap();
    }
    assert_eq!(*l.lock(), 80_000);
}
