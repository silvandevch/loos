/* Boot protocol: the handoff block built by the LoOS boot loader (BIOS or UEFI front end, shared loader core in
 * boot/common) and passed to the kernel in RDI as a *physical* pointer (identity mapped at entry).
 *
 * State at kernel entry: long mode, paging on (CR3 = bootinfo.cr3), CR0.WP set, EFER.NXE set, interrupts off.
 * The kernel is mapped at bootinfo.kernel_virt (higher half, possibly KASLR-slid) with W^X per segment; low 4 GiB
 * plus all RAM and the framebuffer are identity mapped (RAM RW+NX, MMIO uncached) until the kernel builds its own
 * tables. The loader's own image/stack stay valid (type LOOS_MEM_BOOT) until the kernel reclaims them. */
#ifndef LOOS_BOOTINFO_H
#define LOOS_BOOTINFO_H
#include <stdint.h>

#define LOOS_BOOT_MAGIC     0x544f4f42534f4f4cull      /* "LOOSBOOT" little endian */
#define LOOS_BOOT_VERSION   2u

/* Memory types. 1..5 are the ACPI/E820 numbers; the rest are loader-defined. */
enum loos_mem_type {
    LOOS_MEM_USABLE       = 1,
    LOOS_MEM_RESERVED     = 2,
    LOOS_MEM_ACPI_RECLAIM = 3,
    LOOS_MEM_ACPI_NVS     = 4,
    LOOS_MEM_BAD          = 5,
    LOOS_MEM_BOOT         = 16,    /* loader image, stack, bootinfo, page tables: reclaimable after entry */
    LOOS_MEM_KERNEL       = 17,    /* the loaded kernel image */
    LOOS_MEM_MODULE       = 18,    /* boot modules (initrd, boot drivers, registry hive, ...) */
};

struct loos_mem_entry {
    uint64_t base, length;
    uint32_t type;
    uint32_t attr;                 /* raw input: ACPI 3.0 extended attribute (bit0 = valid); output: 0 */
};

#define LOOS_BOOT_FLAG_UEFI    (1u << 0)
#define LOOS_BOOT_FLAG_KASLR   (1u << 1)   /* kernel_virt was slid */

#define LOOS_FB_BGRX  0                /* byte order B,G,R,X in memory (the common case) */
#define LOOS_FB_RGBX  1

#define LOOS_BOOT_MAX_MODULES 8
#define LOOS_BOOT_CMDLINE     256

struct loos_boot_module {
    char name[16];
    uint64_t phys, size;
};

struct loos_bootinfo {
    uint64_t magic;
    uint32_t version, size;                  /* sizeof(struct loos_bootinfo) as built */
    uint32_t flags, mmap_count;
    uint64_t mmap_phys;                      /* array of loos_mem_entry, sorted, non-overlapping */
    uint64_t rsdp_phys;                      /* ACPI RSDP, 0 if none found */
    uint64_t kernel_phys, kernel_size;       /* contiguous physical image */
    uint64_t kernel_virt;                    /* virtual address of image offset 0 */
    uint64_t cr3;                            /* PML4 physical address */
    uint64_t fb_phys;                        /* 0 = no linear framebuffer */
    uint32_t fb_pitch, fb_width, fb_height, fb_bpp, fb_format, module_count;
    struct loos_boot_module modules[LOOS_BOOT_MAX_MODULES];
    char cmdline[LOOS_BOOT_CMDLINE];
};

_Static_assert(sizeof(struct loos_mem_entry) == 24, "E820-compatible entry");
_Static_assert(sizeof(struct loos_boot_module) == 32, "module layout");
#endif
