# LoOS host-testable kernel core contract

`src/kernel/core.c` provides the policy-independent pieces used by the first
kernel increments and is safe to exercise in a host test before privileged
entry code exists.

- Run queues reject duplicate membership and dead threads, select the highest
  priority ready thread, and rotate the cursor for equal-priority fairness.
  A tick decrements the current quantum and requests preemption at zero.
- Syscall numbers are registered once in a bounded table. Dispatch rejects
  unregistered/out-of-range numbers and forwards exactly six 64-bit arguments.
  The final x86-64 entry/return assembly and user ABI remain architecture code
  layered above this table.
- User ranges are overflow-checked before a policy callback examines ownership,
  permissions, and mapped pages. A zero-length range is valid if the callback
  accepts its starting address.

This core owns no global scheduler state and performs no I/O or network access.
