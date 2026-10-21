/* Application-processor startup trampoline. Copied to physical 0x8000 (SIPI vector 0x08) and run in real
 * mode: real -> protected -> long mode, then jumps to the 64-bit entry stored in the data block.
 * The BSP fills the data block at ap_data (cr3 as a 32-bit physical address, 64-bit stack, 64-bit entry). */
.set TRAMP, 0x8000

.section .rodata.ap_tramp, "a"
.global ap_tramp_start
.global ap_tramp_end
.global ap_data_offset
.code16
ap_tramp_start:
    cli
    cld
    movw %cs, %ax
    movw %ax, %ds
    lgdtl (ap_gdtr - ap_tramp_start)
    movl %cr0, %eax
    orl $1, %eax
    movl %eax, %cr0
    ljmpl $0x08, $(TRAMP + (ap_prot - ap_tramp_start))

.code32
ap_prot:
    movw $0x10, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    movl %cr4, %eax
    orl $0x20, %eax                       /* PAE */
    movl %eax, %cr4
    movl (TRAMP + (ap_cr3 - ap_tramp_start)), %eax
    movl %eax, %cr3
    movl $0xc0000080, %ecx                /* EFER */
    rdmsr
    orl $0x900, %eax                      /* LME | NXE */
    wrmsr
    movl %cr0, %eax
    orl $0x80000001, %eax                 /* PG | PE */
    movl %eax, %cr0
    ljmpl $0x18, $(TRAMP + (ap_long - ap_tramp_start))

.code64
ap_long:
    xorl %eax, %eax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    movq (TRAMP + (ap_stack - ap_tramp_start)), %rsp
    movq (TRAMP + (ap_entry - ap_tramp_start)), %rax
    callq *%rax
1:  hlt
    jmp 1b

.balign 8
ap_gdt:
    .quad 0
    .quad 0x00cf9a000000ffff              /* 0x08 32-bit code */
    .quad 0x00cf92000000ffff              /* 0x10 32-bit data */
    .quad 0x00af9a000000ffff              /* 0x18 64-bit code */
ap_gdtr:
    .word 4*8 - 1
    .long TRAMP + (ap_gdt - ap_tramp_start)
.balign 8
.global ap_data_start
ap_data_start:
ap_cr3:   .quad 0
ap_stack: .quad 0
ap_entry: .quad 0
ap_tramp_end:
.set ap_data_offset, ap_data_start - ap_tramp_start
