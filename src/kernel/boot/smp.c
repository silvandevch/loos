/* Application-processor bring-up: INIT-SIPI-SIPI with the real-mode trampoline from crates/khal/src/ap_tramp.S.
 *
 * The trampoline's long-mode tail runs from low memory, which the loader's identity map marks NX. Like Linux's
 * trampoline_pgd, the BSP therefore builds a tiny private PML4 (low 2 MiB executable + the kernel's high half)
 * that the APs use until they switch to the real kernel tables. */
#include "kernel.h"

extern const char ap_tramp_start[], ap_data_start[], ap_tramp_end[];
#define TRAMP_PHYS 0x8000u

static volatile uint32_t cpus_online = 1;
static uint8_t ap_stacks[ACPI_MAX_CPUS][4096] __attribute__((aligned(16)));
static uint64_t tramp_pml4[512] __attribute__((aligned(4096)));
static uint64_t tramp_pdpt[512] __attribute__((aligned(4096)));
static uint64_t tramp_pd[512] __attribute__((aligned(4096)));
static uint64_t kernel_cr3;

static volatile uint32_t *lapic;
static void lapic_write(uint32_t reg, uint32_t v) { lapic[reg / 4] = v; }
static uint32_t lapic_read(uint32_t reg) { return lapic[reg / 4]; }

static void send_ipi(uint32_t apic_id, uint32_t lo)
{
    lapic_write(0x310, apic_id << 24);
    lapic_write(0x300, lo);
    for (int i = 0; i < 1000 && (lapic_read(0x300) & 0x1000); i++) udelay(10);   /* delivery status */
}

/* First C code on an application processor (called from the trampoline on the AP's own stack). */
void ap_entry(void)
{
    __asm__ volatile("movq %0, %%cr3" : : "r"(kernel_cr3) : "memory");
    kgdt_load();
    __atomic_fetch_add(&cpus_online, 1, __ATOMIC_SEQ_CST);
    for (;;) __asm__ volatile("cli; hlt");
}

uint32_t smp_start_aps(const struct loos_bootinfo *bi, const struct acpi_cpus *c)
{
    if (!c->ok || c->count < 2) return 1;
    /* the trampoline page must be ordinary RAM that nothing owns */
    const struct loos_mem_entry *m = (const struct loos_mem_entry *)(uintptr_t)bi->mmap_phys;
    int free_low = 0;
    for (uint32_t i = 0; i < bi->mmap_count; i++)
        if ((m[i].type == LOOS_MEM_USABLE || m[i].type == LOOS_MEM_BOOT) && m[i].base <= TRAMP_PHYS && TRAMP_PHYS + 0x1000 <= m[i].base + m[i].length) free_low = 1;
    if (!free_low) { serial_puts("SMP: trampoline page not available\n"); return 1; }

    __asm__ volatile("movq %%cr3, %0" : "=r"(kernel_cr3));
    uint64_t apic_base = rdmsr(0x1b);
    lapic = (volatile uint32_t *)(uintptr_t)(apic_base & 0x000ffffffffff000ull);
    if (!(apic_base & (1 << 11)) || (apic_base & (1 << 10))) { serial_puts("SMP: local APIC not in xAPIC mode\n"); return 1; }
    uint32_t bsp = lapic_read(0x20) >> 24;
    time_init();

    /* private tables: identity-map the first 2 MiB executable, share the kernel's high-half PML4 slot */
    const uint64_t *kp = (const uint64_t *)(uintptr_t)(kernel_cr3 & 0x000ffffffffff000ull);
    tramp_pml4[0] = vm_virt_to_phys(bi, tramp_pdpt) | 3;
    tramp_pml4[511] = kp[511];
    tramp_pdpt[0] = vm_virt_to_phys(bi, tramp_pd) | 3;
    tramp_pd[0] = 0x83;                                        /* 2 MiB, present | writable, executable */

    size_t len = (size_t)(ap_tramp_end - ap_tramp_start);
    uint8_t *dst = (uint8_t *)(uintptr_t)TRAMP_PHYS;
    for (size_t i = 0; i < len; i++) dst[i] = (uint8_t)ap_tramp_start[i];
    uint64_t *data = (uint64_t *)(dst + (ap_data_start - ap_tramp_start));

    uint32_t started = 0;
    for (uint32_t i = 0; i < c->count; i++) {
        if (c->apic_id[i] == bsp) continue;
        data[0] = vm_virt_to_phys(bi, tramp_pml4);
        data[1] = (uint64_t)(uintptr_t)&ap_stacks[started][sizeof ap_stacks[0]];
        data[2] = (uint64_t)(uintptr_t)&ap_entry;
        uint32_t before = cpus_online;
        send_ipi(c->apic_id[i], 0xc500);                       /* INIT, assert, level */
        udelay(10000);
        send_ipi(c->apic_id[i], 0x8500);                       /* INIT de-assert */
        for (int s = 0; s < 2 && cpus_online == before; s++) {
            send_ipi(c->apic_id[i], 0x4600 | (TRAMP_PHYS >> 12));
            for (int w = 0; w < 100 && cpus_online == before; w++) udelay(1000);
        }
        if (cpus_online == before) { serial_puts("SMP: AP "); serial_dec(c->apic_id[i]); serial_puts(" did not start\n"); continue; }
        started++;
    }
    return cpus_online;
}
