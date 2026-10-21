/* Cooperative kernel context switch (System V callee-saved registers only).
 *   void loos_context_switch(uint64_t *save_sp, uint64_t new_sp)
 * Pushes rbp,rbx,r12-r15, stores rsp to *save_sp, loads new_sp, pops, returns into the new context.
 * A fresh context is laid out by Context::new(): r15,r14,r13,r12(entry),rbx(arg),rbp, ret=loos_thread_start. */
.text
.global loos_context_switch
loos_context_switch:
    pushq %rbp
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    movq %rsp, (%rdi)
    movq %rsi, %rsp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    popq %rbp
    ret

.global loos_thread_start
loos_thread_start:
    movq %rbx, %rdi
    callq *%r12
    ud2                               /* thread entry points never return */
