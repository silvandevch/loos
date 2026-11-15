/* objc_msgSend and friends. They keep every argument register intact around the lookup (the
 * method gets the caller's arguments unchanged) and tail-jump to the implementation. */

#if defined(__x86_64__)
/* Save rdi rsi rdx rcx r8 r9 rax(al: vector count for varargs) xmm0-7; rdi/rsi/rdx are set up
 * for the lookup call by each entry point. */
#define SAVE \
    "push %rbp\n mov %rsp, %rbp\n sub $0xc0, %rsp\n" \
    "movdqu %xmm0, 0x00(%rsp)\n movdqu %xmm1, 0x10(%rsp)\n movdqu %xmm2, 0x20(%rsp)\n movdqu %xmm3, 0x30(%rsp)\n" \
    "movdqu %xmm4, 0x40(%rsp)\n movdqu %xmm5, 0x50(%rsp)\n movdqu %xmm6, 0x60(%rsp)\n movdqu %xmm7, 0x70(%rsp)\n" \
    "mov %rdi, 0x80(%rsp)\n mov %rsi, 0x88(%rsp)\n mov %rdx, 0x90(%rsp)\n mov %rcx, 0x98(%rsp)\n" \
    "mov %r8, 0xa0(%rsp)\n mov %r9, 0xa8(%rsp)\n mov %rax, 0xb0(%rsp)\n"
#define RESTORE_JUMP \
    "mov %rax, %r11\n" \
    "movdqu 0x00(%rsp), %xmm0\n movdqu 0x10(%rsp), %xmm1\n movdqu 0x20(%rsp), %xmm2\n movdqu 0x30(%rsp), %xmm3\n" \
    "movdqu 0x40(%rsp), %xmm4\n movdqu 0x50(%rsp), %xmm5\n movdqu 0x60(%rsp), %xmm6\n movdqu 0x70(%rsp), %xmm7\n" \
    "mov 0x80(%rsp), %rdi\n mov 0x88(%rsp), %rsi\n mov 0x90(%rsp), %rdx\n mov 0x98(%rsp), %rcx\n" \
    "mov 0xa0(%rsp), %r8\n mov 0xa8(%rsp), %r9\n mov 0xb0(%rsp), %rax\n" \
    "leave\n jmp *%r11\n"

__asm__(
    ".text\n"
    ".globl _objc_msgSend\n .globl _objc_msgSend_fpret\n .globl _objc_msgSend_fp2ret\n"
    ".p2align 4\n"
    "_objc_msgSend:\n"
    " test %rdi, %rdi\n jz 1f\n"
    SAVE
    " lea 0x80(%rsp), %rdx\n call __prism_objc_lookup\n"
    RESTORE_JUMP
    "1: xor %eax, %eax\n xor %edx, %edx\n xorps %xmm0, %xmm0\n xorps %xmm1, %xmm1\n ret\n"

    /* long double results (x87): nil returns 0.0 on the FPU stack. */
    "_objc_msgSend_fpret:\n"
    " test %rdi, %rdi\n jz 2f\n"
    SAVE
    " lea 0x80(%rsp), %rdx\n call __prism_objc_lookup\n"
    RESTORE_JUMP
    "2: fldz\n ret\n"
    "_objc_msgSend_fp2ret:\n"
    " test %rdi, %rdi\n jz 3f\n"
    SAVE
    " lea 0x80(%rsp), %rdx\n call __prism_objc_lookup\n"
    RESTORE_JUMP
    "3: fldz\n fldz\n ret\n"

    /* Struct results in memory: rdi = result buffer, rsi = self, rdx = selector. */
    ".globl _objc_msgSend_stret\n"
    "_objc_msgSend_stret:\n"
    " test %rsi, %rsi\n jz 4f\n"
    SAVE
    " mov %rsi, %rdi\n mov %rdx, %rsi\n lea 0x88(%rsp), %rdx\n call __prism_objc_lookup\n"
    RESTORE_JUMP
    "4: mov %rdi, %rax\n ret\n"

    /* Super sends: rdi (rsi for stret) = struct objc_super *; the method gets its receiver. */
    ".globl _objc_msgSendSuper\n"
    "_objc_msgSendSuper:\n"
    SAVE
    " call __prism_objc_lookup_super\n"
    " mov 0x80(%rsp), %rdi\n mov (%rdi), %rdi\n mov %rdi, 0x80(%rsp)\n"
    RESTORE_JUMP
    ".globl _objc_msgSendSuper2\n"
    "_objc_msgSendSuper2:\n"
    SAVE
    " call __prism_objc_lookup_super2\n"
    " mov 0x80(%rsp), %rdi\n mov (%rdi), %rdi\n mov %rdi, 0x80(%rsp)\n"
    RESTORE_JUMP
    ".globl _objc_msgSendSuper_stret\n"
    "_objc_msgSendSuper_stret:\n"
    SAVE
    " mov %rsi, %rdi\n mov %rdx, %rsi\n call __prism_objc_lookup_super\n"
    " mov 0x88(%rsp), %rsi\n mov (%rsi), %rsi\n mov %rsi, 0x88(%rsp)\n"
    RESTORE_JUMP
    ".globl _objc_msgSendSuper2_stret\n"
    "_objc_msgSendSuper2_stret:\n"
    SAVE
    " mov %rsi, %rdi\n mov %rdx, %rsi\n call __prism_objc_lookup_super2\n"
    " mov 0x88(%rsp), %rsi\n mov (%rsi), %rsi\n mov %rsi, 0x88(%rsp)\n"
    RESTORE_JUMP
);

#elif defined(__aarch64__)
/* Save x0-x8 (x8: indirect result) and q0-q7 around the lookup. */
#define SAVE \
    "stp x29, x30, [sp, #-16]!\n mov x29, sp\n sub sp, sp, #208\n" \
    "stp x0, x1, [sp]\n stp x2, x3, [sp, #16]\n stp x4, x5, [sp, #32]\n stp x6, x7, [sp, #48]\n str x8, [sp, #64]\n" \
    "stp q0, q1, [sp, #80]\n stp q2, q3, [sp, #112]\n stp q4, q5, [sp, #144]\n stp q6, q7, [sp, #176]\n"
#define RESTORE_JUMP \
    "mov x17, x0\n" \
    "ldp x0, x1, [sp]\n ldp x2, x3, [sp, #16]\n ldp x4, x5, [sp, #32]\n ldp x6, x7, [sp, #48]\n ldr x8, [sp, #64]\n" \
    "ldp q0, q1, [sp, #80]\n ldp q2, q3, [sp, #112]\n ldp q4, q5, [sp, #144]\n ldp q6, q7, [sp, #176]\n" \
    "mov sp, x29\n ldp x29, x30, [sp], #16\n br x17\n"

__asm__(
    ".text\n"
    ".globl _objc_msgSend\n"
    ".p2align 2\n"
    "_objc_msgSend:\n"
    " cbz x0, 1f\n"
    SAVE
    " mov x2, sp\n bl __prism_objc_lookup\n"
    RESTORE_JUMP
    "1: mov x0, #0\n mov x1, #0\n fmov d0, xzr\n fmov d1, xzr\n fmov d2, xzr\n fmov d3, xzr\n ret\n"
    ".globl _objc_msgSendSuper\n"
    "_objc_msgSendSuper:\n"
    SAVE
    " bl __prism_objc_lookup_super\n"
    " ldr x9, [sp]\n ldr x9, [x9]\n str x9, [sp]\n"
    RESTORE_JUMP
    ".globl _objc_msgSendSuper2\n"
    "_objc_msgSendSuper2:\n"
    SAVE
    " bl __prism_objc_lookup_super2\n"
    " ldr x9, [sp]\n ldr x9, [x9]\n str x9, [sp]\n"
    RESTORE_JUMP
);
#endif
