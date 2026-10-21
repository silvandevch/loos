/* LoOS legacy BIOS stage 1 (512 bytes). Loads STAGE2_SECTORS sectors from LBA 1 to 0x7e00 via INT 13h
 * extended read and jumps there. DL (boot drive) is passed on at 0x7bfe... via the stage-2 header. */
.set STAGE2_SECTORS, 64
.code16
.section .text
.global _start
_start:
    cli
    xorw %ax, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    movw $0x7c00, %sp
    sti
    movb %dl, boot_drive
    movw $msg_load, %si
    call print
    movb $0x41, %ah                 /* extension check */
    movw $0x55aa, %bx
    movb boot_drive, %dl
    int $0x13
    jc fail
    movw $dap, %si
    movb $0x42, %ah
    movb boot_drive, %dl
    int $0x13
    jc fail
    movb boot_drive, %dl
    ljmp $0x0000, $0x7e00
fail:
    movw $msg_fail, %si
    call print
1:  hlt
    jmp 1b

print:                              /* DS:SI = NUL-terminated string, via BIOS teletype */
    lodsb
    testb %al, %al
    jz 2f
    movb $0x0e, %ah
    xorw %bx, %bx
    int $0x10
    jmp print
2:  ret

boot_drive: .byte 0
msg_load:   .asciz "LoOS: loading stage 2\r\n"
msg_fail:   .asciz "LoOS: disk read failed\r\n"
.balign 4
dap:
    .byte 0x10, 0
    .word STAGE2_SECTORS
    .word 0x7e00, 0x0000            /* offset, segment */
    .quad 1                         /* LBA */
    .org 510
    .word 0xaa55
