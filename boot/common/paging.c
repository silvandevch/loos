#include "paging.h"

static uint64_t *new_table(struct pt *p)
{
    if (p->cap - p->used < PAGE_SIZE) return NULL;
    uint64_t *t = (uint64_t *)(p->arena + p->used);
    p->used += PAGE_SIZE;
    memset(t, 0, PAGE_SIZE);
    return t;
}

bool pt_init(struct pt *p, uint8_t *arena, uint64_t cap)
{
    if ((uintptr_t)arena & (PAGE_SIZE - 1)) return false;
    p->arena = arena; p->used = 0; p->cap = cap;
    p->pml4 = new_table(p);
    return p->pml4 != NULL;
}

/* Walks/creates down to the table at `level` (3 = PDPT .. 1 = PT) and returns its entry for va. */
static uint64_t *walk(struct pt *p, uint64_t va, int target_level)
{
    uint64_t *t = p->pml4;
    for (int level = 4; level > target_level; level--) {
        uint64_t *e = &t[(va >> (12 + 9 * (level - 1))) & 511];
        if (!(*e & PT_P)) {
            uint64_t *n = new_table(p);
            if (!n) return NULL;
            *e = (uint64_t)(uintptr_t)n | PT_P | PT_W;       /* permissions are enforced at the leaf */
        } else if (*e & PT_PS) {
            return NULL;                                     /* a large page is already here */
        }
        t = (uint64_t *)(uintptr_t)(*e & PT_ADDR_MASK);
    }
    return &t[(va >> (12 + 9 * (target_level - 1))) & 511];
}

bool pt_map4k(struct pt *p, uint64_t va, uint64_t pa, uint64_t flags)
{
    if ((va | pa) & (PAGE_SIZE - 1)) return false;
    uint64_t *e = walk(p, va, 1);
    if (!e || (*e & PT_P)) return false;
    *e = (pa & PT_ADDR_MASK) | (flags & ~PT_PS) | PT_P;
    return true;
}

bool pt_map2m(struct pt *p, uint64_t va, uint64_t pa, uint64_t flags)
{
    if ((va | pa) & (0x200000ull - 1)) return false;
    uint64_t *e = walk(p, va, 2);
    if (!e || (*e & PT_P)) return false;
    *e = (pa & PT_ADDR_MASK) | flags | PT_PS | PT_P;
    return true;
}

uint64_t pt_lookup(const struct pt *p, uint64_t va, uint64_t *page_size)
{
    uint64_t *t = p->pml4;
    for (int level = 4; level >= 1; level--) {
        uint64_t e = t[(va >> (12 + 9 * (level - 1))) & 511];
        if (!(e & PT_P)) return 0;
        if (level == 1) { if (page_size) *page_size = PAGE_SIZE; return e; }
        if (e & PT_PS) { if (page_size) *page_size = 1ull << (12 + 9 * (level - 1)); return e; }
        t = (uint64_t *)(uintptr_t)(e & PT_ADDR_MASK);
    }
    return 0;
}
