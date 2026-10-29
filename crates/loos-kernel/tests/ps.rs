#![allow(missing_docs)]
use loos_kernel::ps::*;

fn setup() -> Ps {
    let mut ps = Ps::new(4);
    ps.process_created(1, None, false).unwrap();
    ps.thread_created(10, 1, false).unwrap();
    ps
}

#[test]
fn priority_mapping() {
    assert_eq!(thread_base_priority(PriorityClass::Normal, 2), 10);
    assert_eq!(thread_base_priority(PriorityClass::High, THREAD_PRIORITY_TIME_CRITICAL), 15);
    assert_eq!(thread_base_priority(PriorityClass::Realtime, THREAD_PRIORITY_IDLE), 16);
    assert_eq!(thread_base_priority(PriorityClass::Idle, -2), 2);
    let mut ps = setup();
    assert_eq!(ps.set_thread_priority(10, 5), Err(PsError::InvalidParameter));
    ps.set_priority_class(1, PriorityClass::AboveNormal).unwrap();
    ps.set_thread_priority(10, -1).unwrap();
    assert_eq!(ps.base_priority(10), Ok(9));
}

#[test]
fn affinity_rules() {
    let mut ps = setup();
    assert_eq!(ps.system_affinity(), 0b1111);
    assert_eq!(ps.set_thread_affinity(10, 0b0010), Ok(0b1111));
    assert_eq!(ps.set_process_affinity(1, 0b10000), Err(PsError::InvalidParameter));
    ps.set_process_affinity(1, 0b1100).unwrap();
    assert_eq!(ps.thread_affinity(10), Ok(0b1100), "disjoint thread mask resets to process mask");
    assert_eq!(ps.set_thread_affinity(10, 0b0001), Err(PsError::InvalidParameter));
}

#[test]
fn suspend_counts() {
    let mut ps = setup();
    ps.thread_created(11, 1, true).unwrap();
    assert_eq!(ps.runnable(11), Ok(false));
    assert_eq!(ps.suspend(11), Ok(1));
    assert_eq!(ps.resume(11), Ok(2));
    assert_eq!(ps.resume(11), Ok(1));
    assert_eq!(ps.resume(11), Ok(0));
    assert_eq!(ps.runnable(11), Ok(true));
    for _ in 0..MAX_SUSPEND {
        ps.suspend(10).unwrap();
    }
    assert_eq!(ps.suspend(10), Err(PsError::SuspendLimit));
}

#[test]
fn jobs_limits_inheritance_and_kill() {
    let mut ps = setup();
    let lim = JobLimits { active_processes: Some(2), affinity: Some(0b0011), priority_class: Some(PriorityClass::BelowNormal), kill_on_close: true, breakaway_ok: true };
    let j = ps.create_job(lim).unwrap();
    ps.assign(j, 1).unwrap();
    assert_eq!(ps.assign(j, 1), Err(PsError::AlreadyInJob));
    assert_eq!(ps.priority_class(1), Ok(PriorityClass::BelowNormal));
    assert_eq!(ps.thread_affinity(10), Ok(0b0011));
    ps.set_priority_class(1, PriorityClass::High).unwrap();
    assert_eq!(ps.priority_class(1), Ok(PriorityClass::BelowNormal), "job class wins");

    ps.process_created(2, Some(1), false).unwrap();
    assert_eq!(ps.job_of(2), Ok(Some(j)));
    assert_eq!(ps.process_created(3, Some(1), false), Err(PsError::LimitExceeded));
    ps.process_created(4, Some(1), true).unwrap();
    assert_eq!(ps.job_of(4), Ok(None), "breakaway allowed");

    assert_eq!(ps.terminate_job(j).unwrap(), vec![1, 2]);
    ps.process_exited(2);
    let info = ps.job_info(j).unwrap();
    assert_eq!((info.active, info.total_processes, info.terminated_processes), (vec![1], 2, 1));
    assert_eq!(ps.close_job(j).unwrap(), vec![1]);
}

#[test]
fn fibers() {
    let mut ps = setup();
    ps.thread_created(11, 1, false).unwrap();
    assert_eq!(ps.switch_to_fiber(10, 99), Err(PsError::FiberState));
    let main = ps.convert_thread_to_fiber(10, 7).unwrap();
    assert_eq!(ps.convert_thread_to_fiber(10, 7), Err(PsError::FiberState));
    let f = ps.create_fiber(1, 0x10000, 0x1000, 3).unwrap();
    assert_eq!(ps.switch_to_fiber(10, f), Ok(main));
    assert_eq!(ps.fiber_info(f).unwrap().running_on, Some(10));
    ps.convert_thread_to_fiber(11, 0).unwrap();
    assert_eq!(ps.switch_to_fiber(11, f), Err(PsError::FiberBusy));
    assert_eq!(ps.switch_to_fiber(10, main), Ok(f));
    assert_eq!(ps.delete_fiber(f), Ok(None));
    assert_eq!(ps.delete_fiber(main), Ok(Some(10)), "deleting own fiber exits thread");
    ps.convert_fiber_to_thread(11).unwrap();
    assert_eq!(ps.current_fiber(11), Ok(None));
    ps.process_created(2, None, false).unwrap();
    let other = ps.create_fiber(2, 0, 0x1000, 0).unwrap();
    ps.convert_thread_to_fiber(11, 0).unwrap();
    assert_eq!(ps.switch_to_fiber(11, other), Err(PsError::InvalidParameter));
}

#[test]
fn closed_job_freed_after_last_member_exits() {
    let mut ps = setup();
    let j = ps.create_job(JobLimits::default()).unwrap();
    ps.assign(j, 1).unwrap();
    assert_eq!(ps.close_job(j).unwrap(), Vec::<u64>::new());
    assert!(ps.job_exists(j));
    assert_eq!(ps.process_created(2, Some(1), false), Err(PsError::NotFound), "closed job takes no members");
    ps.process_exited(1);
    assert!(!ps.job_exists(j));
}
