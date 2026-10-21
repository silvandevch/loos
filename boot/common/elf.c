#include "elf.h"

#define EM_X86_64 62
#define ET_DYN 3
#define PT_LOAD 1
#define PT_DYNAMIC 2
#define DT_NULL 0
#define DT_RELA 7
#define DT_RELASZ 8
#define DT_RELAENT 9
#define R_X86_64_RELATIVE 8

struct ehdr {
    uint8_t ident[16];
    uint16_t type, machine;
    uint32_t version;
    uint64_t entry, phoff, shoff;
    uint32_t flags;
    uint16_t ehsize, phentsize, phnum, shentsize, shnum, shstrndx;
};
struct phdr { uint32_t type, flags; uint64_t off, vaddr, paddr, filesz, memsz, align; };
struct dyn { int64_t tag; uint64_t val; };
struct rela { uint64_t off; uint64_t info; int64_t addend; };

static bool in_file(uint64_t size, uint64_t off, uint64_t len) { return off <= size && len <= size - off; }

/* file offset of a link-time virtual address that must lie inside a segment's file-backed bytes */
static bool vaddr_to_off(const struct elf_info *i, uint64_t va, uint64_t len, uint64_t *off)
{
    for (uint32_t k = 0; k < i->nload; k++) {
        const struct elf_seg *s = &i->load[k];
        if (va >= s->vaddr && va - s->vaddr <= s->filesz && len <= s->filesz - (va - s->vaddr)) {
            *off = s->off + (va - s->vaddr);
            return true;
        }
    }
    return false;
}

int elf_parse(const void *file, uint64_t size, uint64_t window, struct elf_info *out)
{
    memset(out, 0, sizeof *out);
    if (size < sizeof(struct ehdr)) return ELF_E_TRUNCATED;
    const struct ehdr *h = file;
    if (memcmp(h->ident, "\x7f""ELF", 4) != 0) return ELF_E_MAGIC;
    if (h->ident[4] != 2 || h->ident[5] != 1 || h->type != ET_DYN || h->machine != EM_X86_64 || h->ident[6] != 1)
        return ELF_E_CLASS;
    if (h->phentsize != sizeof(struct phdr) || h->phnum == 0 || h->phnum > 32) return ELF_E_PHDR;
    if (!in_file(size, h->phoff, (uint64_t)h->phnum * sizeof(struct phdr))) return ELF_E_TRUNCATED;
    const struct phdr *ph = (const struct phdr *)((const uint8_t *)file + h->phoff);

    uint64_t prev_end = 0;
    const struct phdr *dynp = NULL;
    for (uint32_t i = 0; i < h->phnum; i++) {
        if (ph[i].type == PT_DYNAMIC) { dynp = &ph[i]; continue; }
        if (ph[i].type != PT_LOAD) continue;
        const struct phdr *p = &ph[i];
        if (out->nload == ELF_MAX_LOAD) return ELF_E_SEGMENT;
        if (p->memsz == 0) continue;
        if (p->filesz > p->memsz || !in_file(size, p->off, p->filesz)) return ELF_E_SEGMENT;
        if ((p->vaddr ^ p->off) & (PAGE_SIZE - 1)) return ELF_E_SEGMENT;           /* page congruence */
        if (p->vaddr + p->memsz < p->vaddr || p->vaddr + p->memsz > window) return ELF_E_TOO_BIG;
        /* ascending, and no page shared between segments (each gets its own protection) */
        if (out->nload && ALIGN_DOWN(p->vaddr, PAGE_SIZE) < prev_end) return ELF_E_SEGMENT;
        if (p->flags & ELF_PF_W && p->flags & ELF_PF_X) return ELF_E_SEGMENT;     /* W^X */
        out->load[out->nload++] = (struct elf_seg){ p->off, p->vaddr, p->filesz, p->memsz, p->flags };
        prev_end = ALIGN_UP(p->vaddr + p->memsz, PAGE_SIZE);
    }
    if (!out->nload) return ELF_E_NOLOAD;
    out->span = prev_end;

    bool entry_ok = false;
    for (uint32_t k = 0; k < out->nload; k++) {
        const struct elf_seg *s = &out->load[k];
        if ((s->flags & ELF_PF_X) && h->entry >= s->vaddr && h->entry < s->vaddr + s->filesz) entry_ok = true;
    }
    if (!entry_ok) return ELF_E_ENTRY;
    out->entry = h->entry;

    if (dynp) {
        if (dynp->filesz % sizeof(struct dyn) || !in_file(size, dynp->off, dynp->filesz)) return ELF_E_DYNAMIC;
        const struct dyn *d = (const struct dyn *)((const uint8_t *)file + dynp->off);
        uint64_t rela = 0, relasz = 0, relaent = sizeof(struct rela);
        bool have_rela = false;
        for (uint64_t k = 0; k < dynp->filesz / sizeof *d; k++) {
            if (d[k].tag == DT_NULL) break;
            if (d[k].tag == DT_RELA) { rela = d[k].val; have_rela = true; }
            else if (d[k].tag == DT_RELASZ) relasz = d[k].val;
            else if (d[k].tag == DT_RELAENT) relaent = d[k].val;
        }
        if (relaent != sizeof(struct rela) || relasz % sizeof(struct rela)) return ELF_E_DYNAMIC;
        if (have_rela && relasz) {
            if (!vaddr_to_off(out, rela, relasz, &out->rela_off)) return ELF_E_DYNAMIC;
            out->rela_size = relasz;
        }
    }
    return ELF_OK;
}

int elf_load(const struct elf_info *info, const void *file, uint64_t size, uint8_t *dest, uint64_t vbase)
{
    (void)size;
    memset(dest, 0, info->span);
    for (uint32_t k = 0; k < info->nload; k++) {
        const struct elf_seg *s = &info->load[k];
        memcpy(dest + s->vaddr, (const uint8_t *)file + s->off, s->filesz);
    }
    const uint8_t *rp = (const uint8_t *)file + info->rela_off;
    for (uint64_t k = 0; k < info->rela_size / sizeof(struct rela); k++) {
        struct rela r;
        memcpy(&r, rp + k * sizeof r, sizeof r);
        if ((r.info & 0xffffffffu) != R_X86_64_RELATIVE || (r.info >> 32) != 0) return ELF_E_RELOC;
        /* the target must be file-backed (the loader never relocates into BSS or outside the image) */
        uint64_t unused;
        if (!vaddr_to_off(info, r.off, 8, &unused)) return ELF_E_RELOC;
        uint64_t v = vbase + (uint64_t)r.addend;
        memcpy(dest + r.off, &v, 8);
    }
    return ELF_OK;
}

const char *elf_strerror(int c)
{
    switch (c) {
    case ELF_OK: return "ok";
    case ELF_E_TRUNCATED: return "kernel image truncated";
    case ELF_E_MAGIC: return "kernel image is not an ELF file";
    case ELF_E_CLASS: return "kernel image is not a little-endian x86-64 ELF64 ET_DYN";
    case ELF_E_PHDR: return "bad ELF program headers";
    case ELF_E_SEGMENT: return "bad PT_LOAD segment (geometry or W+X)";
    case ELF_E_TOO_BIG: return "kernel image does not fit the kernel window";
    case ELF_E_ENTRY: return "entry point outside executable segments";
    case ELF_E_DYNAMIC: return "bad PT_DYNAMIC";
    case ELF_E_RELOC: return "unsupported relocation";
    default: return "no loadable segments";
    }
}
