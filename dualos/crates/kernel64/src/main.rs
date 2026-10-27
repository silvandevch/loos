//! kernel64: Mode A native x86-64 kernel. GRUB hands over in 32-bit protected mode;
//! a 32-bit stub checks long mode, builds identity page tables (first 1 GiB), and jumps to 64-bit code.
#![no_std]
#![no_main]

use core::arch::global_asm;
use core::panic::PanicInfo;
use kcommon::{cpu, halt, log, qemu_exit, serial};

global_asm!(
    r#"
    .section .multiboot, "a"
    .align 8
mb2_start:
    .long 0xE85250D6
    .long 0
    .long mb2_end - mb2_start
    .long -(0xE85250D6 + 0 + (mb2_end - mb2_start))
    .short 0, 0
    .long 8
mb2_end:

    .section .boot, "ax"
    .code32
    .global _start
_start:
    cli
    mov esp, offset stack_top
    mov edi, ebx                 # multiboot info (kept for later)
    cmp eax, 0x36d76289
    jne fail_magic
    # long mode present? CPUID.80000001h:EDX[29]
    mov eax, 0x80000000
    cpuid
    cmp eax, 0x80000001
    jb fail_nolm
    mov eax, 0x80000001
    cpuid
    test edx, (1 << 29)
    jz fail_nolm
    # page tables: PML4[0] -> PDPT[0] -> PD (512 x 2MiB, identity 0..1GiB)
    mov eax, offset pdpt
    or eax, 3
    mov [pml4], eax
    mov eax, offset pd
    or eax, 3
    mov [pdpt], eax
    xor ecx, ecx
1:  mov eax, ecx
    shl eax, 21
    or eax, 0x83                 # present|write|huge
    mov [pd + ecx*8], eax
    inc ecx
    cmp ecx, 512
    jb 1b
    mov eax, offset pml4
    mov cr3, eax
    mov eax, cr4
    or eax, 0x20                 # PAE
    mov cr4, eax
    mov ecx, 0xC0000080          # EFER
    rdmsr
    or eax, 0x100                # LME
    wrmsr
    mov eax, cr0
    or eax, 0x80000000           # PG
    mov cr0, eax
    lgdt [gdt_ptr]
    ljmp 0x08, offset long_entry

fail_magic:
    mov esi, offset msg_magic
    jmp fail
fail_nolm:
    mov esi, offset msg_nolm
fail:                            # tiny serial writer (COM1 assumed initialised by firmware/GRUB)
    mov dx, 0x3F8
2:  lodsb
    test al, al
    jz 3f
    out dx, al
    jmp 2b
3:  mov dx, 0xF4
    mov al, 0x11
    out dx, al
4:  hlt
    jmp 4b

    .code64
long_entry:
    xor eax, eax
    mov ax, 0x10
    mov ds, ax
    mov es, ax
    mov ss, ax
    mov fs, ax
    mov gs, ax
    mov rsp, offset stack_top
    mov edi, edi                 # zero-extend multiboot info pointer
    call kmain
5:  hlt
    jmp 5b

    .section .rodata
msg_magic: .asciz "dualos kernel64: bad multiboot2 magic\n"
msg_nolm:  .asciz "dualos kernel64: CPU lacks long mode; refusing to run\n"
    .align 8
gdt:
    .quad 0
    .quad 0x00AF9A000000FFFF     # 0x08 64-bit code
    .quad 0x00CF92000000FFFF     # 0x10 data
gdt_ptr:
    .word gdt_ptr - gdt - 1
    .long gdt

    .section .bss
    .align 4096
pml4: .skip 4096
pdpt: .skip 4096
pd:   .skip 4096
    .align 16
stack_bottom: .skip 16384
stack_top:
"#
);

#[no_mangle]
extern "C" fn kmain(_mbi: u32) -> ! {
    serial::init();
    let f = cpu::probe();
    cpu::banner("kernel64", "A", "x86_64", &f);
    if !f.long_mode {
        log!("dualos kernel64: FATAL running without long mode");
    }
    qemu_exit(0x10)
}

#[panic_handler]
fn panic(i: &PanicInfo) -> ! {
    log!("dualos kernel64 PANIC: {}", i);
    halt()
}
