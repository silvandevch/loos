#![allow(missing_docs)]
#![cfg(all(target_arch = "x86_64", unix))]
use loos_khal::switch::Context;
use std::cell::UnsafeCell;

struct Shared {
    main: UnsafeCell<Context>,
    a: UnsafeCell<Context>,
    b: UnsafeCell<Context>,
    log: UnsafeCell<Vec<u32>>,
}
unsafe impl Sync for Shared {}
static S: Shared = Shared { main: UnsafeCell::new(Context::empty()), a: UnsafeCell::new(Context::empty()), b: UnsafeCell::new(Context::empty()), log: UnsafeCell::new(Vec::new()) };

extern "C" fn worker(id: usize) -> ! {
    // locals must survive across switches, and the stack must be 16-byte aligned for SSE spills
    let mut acc = [0u64; 8];
    let v = [1.5f64; 4];
    for round in 0..3u64 {
        for (i, x) in acc.iter_mut().enumerate() {
            *x += round * 10 + i as u64 + id as u64;
        }
        unsafe {
            (*S.log.get()).push(id as u32 * 100 + round as u32);
            let (me, other) = if id == 1 { (&mut *S.a.get(), &*S.b.get()) } else { (&mut *S.b.get(), &*S.a.get()) };
            Context::switch(me, other);
        }
    }
    let sum: u64 = acc.iter().sum();
    assert_eq!(sum, (0..3u64).map(|r| (0..8u64).map(|i| r * 10 + i + id as u64).sum::<u64>()).sum::<u64>());
    assert_eq!(v.iter().sum::<f64>(), 6.0);
    unsafe {
        (*S.log.get()).push(id as u32 * 100 + 99);
        let (me, other) = if id == 1 { (&mut *S.a.get(), &*S.main.get()) } else { (&mut *S.b.get(), &*S.main.get()) };
        Context::switch(me, other);
    }
    unreachable!("finished thread resumed")
}

#[test]
fn two_threads_ping_pong_with_register_and_stack_state() {
    let mut sa = vec![0u8; 64 * 1024];
    let mut sb = vec![0u8; 64 * 1024];
    unsafe {
        *S.a.get() = Context::new(sa.as_mut_ptr().add(sa.len() - 7), worker, 1);   // deliberately misaligned top
        *S.b.get() = Context::new(sb.as_mut_ptr().add(sb.len()), worker, 2);
        Context::switch(&mut *S.main.get(), &*S.a.get());
        // a ran until its first switch to b; keep bouncing until a thread reports completion
        while !(*S.log.get()).contains(&199) {
            Context::switch(&mut *S.main.get(), &*S.b.get());
        }
        let log = &*S.log.get();
        assert_eq!(&log[..6], &[100, 200, 101, 201, 102, 202]);
        assert!(log.contains(&199));
    }
}
