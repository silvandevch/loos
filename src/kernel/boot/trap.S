/* Exception stubs 0..31: build a struct ktrap_frame and call ktrap(). */
.macro STUB n
.global ktrap_stub\n
ktrap_stub\n:
.if (\n == 8) || (\n >= 10 && \n <= 14) || (\n == 17) || (\n == 21) || (\n == 29) || (\n == 30)
.else
    pushq $0                            /* no CPU error code */
.endif
    pushq $\n
    jmp ktrap_common
.endm

.text
.irp n, 0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31
STUB \n
.endr

ktrap_common:
    testb $3, 24(%rsp)                  /* saved CS: exception raised in ring 3 */
    jnz utrap_entry
    pushq %r15; pushq %r14; pushq %r13; pushq %r12; pushq %r11; pushq %r10; pushq %r9; pushq %r8
    pushq %rdi; pushq %rsi; pushq %rbp; pushq %rsp; pushq %rbx; pushq %rdx; pushq %rcx; pushq %rax
    movq %rsp, %rdi
    cld
    andq $-16, %rsp
    callq ktrap                          /* returns only for #BP */
    movq %rdi, %rsp
    popq %rax; popq %rcx; popq %rdx; popq %rbx; addq $8, %rsp; popq %rbp; popq %rsi; popq %rdi
    popq %r8; popq %r9; popq %r10; popq %r11; popq %r12; popq %r13; popq %r14; popq %r15
    addq $16, %rsp
    iretq

.section .data.rel.ro, "aw"
.global ktrap_stubs
.balign 8
ktrap_stubs:
.irp n, 0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31
    .quad ktrap_stub\n
.endr
