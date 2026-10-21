/* SYSCALL entry (IA32_LSTAR). Convention: rax = number, args rdi rsi rdx r10 r8 r9, result in rax.
 * Per-CPU block addressed through GS (swapgs): gs:0 = kernel stack top, gs:8 = scratch for the user rsp.
 * Builds a TrapFrame (vector 0x100) and calls loos_syscall_dispatch(frame). IF is cleared by IA32_FMASK. */
.text
.extern loos_syscall_dispatch
.global loos_syscall_entry
loos_syscall_entry:
    swapgs
    movq %rsp, %gs:8
    movq %gs:0, %rsp
    pushq $0x1b                       /* ss  (USER_DS) */
    pushq %gs:8                       /* user rsp */
    pushq %r11                        /* rflags */
    pushq $0x23                       /* cs  (USER_CS) */
    pushq %rcx                        /* rip */
    pushq $0                          /* error */
    pushq $0x100                      /* vector: syscall */
    pushq %rax; pushq %rbx; pushq %rcx; pushq %rdx; pushq %rsi; pushq %rdi; pushq %rbp
    pushq %r8; pushq %r9; pushq %r10; pushq %r11; pushq %r12; pushq %r13; pushq %r14; pushq %r15
    cld
    movq %rsp, %rdi
    callq loos_syscall_dispatch
    /* sysret faults (#GP in ring 0!) on a non-canonical rip, so such frames leave through iretq instead */
    movq 8*17(%rsp), %rcx             /* frame.rip */
    movq %rcx, %rax
    shlq $16, %rax
    sarq $16, %rax
    cmpq %rax, %rcx
    jne .Lslow
    popq %r15; popq %r14; popq %r13; popq %r12; popq %r11; popq %r10; popq %r9; popq %r8
    popq %rbp; popq %rdi; popq %rsi; popq %rdx; popq %rcx; popq %rbx; popq %rax
    addq $16, %rsp                    /* vector, error */
    popq %rcx                         /* rip */
    addq $8, %rsp                     /* cs */
    popq %r11                         /* rflags */
    popq %rsp                         /* user rsp (ss is ignored by sysret) */
    swapgs
    sysretq
.Lslow:
    popq %r15; popq %r14; popq %r13; popq %r12; popq %r11; popq %r10; popq %r9; popq %r8
    popq %rbp; popq %rdi; popq %rsi; popq %rdx; popq %rcx; popq %rbx; popq %rax
    addq $16, %rsp
    swapgs
    iretq
