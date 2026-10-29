# LoOS kernel objects and first syscalls

`crates/loos-kernel` is the architecture-independent executive layer for M2.
It is `no_std` + `alloc`, has no I/O or network path, and is exercised on the
host before the x86-64 entry stubs wire it into a running kernel.

## Handles

`Handle::raw()` encodes a slot index in the low 32 bits (`index + 1`) and a
generation in the high 32 bits. Zero is never a valid handle. Closing a handle
clears the slot and increments its generation; a stale raw value therefore
cannot access a newly allocated object.

Each handle carries an access mask and an inherit flag. Duplication can retain
the source mask (`rights = 0`) or request a subset. Operations validate both
the handle generation and the requested right before touching the object.

Named objects are normalized into `\\BaseNamedObjects`; `Local\\name`,
`Global\\name`, and the fully qualified form address the same namespace. Every
object has a compact security descriptor. Principal zero is the system/public
identity; a non-zero owner descriptor is private to its owner until group/world
ACL entries are added. Denied rights always win, and replacing a descriptor
requires `SET_SECURITY`.

## Objects

The initial object types are:

| Type | State | Operations |
|---|---|---|
| Process | id, parent, threads, termination status | create/open/query/wait/terminate |
| Thread | id, process, termination status | create/open/query/wait/terminate |
| Event | notification or synchronization, signaled state | create/named-open/set/reset/wait |

Named events use the manager's local namespace. Creating an existing name opens
the existing object and leaves its original kind/state unchanged. Process
termination marks all attached threads terminated with the same exit status.
Named objects are removed from the namespace when their last handle closes.

## Stable syscall layer

`dispatch` accepts a syscall number and six 64-bit arguments, returning a
`Status` plus one 64-bit value. The initial LoOS numbers are:

| Number | Operation |
|---:|---|
| 0 | Close |
| 1 | Duplicate |
| 2–4 | CreateEvent, SetEvent, ResetEvent |
| 5 | WaitSingle (immediate check; the scheduler parks/retries blocked callers) |
| 6–9 | CreateProcess, CreateThread, TerminateProcess, TerminateThread |
| 10 | QueryObject (type or process/thread id) |

These numbers are LoOS ABI numbers and are independent of the provisional
compatibility numbers in `src/win32/ntdll/syscalls.tbl`. Architecture entry
code must validate user pointers/register state before calling this dispatcher.

## Deferred execution and timers (`crates/ksync`)

- **Waitable timers** — `Dispatcher::create_timer(kind)`, `set_timer(id, delay, period)`,
  `cancel_timer`, `timer_state`. `Notification` timers stay signaled; `Synchronization`
  timers auto-reset on a satisfied wait. Periodic timers that fall behind fire once and
  re-arm at the next period boundary. Timers fire inside `advance()`.
- **Alertable waits** — `wait_ex(.., alertable)`; `alert(thread)` ends an alertable wait
  with `WaitStatus::Alerted`.
- **APCs** (`ksync::apc::ApcQueues`) — per-thread kernel/user FIFO queues. Kernel APCs are
  gated by `disable_kernel`/`enable_kernel` nesting. User APCs only run at alertable points:
  call `begin_alertable_wait` before an alertable wait (true = don't block), and on
  `QueueResult::Alert` call `Dispatcher::alert`. `rundown` rejects new APCs, drops user
  APCs and returns owed kernel APCs.
- **DPCs** (`ksync::dpc::DpcQueue`) — per-CPU; a DPC id is queued at most once; `High`
  importance goes to the head. `WorkQueue<T>` runs HyperCritical > Critical > Delayed,
  FIFO within a class.

## Ps layer (`crates/loos-kernel/src/ps.rs`)

`ObjectManager` owns lifetime and handles; `ps::Ps` owns scheduler-facing state. The
caller reports `process_created`/`thread_created`/`*_exited`. Operations that must end
processes (`terminate_job`, `close_job` with kill-on-close) return pids for the caller
to terminate through the object manager.

- **Priority**: NT classes (Idle 4 … Realtime 24) + relative −2..2, idle/time-critical
  saturation; dynamic range 1–15, realtime 16–31 (`base_priority`).
- **Affinity**: process mask ⊆ system ∩ job mask; thread mask ⊆ process mask. Narrowing
  the process mask resets disjoint thread masks to it.
- **Suspend**: counts up to 127; `CREATE_SUSPENDED` starts at 1; runnable at 0.
- **Jobs**: one job per process; children inherit it unless breakaway is requested and
  allowed. Limits: active-process count, affinity, priority class (wins over
  `SetPriorityClass`), kill-on-close. Accounting: total/terminated counts.
- **Integration**: `ObjectManager` owns a `Ps` (`with_cpus(n)`); create/terminate keep both in step.
  Jobs are Ob objects (`ObjectType::Job`); closing the last handle applies kill-on-close; a closed
  job is freed when its last member exits. Scheduler hook: `loos_scheduler_sync` (C, `core.c`) pulls
  base priority + suspend state per thread through `loos_kobj_sched_query`.
- **Syscalls** 11–24: CreateJob, AssignJob, TerminateJob, SetPriorityClass, SetThreadPriority,
  SetAffinity, Suspend/ResumeThread, ConvertThreadToFiber, CreateFiber, SwitchToFiber, DeleteFiber,
  ConvertFiberToThread, QuerySched (args in `SyscallNumber` docs). CreateProcess arg1 bit0 =
  breakaway; CreateThread arg1 bit0 = suspended. New statuses: LimitExceeded −7, InvalidState −8.
- **Fibers**: convert/create/switch/delete/convert-back. A fiber runs on ≤1 thread and only
  within its process; deleting the running fiber returns the thread that must exit.

## Structured exception handling (`crates/loos-kernel/src/seh.rs`)

Implemented from the public PE32+ x64 exception-handling spec (no copied code).

- `Image::load` reads `.pdata`; `lookup` = `RtlLookupFunctionEntry`.
- `virtual_unwind` = `RtlVirtualUnwind` for UNWIND_INFO v1/v2: PUSH_NONVOL, ALLOC_SMALL/LARGE,
  SET_FPREG, SAVE_NONVOL(_FAR), SAVE_XMM128(_FAR) (no integer effect), PUSH_MACHFRAME,
  chained info. Codes past the current prolog offset are skipped; the handler is reported
  only past the prolog. Epilog detection is not implemented.
- `dispatch` order: vectored exception handlers (first-registered-first unless `first`),
  frame handlers walking the stack (leaf frames: return address at RSP), continue handlers
  after any `ContinueExecution`, then `unhandled_filter`. Out-of-stack or misaligned RSP,
  or bad unwind data, gives `Outcome::BadStack`. Handlers run via a caller closure.
- Not yet: unwind phase (`RtlUnwindEx`, termination handlers/`__finally`), wiring into the
  #PF/#GP trap path and user-mode `KiUserExceptionDispatcher`.

## Bugcheck and safe mode (boot kernel)

- `trap.S` + `bugcheck.c`: IDT for vectors 0–31 (no IST yet: a kernel stack overflow still triple-faults).
  #BP logs and resumes; #PF → `0x50 PAGE_FAULT_IN_NONPAGED_AREA` (cr2, write, rip, err); #DF/#MC → `0x7F`;
  others → `0x1E KMODE_EXCEPTION_NOT_HANDLED` (NTSTATUS, rip, err).
- Stop path: serial report, frame-pointer stack walk (kernel built with `-fno-omit-frame-pointer`),
  kdump-format dump into a 16 KiB in-memory buffer (`kbugcheck_dump_data/len`; not written to disk yet),
  stop screen on the framebuffer, QEMU exit 39. Nested stops halt immediately.
- `safemode` on the kernel command line: one CPU, no splash, 100% UI scale.
- `loos.bugcheck=pf|ud|gp|manual` crashes on purpose; `boot/bios/bugcheck_test.sh` checks all four.
