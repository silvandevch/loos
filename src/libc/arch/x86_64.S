/* LoOS libc x86-64 entry point and non-local jumps. */
    .text
    .globl _start
    .type _start,@function
_start:
    xor %ebp, %ebp
    mov %rsp, %rdi            /* -> argc, argv..., NULL, envp..., NULL */
    and $-16, %rsp
    call __libc_start_main
    hlt

/* jmp_buf layout: rbx rbp r12 r13 r14 r15 rsp rip */
    .globl setjmp
    .globl _setjmp
    .globl sigsetjmp
    .type setjmp,@function
sigsetjmp:
setjmp:
_setjmp:
    mov %rbx, 0(%rdi)
    mov %rbp, 8(%rdi)
    mov %r12, 16(%rdi)
    mov %r13, 24(%rdi)
    mov %r14, 32(%rdi)
    mov %r15, 40(%rdi)
    lea 8(%rsp), %rdx
    mov %rdx, 48(%rdi)
    mov (%rsp), %rdx
    mov %rdx, 56(%rdi)
    xor %eax, %eax
    ret

    .globl longjmp
    .globl _longjmp
    .globl siglongjmp
    .type longjmp,@function
siglongjmp:
longjmp:
_longjmp:
    mov %esi, %eax
    test %eax, %eax
    jnz 1f
    inc %eax
1:  mov 0(%rdi), %rbx
    mov 8(%rdi), %rbp
    mov 16(%rdi), %r12
    mov 24(%rdi), %r13
    mov 32(%rdi), %r14
    mov 40(%rdi), %r15
    mov 48(%rdi), %rsp
    jmp *56(%rdi)

    .section .note.GNU-stack,"",@progbits
