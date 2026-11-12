/* Early boot allocator: bump allocation from the largest usable RAM range below 4 GiB (always identity mapped), and
 * kalloc_high() from RAM above 4 GiB for memory without a 32-bit address limit (the Rust heap). Memory is
 * never freed; it backs the framebuffer back buffers and app surfaces until the real kernel heap (crates/kmem) takes over. */
#include "kernel.h"

static uint64_t cur, end;
static uint64_t hcur, hend;   /* largest usable range above 4 GiB that the live page tables identity-map */

static uint64_t read_cr3(void) { uint64_t v; __asm__ volatile("mov %%cr3, %0" : "=r"(v)); return v; }

void kalloc_init(const struct loos_bootinfo *bi)
{
    const struct loos_mem_entry *m = (const struct loos_mem_entry *)(uintptr_t)bi->mmap_phys;
    uint64_t best = 0;
    for (uint32_t i = 0; i < bi->mmap_count; i++) {
        if (m[i].type != LOOS_MEM_USABLE) continue;
        uint64_t b = m[i].base < 0x100000 ? 0x100000 : m[i].base;         /* stay out of low memory */
        uint64_t e = m[i].base + m[i].length;
        if (e > 0x100000000ull) e = 0x100000000ull;
        if (e <= b || e - b <= best) continue;
        best = e - b; cur = b; end = e;
    }
    /* RAM above 4 GiB: the loader identity-maps it with 2 MiB pages as far as its table arena allowed,
     * so only the mapped prefix of a range is used. */
    uint64_t cr3 = read_cr3(), hbest = 0;
    for (uint32_t i = 0; i < bi->mmap_count; i++) {
        if (m[i].type != LOOS_MEM_USABLE) continue;
        uint64_t b = m[i].base < 0x100000000ull ? 0x100000000ull : m[i].base, e = m[i].base + m[i].length;
        if (e <= b) continue;
        uint64_t mapped = b;
        while (mapped < e) {
            uint64_t ps = 0, pte = vm_pte(cr3, mapped, &ps);
            if (!pte || !ps || (pte & 0x000ffffffffff000ull & ~(ps - 1)) != (mapped & ~(ps - 1))) break;   /* not identity */
            mapped = (mapped & ~(ps - 1)) + ps;
        }
        if (mapped > e) mapped = e;
        if (mapped > b && mapped - b > hbest) { hbest = mapped - b; hcur = b; hend = mapped; }
    }
}

void *kalloc_high(uint64_t size, uint64_t align)
{
    if (!align) align = 16;
    uint64_t a = (hcur + align - 1) & ~(align - 1);
    if (!hend || a + size > hend || a + size < a) return kalloc(size, align);   /* no high RAM left: fall back below 4 GiB */
    hcur = a + size;
    return (void *)(uintptr_t)a;
}

void *kalloc(uint64_t size, uint64_t align)
{
    if (!align) align = 16;
    uint64_t a = (cur + align - 1) & ~(align - 1);
    if (a + size > end || a + size < a) return 0;
    cur = a + size;
    return (void *)(uintptr_t)a;
}

uint64_t kalloc_free(void) { return (end - cur) + (hend - hcur); }
