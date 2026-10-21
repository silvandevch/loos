/* x86-64 4-level page tables built in a caller-supplied arena. Physical == virtual for the loader. */
#ifndef LOOS_LOADER_PAGING_H
#define LOOS_LOADER_PAGING_H
#include "base.h"

#define PT_P   (1ull << 0)
#define PT_W   (1ull << 1)
#define PT_PWT (1ull << 3)
#define PT_PCD (1ull << 4)
#define PT_PS  (1ull << 7)
#define PT_NX  (1ull << 63)
#define PT_UC  (PT_PWT | PT_PCD)
#define PT_PAT2M (1ull << 12)          /* PAT selector bit of a 2 MiB entry: with PWT=PCD=0 it picks PAT entry 4 (write-combining, see loader.c) */
#define PT_ADDR_MASK 0x000ffffffffff000ull

struct pt {
    uint8_t *arena;               /* 4 KiB aligned */
    uint64_t used, cap;
    uint64_t *pml4;
};

bool pt_init(struct pt *p, uint8_t *arena, uint64_t cap);
bool pt_map4k(struct pt *p, uint64_t va, uint64_t pa, uint64_t flags);
bool pt_map2m(struct pt *p, uint64_t va, uint64_t pa, uint64_t flags);
/* Returns the final entry covering va (4K PTE or 2M PDE), or 0 if unmapped. */
uint64_t pt_lookup(const struct pt *p, uint64_t va, uint64_t *page_size);
#endif
