/* Loader core: the shared, firmware-independent part of the LoOS boot loader.
 *
 * A front end (BIOS stage 2 or the UEFI application) gathers what only firmware can provide - physical memory map,
 * framebuffer, files already read into RAM, ACPI RSDP, a scratch arena - fills struct loader_env and calls
 * loader_prepare() (pure, host-testable) or loader_main() (prepare + switch to the kernel). */
#ifndef LOOS_LOADER_H
#define LOOS_LOADER_H
#include <stddef.h>
#include <stdint.h>
#include "../../src/kernel/boot/bootinfo.h"

#define LOADER_ENV_MAGIC  0x5645444145444f4cull      /* "LOADEDEV" little endian */
#define LOADER_ENV_VERSION 1u
#define LOADER_MAX_FILES  (1 + LOOS_BOOT_MAX_MODULES)  /* file 0 is the kernel */
#define LOADER_MAX_RESERVED 8
#define LOADER_MAX_RAW_MMAP 256

#define LOADER_ENV_UEFI       (1u << 0)
#define LOADER_ENV_NO_BIOS_SCAN (1u << 1)            /* do not scan low memory for the RSDP (UEFI / host tests) */

struct loader_file { char name[16]; uint64_t addr, size; };   /* contents already in RAM at addr */
struct loader_range { uint64_t base, length; };

struct loader_env {
    uint64_t magic;
    uint32_t version, flags;
    uint64_t mmap_phys;                      /* raw E820-format entries (type 1..5, attr bit0 = valid) */
    uint32_t mmap_count, nreserved;
    struct loader_range reserved[LOADER_MAX_RESERVED];   /* loader stack/low structures: kept as LOOS_MEM_BOOT */
    struct loader_range image;               /* the loader's own code: stays executable (and BOOT) across the CR3 switch */
    uint64_t scratch_base, scratch_size;     /* usable RAM, 4 KiB aligned, below 4 GiB: bootinfo, tables, kernel image */
    uint64_t rsdp_phys;
    uint64_t fb_phys;
    uint32_t fb_pitch, fb_width, fb_height, fb_bpp, fb_format, nfiles;
    uint64_t seed;                           /* KASLR entropy supplied by the front end (0 = derive from the CPU) */
    struct loader_file files[LOADER_MAX_FILES];
    char cmdline[LOOS_BOOT_CMDLINE];
};

enum loader_status {
    LOADER_OK = 0,
    LOADER_E_ENV = -1,        /* malformed environment */
    LOADER_E_MEMORY = -2,     /* scratch/files not inside usable RAM, or scratch too small */
    LOADER_E_ELF = -3,        /* kernel image rejected (see loader_result.detail) */
    LOADER_E_PAGING = -4,
    LOADER_E_MMAP = -5,
};

struct loader_result {
    int status;
    int detail;                              /* ELF error code when status == LOADER_E_ELF */
    struct loos_bootinfo *bootinfo;          /* physical == loader-visible address */
    uint64_t cr3, entry_virt;
};

void loader_prepare(struct loader_env *env, struct loader_result *out);
const char *loader_strerror(const struct loader_result *r);
/* Prepares and jumps to the kernel; on failure returns the status so the front end can report it. */
int loader_main(struct loader_env *env);
#endif
