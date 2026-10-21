/* LoOS BIOS stage 2: the firmware-dependent half of the BIOS boot path. Everything that needs INT 10h/13h/15h runs
 * here in real mode; the rest is C (frontend.c + the shared loader core in boot/common).
 *
 *   real mode : BIOS memory map (E820), VBE 32-bpp linear framebuffer, boot table, every boot file read from disk into
 *               high RAM through a 32 KiB bounce buffer and copied above 1 MiB in "unreal" mode (4 GiB segment limits)
 *   long mode : temporary identity map of 4 GiB, then call bios_main() (C) which builds the loader environment.
 *
 * Disk: LBA 0 stage 1 | 1..64 stage 2 | 65..66 boot table | 67.. boot files (kernel ELF, modules), sector aligned.
 * Low memory: 0x500 VBE info | 0x600 bootdata | 0x1000 E820 (count in bootdata, 24-byte entries) | 0x2000 boot table
 *             0x7e00 this image (+bss, up to 0x27e00) | 0x30000 bounce | 0x40000..0x6f000 stack | 0x70000 page tables.
 * Boot files land at 16 MiB, each page aligned. */
.set VBEINFO,  0x500
.set VBECTRL,  0x2400                 /* 512-byte VBE controller info block */
.set BOOTDATA, 0x600
.set E820,     0x1000
.set BTAB,     0x2000
.set BOUNCE_SEG, 0x3000                 /* linear 0x30000 */
.set BOUNCE,   0x30000
.set STACK_TOP, 0x6f000
.set PML4,     0x70000
.set PDPT,     0x71000
.set PD,       0x72000                  /* four page directories */
.set FILE_BASE, 0x1000000
.set MAX_E820, 128
.set BTAB_SECT_LBA, 65
.set BTAB_ENTRIES, 248                  /* offset of the file entries inside the table */
.set ENTRY_SIZE, 40
.code16
.section .text
.global stage2
stage2:
    cli
    xorw %ax, %ax
    movw %ax, %ds
    movw %ax, %es
    movb %dl, drive
    movw $0x7000, %sp

    /* ---- E820 ---- */
    xorl %ebx, %ebx
    movw $E820, %di
    xorw %bp, %bp
1:  movl $1, 20(%di)                    /* ACPI 3.0 extended attribute pre-set: "entry valid" */
    movl $0xe820, %eax
    movl $24, %ecx
    movl $0x534d4150, %edx
    int $0x15
    jc 2f
    cmpl $0x534d4150, %eax
    jne 2f
    cmpl $20, %ecx                      /* the BIOS may return 20 byte entries and leave the attribute untouched */
    jae 3f
    jmp 2f
3:  incw %bp
    addw $24, %di
    testl %ebx, %ebx
    jz 2f
    cmpw $MAX_E820, %bp
    jb 1b
2:  movzwl %bp, %eax
    movl %eax, BOOTDATA                 /* bootdata.e820_count */
    testl %eax, %eax
    jz no_memmap

    /* ---- boot table ---- */
    movw $BTAB, %bx
    movl $BTAB_SECT_LBA, %eax
    movw $2, %cx
    xorw %dx, %dx
    call read_sectors
    jc disk_fail
    cmpl $0x534f4f4c, BTAB              /* "LOOS" */
    jne bad_table
    cmpl $0x42415442, (BTAB + 4)        /* "BTAB" */
    jne bad_table
    movl (BTAB + 8), %eax
    movl %eax, (BOOTDATA + 8)           /* bootdata.nfiles */
    testl %eax, %eax
    jz bad_table
    cmpl $9, %eax
    ja bad_table

    /* ---- VBE: pick the 32-bpp linear-framebuffer mode whose pixel count is closest to the requested size ---- */
    movl $0, (VBEINFO + 0x28)
    movl (BTAB + 12), %eax              /* requested size: width | height << 16 (0 = 1920x1080) */
    testl %eax, %eax
    jnz 20f
    movl $(1920 | (1080 << 16)), %eax
20: movzwl %ax, %ecx
    shrl $16, %eax
    mull %ecx                           /* edx:eax = want_w * want_h */
    movl %eax, want_area
    movl $0x32454256, VBECTRL           /* "VBE2": ask for the VBE 2.0 info block */
    movw $0x4f00, %ax
    movw $VBECTRL, %di
    int $0x10
    cmpw $0x004f, %ax
    jne vbe_done
    movw (VBECTRL + 0x0e), %si          /* far pointer to the mode list */
    movw (VBECTRL + 0x10), %ax
    movw %ax, %fs
    movl $0xffffffff, best_diff
    movw $0, best_mode
21: movw %fs:(%si), %cx
    addw $2, %si
    cmpw $0xffff, %cx
    je 25f
    pushw %si
    pushw %cx
    movw $0x4f01, %ax
    movw $VBEINFO, %di
    int $0x10
    popw %cx
    popw %si
    cmpw $0x004f, %ax
    jne 21b
    testw $0x80, (VBEINFO)              /* linear framebuffer available */
    jz 21b
    cmpb $32, (VBEINFO + 0x19)
    jne 21b
    cmpb $6, (VBEINFO + 0x1b)           /* direct colour */
    jne 21b
    movzwl (VBEINFO + 0x12), %eax
    movzwl (VBEINFO + 0x14), %edx
    mull %edx                           /* area */
    subl want_area, %eax
    jae 22f
    negl %eax
22: cmpl best_diff, %eax
    jae 21b
    movl %eax, best_diff
    movw %cx, best_mode
    jmp 21b
25: movw best_mode, %bx
    testw %bx, %bx
    jz vbe_done
    movw %bx, %cx
    movw $0x4f01, %ax
    movw $VBEINFO, %di
    int $0x10
    orw $0x4000, %bx
    movw $0x4f02, %ax
    int $0x10
    cmpw $0x004f, %ax
    je vbe_done
    movl $0, (VBEINFO + 0x28)           /* mode switch failed: no framebuffer */
vbe_done:

    /* ---- enough RAM at FILE_BASE for all files? ---- */
    movl $FILE_BASE, %edx               /* running end address */
    movl (BOOTDATA + 8), %ecx
    movw $(BTAB + BTAB_ENTRIES), %si
8:  movl 24(%si), %eax                  /* sectors */
    shll $9, %eax
    addl $0xfff, %eax
    andl $0xfffff000, %eax
    addl %eax, %edx
    addw $ENTRY_SIZE, %si
    loop 8b
    addl $0x10000, %edx                 /* slack for the loader's scratch arena start */
    movw $E820, %si
    movzwl BOOTDATA, %ecx
9:  cmpl $1, 16(%si)
    jne 10f
    cmpl $0, 4(%si)
    jne 10f
    movl (%si), %eax
    cmpl $FILE_BASE, %eax
    ja 10f
    cmpl $0, 12(%si)
    jne ram_ok                          /* region reaches past 4 GiB */
    addl 8(%si), %eax
    jc ram_ok
    cmpl %edx, %eax
    jae ram_ok
10: addw $24, %si
    loop 9b
    jmp no_ram
ram_ok:

    /* ---- read every file into high memory ---- */
    movl $FILE_BASE, cursor
    movw $(BTAB + BTAB_ENTRIES), entry_ptr
    movl $0, file_idx
next_file:
    movl file_idx, %eax
    cmpl (BOOTDATA + 8), %eax
    jae files_done
    movw entry_ptr, %bp
    movl 24(%bp), %eax
    movl %eax, remaining
    movl 16(%bp), %eax
    movl %eax, cur_lba
    movl cursor, %eax
    movl %eax, 32(%bp)                  /* entry.dest (64 bit) */
    movl $0, 36(%bp)
    movl %eax, cur_dest
chunk:
    movl remaining, %eax
    testl %eax, %eax
    jz file_done
    cmpl $64, %eax
    jbe 11f
    movl $64, %eax
11: movl %eax, chunk_n
    movl cur_lba, %eax
    movw chunk_n, %cx
    movw $BOUNCE_SEG, %dx
    xorw %bx, %bx
    call read_sectors
    jc disk_fail
    call copy_high
    movl chunk_n, %eax
    addl %eax, cur_lba
    subl %eax, remaining
    shll $9, %eax
    addl %eax, cur_dest
    jmp chunk
file_done:
    movl cur_dest, %eax
    addl $0xfff, %eax
    andl $0xfffff000, %eax
    movl %eax, cursor
    addw $ENTRY_SIZE, entry_ptr
    incl file_idx
    jmp next_file
files_done:
    movl cursor, %eax
    movl %eax, (BOOTDATA + 12)          /* bootdata.files_end */
    movzbl drive, %eax
    movl %eax, (BOOTDATA + 4)

    /* ---- A20, flat GDT, protected mode ---- */
    inb $0x92, %al
    orb $2, %al
    andb $0xfe, %al
    outb %al, $0x92
    lgdtl gdtr
    movl %cr0, %eax
    orl $1, %eax
    movl %eax, %cr0
    ljmpl $0x08, $prot32

/* ---- real-mode helpers ---- */
/* EAX = LBA, CX = sector count, DX:BX = segment:offset. CF set on error. Clobbers SI, AH, DL. */
read_sectors:
    movl %eax, dap_lba
    movw %cx, dap_count
    movw %bx, dap_off
    movw %dx, dap_seg
    movw $dap, %si
    movb $0x42, %ah
    movb drive, %dl
    int $0x13
    ret

/* Copies chunk_n sectors from the bounce buffer to the linear address cur_dest (anywhere below 4 GiB) by entering
 * protected mode just long enough to load 4 GiB segment limits into DS/ES ("unreal mode"). */
copy_high:
    cli
    lgdtl gdtr
    movl %cr0, %eax
    orb $1, %al
    movl %eax, %cr0
    jmp 1f
1:  movw $0x10, %bx
    movw %bx, %ds
    movw %bx, %es
    andb $0xfe, %al
    movl %eax, %cr0
    jmp 2f
2:  movl $BOUNCE, %esi
    movl cur_dest, %edi
    movl chunk_n, %ecx
    shll $7, %ecx                       /* sectors * 512 / 4 dwords */
    cld
    .byte 0x67
    rep movsl
    xorw %ax, %ax                       /* back to normal 64 KiB limits */
    movw %ax, %ds
    movw %ax, %es
    ret

no_memmap:
    movw $msg_memmap, %si
    jmp fatal
no_ram:
    movw $msg_ram, %si
    jmp fatal
bad_table:
    movw $msg_table, %si
    jmp fatal
disk_fail:
    movw $msg_disk, %si
fatal:
    lodsb
    testb %al, %al
    jz 12f
    movb $0x0e, %ah
    xorw %bx, %bx
    int $0x10
    jmp fatal
12: hlt
    jmp 12b

.balign 4
drive:     .long 0
cursor:    .long 0
want_area: .long 0
best_diff: .long 0
best_mode: .word 0
file_idx:  .long 0
remaining: .long 0
cur_lba:   .long 0
cur_dest:  .long 0
chunk_n:   .long 0
entry_ptr: .word 0
.balign 4
dap:
    .byte 0x10, 0
dap_count: .word 0
dap_off:   .word 0
dap_seg:   .word 0
dap_lba:   .quad 0
msg_memmap: .asciz "LoOS: BIOS returned no memory map\r\n"
msg_ram:    .asciz "LoOS: not enough RAM for the boot files (need ~16 MiB + files)\r\n"
msg_table:  .asciz "LoOS: invalid boot table\r\n"
msg_disk:   .asciz "LoOS: disk read failed\r\n"

.code32
prot32:
    movw $0x10, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    movl $STACK_TOP, %esp
    /* identity map 4 GiB with 2 MiB pages (writable, present) */
    movl $PML4, %edi
    xorl %eax, %eax
    movl $(6*4096/4), %ecx
    rep stosl
    movl $(PDPT | 3), PML4
    movl $PDPT, %edi
    movl $(PD | 3), %eax
    movl $4, %ecx
13: movl %eax, (%edi)
    addl $4096, %eax
    addl $8, %edi
    loop 13b
    movl $PD, %edi
    movl $0x83, %eax
    movl $2048, %ecx
14: movl %eax, (%edi)
    addl $0x200000, %eax
    addl $8, %edi
    loop 14b
    movl $PML4, %eax
    movl %eax, %cr3
    movl %cr4, %eax
    orl $0x20, %eax                     /* PAE */
    movl %eax, %cr4
    movl $0xc0000080, %ecx
    rdmsr
    orl $0x900, %eax                    /* LME | NXE */
    wrmsr
    movl %cr0, %eax
    orl $0x80000000, %eax
    movl %eax, %cr0
    ljmpl $0x18, $long64

.code64
long64:
    xorl %eax, %eax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    movq $STACK_TOP, %rsp
    xorl %ebp, %ebp
    cld
    /* COM1 115200 8N1 */
    movw $0x3f9, %dx; xorb %al, %al; outb %al, %dx
    movw $0x3fb, %dx; movb $0x80, %al; outb %al, %dx
    movw $0x3f8, %dx; movb $1, %al; outb %al, %dx
    movw $0x3f9, %dx; xorb %al, %al; outb %al, %dx
    movw $0x3fb, %dx; movb $3, %al; outb %al, %dx
    movw $0x3fa, %dx; movb $0xc7, %al; outb %al, %dx
    callq bios_main
15: hlt
    jmp 15b

.balign 8
gdt:
    .quad 0
    .quad 0x00cf9a000000ffff            /* 0x08 32-bit code */
    .quad 0x00cf92000000ffff            /* 0x10 flat data (also the unreal-mode descriptor) */
    .quad 0x00af9a000000ffff            /* 0x18 64-bit code */
gdtr:
    .word 4*8 - 1
    .long gdt
