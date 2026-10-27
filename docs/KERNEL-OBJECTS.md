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
