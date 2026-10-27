use loos_kernel::{
    access, dispatch, EventInfo, EventKind, ObjectManager, ObjectType, SecurityDescriptor, Status,
    SyscallArgs, SyscallNumber,
};

fn args(a: u64, b: u64, c: u64) -> SyscallArgs {
    SyscallArgs([a, b, c, 0, 0, 0])
}

#[test]
fn handles_are_generation_safe_and_duplicate_tracks_rights() {
    let mut manager = ObjectManager::new();
    let event = manager.create_event(EventKind::Notification, false);
    let raw = event.raw();
    let duplicate = manager
        .duplicate(event, access::QUERY | access::SYNCHRONIZE, true)
        .unwrap();
    assert_eq!(
        manager.handle_rights(duplicate).unwrap(),
        (access::QUERY | access::SYNCHRONIZE, true)
    );
    manager.close(event).unwrap();
    assert_eq!(
        manager.event_info(event),
        Err(loos_kernel::Error::InvalidHandle)
    );
    assert!(manager.set_event(duplicate).is_err());
    assert_eq!(loos_kernel::Handle::from_raw(raw), Some(event));
    manager.close(duplicate).unwrap();
}

#[test]
fn processes_threads_and_cascading_termination() {
    let mut manager = ObjectManager::new();
    let (process, pid) = manager.create_process(None);
    let (thread, tid) = manager.create_thread(process).unwrap();
    assert_eq!(manager.process_info(process).unwrap().thread_count, 1);
    assert_eq!(manager.thread_info(thread).unwrap().process, pid);
    manager.terminate_process(process, 42).unwrap();
    assert_eq!(manager.process_info(process).unwrap().exit_status, 42);
    assert_eq!(
        manager.thread_info(thread).unwrap(),
        loos_kernel::ThreadInfo {
            id: tid,
            process: pid,
            terminated: true,
            exit_status: 42
        }
    );
    assert_eq!(manager.wait(process), Ok(true));
}

#[test]
fn named_event_is_shared_and_sync_event_is_consumed() {
    let mut manager = ObjectManager::new();
    let (first, created) = manager
        .create_named_event(
            "\\BaseNamedObjects\\ready",
            EventKind::Synchronization,
            true,
        )
        .unwrap();
    assert!(created);
    let (second, created) = manager
        .create_named_event("\\BaseNamedObjects\\ready", EventKind::Notification, false)
        .unwrap();
    assert!(!created);
    assert_eq!(
        manager.event_info(second).unwrap(),
        EventInfo {
            kind: EventKind::Synchronization,
            signaled: true
        }
    );
    assert_eq!(manager.wait(first), Ok(true));
    assert_eq!(manager.wait(second), Ok(false));
    manager.close(first).unwrap();
    manager.close(second).unwrap();
    assert_eq!(
        manager.open_named_event("\\BaseNamedObjects\\ready"),
        Err(loos_kernel::Error::NotFound)
    );
}

#[test]
fn named_namespace_aliases_share_objects_and_security_is_enforced() {
    let mut manager = ObjectManager::new();
    manager.set_principal(7);
    let (owner, created) = manager
        .create_named_event_with_security(
            "Global\\private",
            EventKind::Notification,
            false,
            SecurityDescriptor::new(7, access::ALL, 0),
        )
        .unwrap();
    assert!(created);
    assert_eq!(
        manager.event_info(owner).unwrap().kind,
        EventKind::Notification
    );
    manager.set_principal(9);
    assert_eq!(
        manager.open_named_event("\\BaseNamedObjects\\private"),
        Err(loos_kernel::Error::AccessDenied)
    );
    assert_eq!(
        manager.event_info(owner),
        Err(loos_kernel::Error::AccessDenied)
    );
    manager.set_principal(7);
    let (alias, created) = manager
        .create_named_event("Local\\private", EventKind::Synchronization, true)
        .unwrap();
    assert!(!created);
    assert_eq!(
        manager.event_info(alias).unwrap().kind,
        EventKind::Notification
    );
}

#[test]
fn syscall_dispatch_covers_object_lifecycle_and_rejects_bad_numbers() {
    let mut manager = ObjectManager::new();
    let created = dispatch(
        &mut manager,
        SyscallNumber::CreateEvent as u32,
        args(1, 1, 0),
    );
    assert_eq!(created.status, Status::Success);
    assert_eq!(
        dispatch(
            &mut manager,
            SyscallNumber::QueryObject as u32,
            args(created.value, 0, 0)
        )
        .value,
        ObjectType::Event as u64
    );
    assert_eq!(
        dispatch(
            &mut manager,
            SyscallNumber::WaitSingle as u32,
            args(created.value, 0, 0)
        )
        .value,
        1
    );
    assert_eq!(
        dispatch(
            &mut manager,
            SyscallNumber::Close as u32,
            args(created.value, 0, 0)
        )
        .status,
        Status::Success
    );
    assert_eq!(
        dispatch(
            &mut manager,
            SyscallNumber::Close as u32,
            args(created.value, 0, 0)
        )
        .status,
        Status::InvalidHandle
    );
    assert_eq!(
        dispatch(&mut manager, 0xffff, args(0, 0, 0)).status,
        Status::InvalidParameter
    );
}
