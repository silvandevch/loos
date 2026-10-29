#![allow(missing_docs)]
use loos_kernel::{dispatch, ObjectManager, Status, SyscallArgs, SyscallNumber as N};

fn call(m: &mut ObjectManager, n: N, a: &[u64]) -> (Status, u64) {
    let mut args = [0; 6];
    args[..a.len()].copy_from_slice(a);
    let r = dispatch(m, n as u32, SyscallArgs(args));
    (r.status, r.value)
}

fn ok(m: &mut ObjectManager, n: N, a: &[u64]) -> u64 {
    let (s, v) = call(m, n, a);
    assert_eq!(s, Status::Success, "{n:?}");
    v
}

#[test]
fn jobs_via_syscalls_and_kill_on_close() {
    let mut m = ObjectManager::with_cpus(4);
    let p = ok(&mut m, N::CreateProcess, &[0]);
    let t = ok(&mut m, N::CreateThread, &[p, 1]);
    assert_eq!(ok(&mut m, N::QuerySched, &[t, 1]), 0, "created suspended");
    assert_eq!(ok(&mut m, N::ResumeThread, &[t]), 1);
    let job = ok(&mut m, N::CreateJob, &[1, 1, 0b11, 0]);
    ok(&mut m, N::AssignJob, &[job, p]);
    assert_eq!(ok(&mut m, N::QuerySched, &[t, 2]), 0b11);
    assert_eq!(call(&mut m, N::CreateProcess, &[p]).0, Status::LimitExceeded);
    ok(&mut m, N::SetPriorityClass, &[p, 5]);
    ok(&mut m, N::SetThreadPriority, &[t, 2]);
    assert_eq!(ok(&mut m, N::QuerySched, &[t, 0]), 15);
    ok(&mut m, N::Close, &[job]);
    assert_eq!(call(&mut m, N::SuspendThread, &[t]).0, Status::AlreadyTerminated, "kill-on-close");
}

#[test]
fn fibers_via_syscalls() {
    let mut m = ObjectManager::new();
    let p = ok(&mut m, N::CreateProcess, &[0]);
    let t = ok(&mut m, N::CreateThread, &[p, 0]);
    let main = ok(&mut m, N::ConvertThreadToFiber, &[t, 0]);
    let f = ok(&mut m, N::CreateFiber, &[p, 0x10000, 0x1000, 0]);
    assert_eq!(ok(&mut m, N::SwitchToFiber, &[t, f]), main);
    assert_eq!(ok(&mut m, N::QuerySched, &[t, 3]), f);
    assert_eq!(ok(&mut m, N::DeleteFiber, &[f]), m.thread_info(loos_kernel::Handle::from_raw(t).unwrap()).unwrap().id);
    assert_eq!(call(&mut m, N::ResumeThread, &[t]).0, Status::AlreadyTerminated);
}
