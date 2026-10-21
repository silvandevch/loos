//! Privileged instructions. Only compiled for the bare-metal target.

#[cfg(target_os = "none")]
use core::arch::asm;

/// Reads an MSR.
///
/// # Safety
/// `msr` must exist on this CPU; reading some MSRs has side effects.
#[cfg(target_os = "none")]
pub unsafe fn rdmsr(msr: u32) -> u64 {
    let (lo, hi): (u32, u32);
    // SAFETY: caller upholds the MSR contract.
    unsafe { asm!("rdmsr", in("ecx") msr, out("eax") lo, out("edx") hi, options(nomem, nostack, preserves_flags)) };
    (u64::from(hi) << 32) | u64::from(lo)
}

/// Writes an MSR.
///
/// # Safety
/// The write must be valid for `msr` and must not break kernel invariants (paging, syscall entry, ...).
#[cfg(target_os = "none")]
pub unsafe fn wrmsr(msr: u32, value: u64) {
    // SAFETY: caller upholds the MSR contract.
    unsafe { asm!("wrmsr", in("ecx") msr, in("eax") value as u32, in("edx") (value >> 32) as u32, options(nomem, nostack, preserves_flags)) };
}

/// Loads the GDT and reloads segment registers.
///
/// # Safety
/// `ptr` must describe a valid, permanently-live GDT whose selectors match [`crate::gdt::sel`].
#[cfg(target_os = "none")]
pub unsafe fn load_gdt(ptr: &crate::gdt::DescriptorPointer) {
    // SAFETY: caller guarantees the table; the far return reloads CS from the new table.
    unsafe {
        asm!(
            "lgdt [{p}]",
            "mov ax, {ds}", "mov ds, ax", "mov es, ax", "mov ss, ax",
            "push {cs}", "lea {t}, [rip + 2f]", "push {t}", "retfq", "2:",
            p = in(reg) ptr, ds = const crate::gdt::sel::KERNEL_DS, cs = const crate::gdt::sel::KERNEL_CS as u64,
            t = lateout(reg) _, out("ax") _,
        );
    }
}

/// Loads the IDT.
///
/// # Safety
/// `idt` must outlive all interrupts and contain valid gates.
#[cfg(target_os = "none")]
pub unsafe fn load_idt(idt: &crate::idt::Idt) {
    let p = crate::gdt::DescriptorPointer { limit: crate::idt::Idt::limit(), base: idt as *const _ as u64 };
    // SAFETY: caller guarantees the table.
    unsafe { asm!("lidt [{}]", in(reg) &p, options(readonly, nostack, preserves_flags)) };
}

#[cfg(target_os = "none")]
core::arch::global_asm!(include_str!("isr.S"), options(att_syntax));

#[cfg(target_os = "none")]
core::arch::global_asm!(include_str!("syscall.S"), options(att_syntax));
#[cfg(target_os = "none")]
core::arch::global_asm!(include_str!("ap_tramp.S"), options(att_syntax));

/// Programs EFER.SCE, STAR, LSTAR and FMASK so `syscall` enters `loos_syscall_entry` with IF cleared.
///
/// # Safety
/// The per-CPU GS block (kernel stack at `gs:0`) must be installed in `IA32_KERNEL_GS_BASE`, and
/// `loos_syscall_dispatch` must exist.
#[cfg(target_os = "none")]
pub unsafe fn enable_syscalls() {
    extern "C" {
        fn loos_syscall_entry();
    }
    // SAFETY: caller guarantees the environment; values follow the layout in `gdt::sel`.
    unsafe {
        wrmsr(crate::msr::EFER, rdmsr(crate::msr::EFER) | crate::msr::EFER_SCE | crate::msr::EFER_NXE);
        wrmsr(crate::msr::STAR, crate::msr::star(crate::gdt::sel::KERNEL_CS, 0x10));
        wrmsr(crate::msr::LSTAR, loos_syscall_entry as usize as u64);
        wrmsr(crate::msr::FMASK, 0x4_0700);          // clear IF, TF, DF, AC on entry
    }
}
