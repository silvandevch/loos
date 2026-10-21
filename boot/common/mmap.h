/* Physical memory map normalisation: merge raw firmware entries and loader reservations into one sorted,
 * non-overlapping map. Where entries overlap, the most restrictive type wins (usable < boot < acpi-reclaim <
 * kernel/module < reserved < nvs < bad); holes stay holes. */
#ifndef LOOS_LOADER_MMAP_H
#define LOOS_LOADER_MMAP_H
#include "base.h"
#include "../../src/kernel/boot/bootinfo.h"

#define MM_CAP 640

struct mm {
    struct loos_mem_entry e[MM_CAP];
    uint32_t n;
    bool overflow;
};

void mm_init(struct mm *m);
/* Appends a range (zero length ignored). Usable ranges are shrunk to whole pages. */
void mm_add(struct mm *m, uint64_t base, uint64_t length, uint32_t type);
/* Sweeps appended entries into the final map; returns false on overflow. */
bool mm_normalize(struct mm *m);
/* True if [base, base+length) is entirely inside entries satisfying type == want. */
bool mm_covered_by(const struct mm *m, uint64_t base, uint64_t length, uint32_t want);
/* True if the whole range lies in RAM (any type that is backed by memory). */
bool mm_is_ram(const struct mm *m, uint64_t base, uint64_t length);
#endif
