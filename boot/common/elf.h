/* ELF64 loader for the kernel image: strict validation of untrusted input, segment layout, and
 * R_X86_64_RELATIVE relocation (the kernel is a static PIE so the loader can slide it for KASLR). */
#ifndef LOOS_LOADER_ELF_H
#define LOOS_LOADER_ELF_H
#include "base.h"

#define ELF_MAX_LOAD 8
#define ELF_PF_X 1u
#define ELF_PF_W 2u
#define ELF_PF_R 4u

enum elf_error {
    ELF_OK = 0,
    ELF_E_TRUNCATED = -1,    /* header/program headers outside the file */
    ELF_E_MAGIC = -2,
    ELF_E_CLASS = -3,        /* not ELF64 little-endian x86-64 ET_DYN */
    ELF_E_PHDR = -4,
    ELF_E_SEGMENT = -5,      /* bad PT_LOAD geometry */
    ELF_E_TOO_BIG = -6,      /* image does not fit the kernel window */
    ELF_E_ENTRY = -7,
    ELF_E_DYNAMIC = -8,
    ELF_E_RELOC = -9,        /* unsupported or out-of-range relocation */
    ELF_E_NOLOAD = -10,
};

struct elf_seg { uint64_t off, vaddr, filesz, memsz; uint32_t flags; };

struct elf_info {
    uint64_t entry;                  /* link-time virtual address */
    uint32_t nload;
    struct elf_seg load[ELF_MAX_LOAD];
    uint64_t span;                   /* page-rounded extent of the image from vaddr 0 */
    uint64_t rela_off, rela_size;    /* file offset / bytes of the RELA table (0 if none) */
};

/* Validates `file` (untrusted) and fills `out`. `window` bounds the image's virtual extent. */
int elf_parse(const void *file, uint64_t size, uint64_t window, struct elf_info *out);
/* Copies segments into `dest` (span bytes, link address 0), zero-fills BSS and applies relocations for a load at
 * virtual base `vbase`. */
int elf_load(const struct elf_info *info, const void *file, uint64_t size, uint8_t *dest, uint64_t vbase);
const char *elf_strerror(int code);
#endif
