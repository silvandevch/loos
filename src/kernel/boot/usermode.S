/* Ring-3 support: run_user() enters user mode with a full register frame and returns to its C caller when the user
 * program executes SYSCALL or raises an exception. Kernel and user run on different page tables (the user PML4 shares only
 * the kernel image entry), so every entry path switches CR3 back before touching kernel data.
 *
 * Per-CPU block (addressed through GS in kernel mode; IA32_KERNEL_GS_BASE points at it while user code runs):
 *   0 kernel rsp left by run_user   8 user rsp scratch   16 spare   24 kernel CR3   32 struct uframe *   48 entry stack top
 *
 * struct uframe (layout shared with crates/kffi/src/lin.rs): gpr[16] in x86 encoding order (rax rcx rdx rbx rsp rbp rsi rdi
 * r8-r15), vector, error, rip, cs, rflags, rsp, ss, cr3 (24 qwords), 512-byte fxsave area at offset 192 (16-byte aligned). */
.text
.global run_user
run_user:                               /* uint64_t run_user(struct uframe *f): returns the trap vector (0x100 = syscall) */
    pushq %rbp; pushq %rbx; pushq %r12; pushq %r13; pushq %r14; pushq %r15
    movq %rsp, %gs:0
    movq %rdi, %gs:32
    movq %cr3, %rax
    movq %rax, %gs:24
    fxrstor 192(%rdi)
    pushq 8*22(%rdi)                    /* ss */
    pushq 8*21(%rdi)                    /* rsp */
    pushq 8*20(%rdi)                    /* rflags */
    pushq 8*19(%rdi)                    /* cs */
    pushq 8*18(%rdi)                    /* rip */
    movq 8*23(%rdi), %rax
    movq %rax, %cr3                     /* user address space */
    movq %rdi, %r15
    movq 0(%r15), %rax; movq 8(%r15), %rcx; movq 16(%r15), %rdx; movq 24(%r15), %rbx
    movq 40(%r15), %rbp; movq 48(%r15), %rsi; movq 56(%r15), %rdi
    movq 64(%r15), %r8; movq 72(%r15), %r9; movq 80(%r15), %r10; movq 88(%r15), %r11
    movq 96(%r15), %r12; movq 104(%r15), %r13; movq 112(%r15), %r14
    movq 120(%r15), %r15
    swapgs
    iretq

/* SYSCALL entry (IA32_LSTAR): builds the same frame the exception path produces. */
.global usys_entry
usys_entry:
    swapgs
    movq %rsp, %gs:8
    movq %gs:48, %rsp
    pushq $0x1b
    pushq %gs:8
    pushq %r11                          /* user rflags */
    pushq $0x23
    pushq %rcx                          /* user rip */
    pushq $0
    pushq $0x100
    jmp usave_gprs

/* Exception from ring 3 (from ktrap_common): the CPU already pushed ss/rsp/rflags/cs/rip on TSS.rsp0, the stub pushed
 * error code and vector. */
.global utrap_entry
utrap_entry:
    swapgs
usave_gprs:
    pushq %r15; pushq %r14; pushq %r13; pushq %r12; pushq %r11; pushq %r10; pushq %r9; pushq %r8
    pushq %rdi; pushq %rsi; pushq %rbp; pushq %rsp; pushq %rbx; pushq %rdx; pushq %rcx; pushq %rax
    cld
    movq %gs:32, %rdi
    movq %rdi, %r8
    movq %rsp, %rsi
    movl $23, %ecx
    rep movsq                           /* frame -> uframe */
    fxsave 192(%r8)
    movq %gs:24, %rax
    movq %rax, %cr3                     /* back on the kernel address space */
    movq %gs:0, %rsp
    popq %r15; popq %r14; popq %r13; popq %r12; popq %rbx; popq %rbp
    movq 8*16(%r8), %rax                /* vector */
    ret

.section .bss
.balign 16
.global ucpu
ucpu:   .skip 64
.balign 4096
utrap_stack:    .skip 8192
.global utrap_stack_end
utrap_stack_end:
