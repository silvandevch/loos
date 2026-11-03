/* Ring-3 bring-up: SYSCALL MSRs, per-CPU block, SSE for user code. The run loop lives in crates/kffi (lin.rs). */
#include "kernel.h"

extern char ucpu[], utrap_stack_end[];
extern void usys_entry(void);

uint64_t user_trap_stack_top(void) { return (uint64_t)(uintptr_t)utrap_stack_end; }

#define MSR_EFER 0xC0000080u
#define MSR_STAR 0xC0000081u
#define MSR_LSTAR 0xC0000082u
#define MSR_FMASK 0xC0000084u
#define MSR_GS_BASE 0xC0000101u
#define MSR_KGS_BASE 0xC0000102u

void user_init(void)
{
    uint64_t cr0, cr4, cr3;
    __asm__ volatile("movq %%cr0, %0" : "=r"(cr0));
    cr0 = (cr0 & ~4ull) | 2ull;                                   /* EM=0, MP=1: SSE/x87 usable by user code */
    __asm__ volatile("movq %0, %%cr0" : : "r"(cr0));
    __asm__ volatile("movq %%cr4, %0" : "=r"(cr4));
    cr4 |= (1ull << 9) | (1ull << 10);                            /* OSFXSR, OSXMMEXCPT (OSXSAVE stays off: user sees SSE2 only) */
    __asm__ volatile("movq %0, %%cr4" : : "r"(cr4));
    __asm__ volatile("movq %%cr3, %0" : "=r"(cr3));
    ((uint64_t *)ucpu)[3] = cr3;
    ((uint64_t *)ucpu)[6] = user_trap_stack_top();
    wrmsr(MSR_EFER, rdmsr(MSR_EFER) | 1);                         /* SCE */
    wrmsr(MSR_STAR, (0x10ull << 48) | (0x08ull << 32));           /* sysret: SS=0x18|3, CS=0x20|3; syscall: CS=0x08 */
    wrmsr(MSR_LSTAR, (uint64_t)(uintptr_t)usys_entry);
    wrmsr(MSR_FMASK, 0x4700);                                     /* clear IF, DF, TF, NT on entry */
    wrmsr(MSR_GS_BASE, (uint64_t)(uintptr_t)ucpu);                /* kernel mode GS -> per-CPU block */
    wrmsr(MSR_KGS_BASE, 0);                                       /* user GS (swapped in by swapgs) */
}
