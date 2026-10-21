/* Kernel entry, reached from the boot loader in 64-bit mode with RDI = &loos_bootinfo (physical, identity mapped).
 * The image is position independent (static PIE slid by the loader), so everything here is RIP-relative. */
.section .text.start, "ax"
.global _kstart
_kstart:
    cli
    movq %rdi, %r15                     /* bootinfo survives the GDT switch */
    leaq kstack_top(%rip), %rsp
    leaq kgdt(%rip), %rax               /* own GDT: the firmware's may live in memory we reclaim */
    movq %rax, kgdt_ptr+2(%rip)
    lgdt kgdt_ptr(%rip)
    pushq $0x08
    leaq 1f(%rip), %rax
    pushq %rax
    lretq
1:  movw $0x10, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    xorl %eax, %eax
    movw %ax, %fs
    movw %ax, %gs
    movq %r15, %rdi
    xorl %ebp, %ebp
    callq kmain
2:  hlt
    jmp 2b

.global kgdt_load                       /* void kgdt_load(void): used by application processors */
kgdt_load:
    lgdt kgdt_ptr(%rip)
    pushq $0x08
    leaq 1f(%rip), %rax
    pushq %rax
    lretq
1:  movw $0x10, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    xorl %eax, %eax
    movw %ax, %fs
    movw %ax, %gs
    ret

.section .data
.balign 16
kgdt:
    .quad 0
    .quad 0x00af9a000000ffff            /* 0x08 kernel code */
    .quad 0x00cf92000000ffff            /* 0x10 kernel data */
kgdt_ptr:
    .word 3*8 - 1
    .quad 0                             /* filled with the (slid) address of kgdt at entry */

.section .bss
.balign 16
.global kstack_bottom
kstack_bottom:
    .skip 64*1024
.global kstack_top
kstack_top:
