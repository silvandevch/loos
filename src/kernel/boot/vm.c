#include "kernel.h"
#include <string.h>

/* Walks the live tables (identity mapped at this stage); returns the leaf entry or 0. */
uint64_t vm_pte(uint64_t cr3, uint64_t va, uint64_t *page_size)
{
    uint64_t t = cr3 & 0x000ffffffffff000ull;
    for (int level = 4; level >= 1; level--) {
        uint64_t e = ((const uint64_t *)(uintptr_t)t)[(va >> (12 + 9 * (level - 1))) & 511];
        if (!(e & 1)) return 0;
        if (level == 1 || (e & 0x80)) { if (page_size) *page_size = 1ull << (12 + 9 * (level - 1)); return e; }
        t = e & 0x000ffffffffff000ull;
    }
    return 0;
}

uint64_t vm_virt_to_phys(const struct loos_bootinfo *bi, const void *p)
{
    return bi->kernel_phys + ((uint64_t)(uintptr_t)p - bi->kernel_virt);
}

/* ---- device memory ---- */
#define PTE_P 1ull
#define PTE_W 2ull
#define PTE_PWT 8ull
#define PTE_PCD 16ull
#define PTE_PS 0x80ull
#define PTE_NX (1ull << 63)
#define PTE_ADDR 0x000ffffffffff000ull

static uint64_t *new_table(void)
{
    uint64_t *t = kalloc(4096, 4096);
    if (t) memset(t, 0, 4096);
    return t;
}

/* Maps [phys, phys+len) uncached, identity (virtual == physical) with 2 MiB pages, in the live page tables. The loader only maps
 * the low 4 GiB (and RAM): 64-bit PCI BARs of NVMe/AHCI controllers can live above that. */
int vm_map_mmio(uint64_t phys, uint64_t len)
{
    uint64_t cr3;
    __asm__ volatile("movq %%cr3, %0" : "=r"(cr3));
    uint64_t *pml4 = (uint64_t *)(uintptr_t)(cr3 & PTE_ADDR);
    for (uint64_t a = phys & ~0x1fffffull; a < phys + len; a += 0x200000) {
        if (a >= (1ull << 39)) return -1;                                   /* beyond the first PML4 slot */
        if (!(pml4[0] & PTE_P)) return -1;
        uint64_t *pdpt = (uint64_t *)(uintptr_t)(pml4[0] & PTE_ADDR);
        uint64_t *e3 = &pdpt[(a >> 30) & 511];
        if (!(*e3 & PTE_P)) {
            uint64_t *pd = new_table();
            if (!pd) return -1;
            *e3 = (uint64_t)(uintptr_t)pd | PTE_P | PTE_W;
        } else if (*e3 & PTE_PS) {
            continue;                                                       /* a 1 GiB page already covers it */
        }
        uint64_t *pd = (uint64_t *)(uintptr_t)(*e3 & PTE_ADDR);
        uint64_t *e2 = &pd[(a >> 21) & 511];
        if (*e2 & PTE_P) continue;                                          /* already mapped (RAM or earlier mapping) */
        *e2 = a | PTE_P | PTE_W | PTE_PS | PTE_PWT | PTE_PCD | PTE_NX;
        __asm__ volatile("invlpg (%0)" : : "r"((uintptr_t)a) : "memory");
    }
    return 0;
}
