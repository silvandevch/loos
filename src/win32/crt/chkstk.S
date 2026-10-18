/* __chkstk for the Windows x64 ABI: probe the stack one page at a time so the
 * guard page mechanism can grow it. Input: rax = bytes about to be allocated.
 * Preserves rax and rcx; the caller performs the actual `sub rsp, rax`.
 * Standard probe loop; written fresh (same algorithm as every x64 toolchain). */
    .text
    .globl __chkstk
__chkstk:
    pushq %rcx
    pushq %rax
    cmpq  $0x1000, %rax
    leaq  24(%rsp), %rcx
    jb    2f
1:  subq  $0x1000, %rcx
    testq %rcx, (%rcx)
    subq  $0x1000, %rax
    cmpq  $0x1000, %rax
    ja    1b
2:  subq  %rax, %rcx
    testq %rcx, (%rcx)
    popq  %rax
    popq  %rcx
    ret
