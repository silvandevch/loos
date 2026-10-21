/* Early boot allocator: bump allocation from the largest usable RAM range below 4 GiB (always identity mapped). Memory is
 * never freed; it backs the framebuffer back buffers and app surfaces until the real kernel heap (crates/kmem) takes over. */
#include "kernel.h"

static uint64_t cur, end;

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
}

void *kalloc(uint64_t size, uint64_t align)
{
    if (!align) align = 16;
    uint64_t a = (cur + align - 1) & ~(align - 1);
    if (a + size > end || a + size < a) return 0;
    cur = a + size;
    return (void *)(uintptr_t)a;
}

uint64_t kalloc_free(void) { return end - cur; }
