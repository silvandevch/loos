#include "mmap.h"

void mm_init(struct mm *m) { m->n = 0; m->overflow = false; }

static int prio(uint32_t t)
{
    switch (t) {
    case LOOS_MEM_USABLE:       return 1;
    case LOOS_MEM_BOOT:         return 2;
    case LOOS_MEM_ACPI_RECLAIM: return 3;
    case LOOS_MEM_KERNEL:
    case LOOS_MEM_MODULE:       return 4;
    case LOOS_MEM_RESERVED:     return 5;
    case LOOS_MEM_ACPI_NVS:     return 6;
    default:                    return 7;                 /* BAD and anything unknown: never touch */
    }
}

void mm_add(struct mm *m, uint64_t base, uint64_t length, uint32_t type)
{
    if (length == 0) return;
    uint64_t end = base + length;
    if (end < base) end = ~0ull;                          /* clamp wrap-around */
    if (type == LOOS_MEM_USABLE) {
        base = ALIGN_UP(base, PAGE_SIZE);
        end = ALIGN_DOWN(end, PAGE_SIZE);
        if (end <= base) return;
    }
    if (m->n >= MM_CAP) { m->overflow = true; return; }
    m->e[m->n].base = base;
    m->e[m->n].length = end - base;
    m->e[m->n].type = type;
    m->e[m->n].attr = 0;
    m->n++;
}

bool mm_normalize(struct mm *m)
{
    if (m->overflow) return false;
    static uint64_t pts[2 * MM_CAP];
    uint32_t np = 0;
    for (uint32_t i = 0; i < m->n; i++) {
        pts[np++] = m->e[i].base;
        pts[np++] = m->e[i].base + m->e[i].length;
    }
    /* insertion sort: n is small and this runs once */
    for (uint32_t i = 1; i < np; i++) {
        uint64_t v = pts[i]; uint32_t j = i;
        while (j && pts[j - 1] > v) { pts[j] = pts[j - 1]; j--; }
        pts[j] = v;
    }
    static struct loos_mem_entry out[MM_CAP];
    uint32_t no = 0;
    for (uint32_t k = 0; k + 1 < np; k++) {
        uint64_t a = pts[k], b = pts[k + 1];
        if (a == b) continue;
        int best = 0; uint32_t bt = 0;
        for (uint32_t i = 0; i < m->n; i++) {
            if (m->e[i].base <= a && b <= m->e[i].base + m->e[i].length) {
                int p = prio(m->e[i].type);
                if (p > best) { best = p; bt = m->e[i].type; }
            }
        }
        if (!best) continue;                              /* hole */
        if (no && out[no - 1].type == bt && out[no - 1].base + out[no - 1].length == a) {
            out[no - 1].length += b - a;
        } else {
            if (no >= MM_CAP) return false;
            out[no].base = a; out[no].length = b - a; out[no].type = bt; out[no].attr = 0;
            no++;
        }
    }
    for (uint32_t i = 0; i < no; i++) m->e[i] = out[i];
    m->n = no;
    return true;
}

bool mm_covered_by(const struct mm *m, uint64_t base, uint64_t length, uint32_t want)
{
    uint64_t end = base + length;
    if (end < base) return false;
    uint64_t cur = base;
    while (cur < end) {
        bool hit = false;
        for (uint32_t i = 0; i < m->n; i++) {
            if (m->e[i].type == want && m->e[i].base <= cur && cur < m->e[i].base + m->e[i].length) {
                cur = m->e[i].base + m->e[i].length;
                hit = true;
                break;
            }
        }
        if (!hit) return false;
    }
    return true;
}

bool mm_is_ram(const struct mm *m, uint64_t base, uint64_t length)
{
    uint64_t end = base + length;
    if (end < base) return false;
    uint64_t cur = base;
    while (cur < end) {
        bool hit = false;
        for (uint32_t i = 0; i < m->n; i++) {
            uint32_t t = m->e[i].type;
            bool ram = t == LOOS_MEM_USABLE || t == LOOS_MEM_BOOT || t == LOOS_MEM_ACPI_RECLAIM ||
                       t == LOOS_MEM_KERNEL || t == LOOS_MEM_MODULE;
            if (ram && m->e[i].base <= cur && cur < m->e[i].base + m->e[i].length) {
                cur = m->e[i].base + m->e[i].length;
                hit = true;
                break;
            }
        }
        if (!hit) return false;
    }
    return true;
}
