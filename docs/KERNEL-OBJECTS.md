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
