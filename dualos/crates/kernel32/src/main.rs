//! kernel32: Mode B kernel for CPUs without long mode. i586 baseline, no FPU/SIMD.
#![no_std]
#![no_main]

use core::arch::global_asm;
use core::panic::PanicInfo;
use kcommon::{cpu, halt, log, qemu_exit, serial};

// Multiboot2 header (arch 0 = i386) + entry. GRUB enters in 32-bit protected mode, EAX=magic, EBX=info.
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

    .section .text
    .global _start
_start:
    cli
    mov esp, offset stack_top
    xor ebp, ebp
    push ebx
    push eax
    call kmain
1:  hlt
    jmp 1b

    .section .bss
    .align 16
stack_bottom: .skip 16384
stack_top:
"#
);

#[no_mangle]
extern "C" fn kmain(magic: u32, _mbi: u32) -> ! {
    serial::init();
    if magic != 0x36d76289 {
        log!("dualos kernel32: bad multiboot2 magic {:#x}", magic);
        qemu_exit(0x11);
    }
    let f = cpu::probe();
    cpu::banner("kernel32", "B", "i586", &f);
    // Baseline: CPUID + CMPXCHG8B (Pentium). Anything older is refused, not run.
    if !f.cpuid || (f.edx1 >> 8) & 1 == 0 {
        log!("dualos kernel32: CPU below Pentium baseline (CPUID+CX8 required); halting");
        qemu_exit(0x11);
    }
    if f.long_mode {
        log!("dualos kernel32: WARNING long mode available; this image was selected anyway");
    }
    qemu_exit(0x10)
}

#[panic_handler]
fn panic(i: &PanicInfo) -> ! {
    log!("dualos kernel32 PANIC: {}", i);
    halt()
}
