/* Host tests for the shared loader core (boot/common): memory-map normalisation, ELF64 validation and relocation,
 * page-table construction, and the full loader_prepare() pipeline against the real stage-0 kernel image. */
#define LOADER_HOST_TEST 1
#include "../../boot/common/loader.h"
#include "../../boot/common/elf.h"
#include "../../boot/common/mmap.h"
#include "../../boot/common/paging.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>

static int fails;
#define CHECK(c) do { if (!(c)) { printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c); fails++; } } while (0)

/* ---------------------------------------------------------------------------------------------- mmap */
static void test_mmap(void)
{
    static struct mm m;
    mm_init(&m);
    mm_add(&m, 0x0, 0x9fc00, LOOS_MEM_USABLE);                 /* shrinks to 0x0..0x9f000 */
    mm_add(&m, 0x9fc00, 0x400, LOOS_MEM_RESERVED);
    mm_add(&m, 0x100000, 0x1000000, LOOS_MEM_USABLE);
    mm_add(&m, 0x200000, 0x1000, LOOS_MEM_BOOT);               /* carved out of usable */
    mm_add(&m, 0x200800, 0x1000, LOOS_MEM_KERNEL);             /* overlaps the BOOT page: kernel wins */
    mm_add(&m, 0x500000, 0x1000, LOOS_MEM_RESERVED);
    mm_add(&m, 0x500000, 0x2000, LOOS_MEM_USABLE);             /* reserved beats usable */
    CHECK(mm_normalize(&m));
    uint64_t prev = 0;
    for (uint32_t i = 0; i < m.n; i++) {
        CHECK(m.e[i].base >= prev);
        prev = m.e[i].base + m.e[i].length;
        if (i) CHECK(!(m.e[i].type == m.e[i - 1].type && m.e[i].base == m.e[i - 1].base + m.e[i - 1].length));   /* merged */
    }
    CHECK(mm_covered_by(&m, 0x1000, 0x9e000, LOOS_MEM_USABLE));
    CHECK(!mm_covered_by(&m, 0x9e000, 0x2000, LOOS_MEM_USABLE));              /* sliver 0x9f000..0x9fc00 is a hole */
    CHECK(mm_covered_by(&m, 0x200800, 0x1000, LOOS_MEM_KERNEL));
    CHECK(mm_covered_by(&m, 0x200000, 0x800, LOOS_MEM_BOOT));
    CHECK(mm_covered_by(&m, 0x500000, 0x1000, LOOS_MEM_RESERVED));
    CHECK(mm_covered_by(&m, 0x501000, 0x1000, LOOS_MEM_USABLE));
    CHECK(mm_is_ram(&m, 0x100000, 0x1000) && !mm_is_ram(&m, 0x500000, 0x1000) && !mm_is_ram(&m, 0x9f000, 0x100));
    mm_init(&m);                                                               /* overflow is reported, not ignored */
    for (int i = 0; i < MM_CAP + 1; i++) mm_add(&m, (uint64_t)i * 0x2000, 0x1000, LOOS_MEM_RESERVED);
    CHECK(!mm_normalize(&m));
    mm_init(&m);
    mm_add(&m, ~0ull - 0xfff, 0x10000, LOOS_MEM_RESERVED);                     /* wrap-around is clamped */
    CHECK(mm_normalize(&m) && m.n == 1 && m.e[0].base + m.e[0].length == ~0ull);
}

/* ---------------------------------------------------------------------------------------------- paging */
static void test_paging(void)
{
    uint8_t *mem = mmap(0, 64 * 4096, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    struct pt p;
    CHECK(pt_init(&p, mem, 64 * 4096));
    CHECK(pt_map4k(&p, 0xffffffff80001000ull, 0x5000, PT_W | PT_NX));
    CHECK(!pt_map4k(&p, 0xffffffff80001000ull, 0x6000, PT_W));                 /* no silent remap */
    CHECK(!pt_map4k(&p, 0xffffffff80001001ull, 0x6000, PT_W));                 /* alignment */
    uint64_t sz = 0, e = pt_lookup(&p, 0xffffffff80001abcull, &sz);
    CHECK(e && sz == 4096 && (e & PT_ADDR_MASK) == 0x5000 && (e & PT_NX) && (e & PT_W));
    CHECK(!pt_lookup(&p, 0xffffffff80002000ull, 0));
    CHECK(pt_map2m(&p, 0x40000000ull, 0x40000000ull, PT_W));
    e = pt_lookup(&p, 0x40123456ull, &sz);
    CHECK(e && sz == 0x200000 && (e & PT_PS));
    CHECK(!pt_map4k(&p, 0x40001000ull, 0x1000, PT_W));                         /* inside a 2 MiB page */
    CHECK(!pt_map2m(&p, 0xffffffff80000000ull, 0, PT_W));                      /* a 4K table is already there */
    struct pt tiny;
    CHECK(pt_init(&tiny, mem + 4096, 3 * 4096));                               /* PML4 + 2 more tables */
    CHECK(!pt_map4k(&tiny, 0x1000, 0x1000, PT_W));                             /* needs PDPT+PD+PT: arena exhausted */
    munmap(mem, 64 * 4096);
}

/* ---------------------------------------------------------------------------------------------- elf */
static uint8_t *slurp(const char *path, size_t *n)
{
    FILE *f = fopen(path, "rb");
    if (!f) { printf("cannot open %s\n", path); exit(2); }
    fseek(f, 0, SEEK_END); *n = (size_t)ftell(f); fseek(f, 0, SEEK_SET);
    uint8_t *b = malloc(*n);
    if (fread(b, 1, *n, f) != *n) exit(2);
    fclose(f);
    return b;
}

struct ehdr { uint8_t ident[16]; uint16_t type, machine; uint32_t version; uint64_t entry, phoff, shoff; uint32_t flags;
              uint16_t ehsize, phentsize, phnum, shentsize, shnum, shstrndx; };
struct phdr { uint32_t type, flags; uint64_t off, vaddr, paddr, filesz, memsz, align; };

static void test_elf(const uint8_t *good, size_t n)
{
    struct elf_info ei;
    CHECK(elf_parse(good, n, 1ull << 30, &ei) == ELF_OK);
    CHECK(ei.nload >= 3 && ei.span > (3u << 20) && ei.rela_size > 0);
    CHECK(elf_parse(good, n, 1 << 20, &ei) == ELF_E_TOO_BIG);                  /* window enforcement */
    CHECK(elf_parse(good, 40, 1ull << 30, &ei) == ELF_E_TRUNCATED);
    CHECK(elf_parse(good, n / 2, 1ull << 30, &ei) != ELF_OK);                  /* truncated body */

    uint8_t *b = malloc(n);
    memcpy(b, good, n); b[1] = 'X';
    CHECK(elf_parse(b, n, 1ull << 30, &ei) == ELF_E_MAGIC);
    memcpy(b, good, n); b[4] = 1;                                              /* ELFCLASS32 */
    CHECK(elf_parse(b, n, 1ull << 30, &ei) == ELF_E_CLASS);
    memcpy(b, good, n); ((struct ehdr *)b)->type = 2;                          /* ET_EXEC */
    CHECK(elf_parse(b, n, 1ull << 30, &ei) == ELF_E_CLASS);
    memcpy(b, good, n); ((struct ehdr *)b)->phnum = 0;
    CHECK(elf_parse(b, n, 1ull << 30, &ei) == ELF_E_PHDR);
    memcpy(b, good, n); ((struct ehdr *)b)->phoff = n - 8;                     /* program headers run off the file */
    CHECK(elf_parse(b, n, 1ull << 30, &ei) == ELF_E_TRUNCATED);
    memcpy(b, good, n); ((struct ehdr *)b)->entry = 0x1000000;
    CHECK(elf_parse(b, n, 1ull << 30, &ei) == ELF_E_ENTRY);

    struct ehdr *h = (struct ehdr *)good;
    struct phdr *ph = (struct phdr *)(good + h->phoff);
    for (int i = 0; i < h->phnum; i++) {
        if (ph[i].type != 1) continue;
        struct phdr *q = (struct phdr *)(b + h->phoff) + i;
        memcpy(b, good, n); q->flags = 7;                                      /* W+X */
        CHECK(elf_parse(b, n, 1ull << 30, &ei) == ELF_E_SEGMENT);
        memcpy(b, good, n); q->filesz = q->memsz + 1;
        CHECK(elf_parse(b, n, 1ull << 30, &ei) == ELF_E_SEGMENT);
        memcpy(b, good, n); q->off += 1;                                       /* breaks vaddr/offset page congruence */
        CHECK(elf_parse(b, n, 1ull << 30, &ei) == ELF_E_SEGMENT);
        memcpy(b, good, n); q->off = n - 4; q->filesz = 64;                    /* file range outside the file */
        CHECK(elf_parse(b, n, 1ull << 30, &ei) == ELF_E_SEGMENT);
        break;
    }

    CHECK(elf_parse(good, n, 1ull << 30, &ei) == ELF_OK);
    uint8_t *dest = calloc(1, ei.span);
    CHECK(elf_load(&ei, good, n, dest, 0xffffffffa0000000ull) == ELF_OK);
    struct { uint64_t off, info; int64_t addend; } r;
    int checked = 0;
    for (uint64_t k = 0; k < ei.rela_size / sizeof r; k++) {
        memcpy(&r, good + ei.rela_off + k * sizeof r, sizeof r);
        uint64_t v; memcpy(&v, dest + r.off, 8);
        CHECK(v == 0xffffffffa0000000ull + (uint64_t)r.addend);
        checked++;
    }
    CHECK(checked == (int)(ei.rela_size / sizeof r) && checked > 0);
    for (uint64_t i = 0; i < ei.nload; i++)                                    /* BSS beyond filesz is zero */
        for (uint64_t k = ei.load[i].filesz; k < ei.load[i].memsz; k += 997) CHECK(dest[ei.load[i].vaddr + k] == 0);
    /* an unsupported relocation type is rejected, not applied */
    memcpy(b, good, n);
    memcpy(&r, b + ei.rela_off, sizeof r); r.info = (r.info & ~0xffffffffull) | 1;   /* R_X86_64_64 */
    memcpy(b + ei.rela_off, &r, sizeof r);
    CHECK(elf_parse(b, n, 1ull << 30, &ei) == ELF_OK && elf_load(&ei, b, n, dest, 0xffffffffa0000000ull) == ELF_E_RELOC);
    memcpy(b, good, n);                                                        /* relocation target outside the image */
    memcpy(&r, b + ei.rela_off, sizeof r); r.off = ei.span + 8;
    memcpy(b + ei.rela_off, &r, sizeof r);
    CHECK(elf_load(&ei, b, n, dest, 0xffffffffa0000000ull) == ELF_E_RELOC);
    free(dest); free(b);
}

/* ---------------------------------------------------------------------------------------------- full pipeline */
#define ARENA (96u << 20)
static uint8_t *arena;
static struct loos_mem_entry raw[8];
static struct loader_env env;

static void make_env(const uint8_t *kernel, size_t ksize, const char *cmdline)
{
    memset(&env, 0, sizeof env);
    uint64_t base = (uint64_t)(uintptr_t)arena;
    raw[0] = (struct loos_mem_entry){ 0, 0x9fc00, 1, 1 };
    raw[1] = (struct loos_mem_entry){ 0xf0000, 0x10000, 2, 1 };
    raw[2] = (struct loos_mem_entry){ base, ARENA, 1, 1 };
    raw[3] = (struct loos_mem_entry){ 0xfee00000, 0x100000, 2, 1 };
    raw[4] = (struct loos_mem_entry){ 0x200000000ull, 64ull << 20, 1, 1 };            /* RAM above 4 GiB */
    raw[5] = (struct loos_mem_entry){ 0x300000, 0x1000, 1, 0 };                       /* ACPI: attribute bit 0 clear = ignore */
    env.magic = LOADER_ENV_MAGIC; env.version = LOADER_ENV_VERSION; env.flags = LOADER_ENV_NO_BIOS_SCAN;
    env.mmap_phys = (uint64_t)(uintptr_t)raw; env.mmap_count = 6;
    memcpy(arena, kernel, ksize);
    memcpy(env.files[0].name, "kernel", 7);
    env.files[0].addr = base; env.files[0].size = ksize;
    memset(arena + (8u << 20), 0x5a, 40000);
    memcpy(env.files[1].name, "initrd", 7);
    env.files[1].addr = base + (8u << 20); env.files[1].size = 40000;
    env.nfiles = 2;
    env.scratch_base = base + (16u << 20); env.scratch_size = 48u << 20;
    env.image = (struct loader_range){ 0x7e00, 0x20000 };
    env.reserved[0] = (struct loader_range){ 0x30000, 0x8000 }; env.nreserved = 1;
    env.fb_phys = 0xfd000000; env.fb_pitch = 4096; env.fb_width = 1024; env.fb_height = 768; env.fb_bpp = 32;
    env.seed = 0x1badb002;
    if (cmdline) strncpy(env.cmdline, cmdline, sizeof env.cmdline - 1);
}

static uint32_t type_at(const struct loos_bootinfo *bi, uint64_t a)
{
    const struct loos_mem_entry *e = (const void *)(uintptr_t)bi->mmap_phys;
    for (uint32_t i = 0; i < bi->mmap_count; i++) if (e[i].base <= a && a < e[i].base + e[i].length) return e[i].type;
    return 0;
}

static void test_pipeline(const uint8_t *kernel, size_t ksize)
{
    struct loader_result r;
    make_env(kernel, ksize, "console=ttyS0");
    loader_prepare(&env, &r);
    CHECK(r.status == LOADER_OK);
    if (r.status != LOADER_OK) { printf("  %s\n", loader_strerror(&r)); return; }
    const struct loos_bootinfo *bi = r.bootinfo;
    CHECK(bi->magic == LOOS_BOOT_MAGIC && bi->version == LOOS_BOOT_VERSION && bi->size == sizeof *bi);
    CHECK(bi->kernel_virt >= 0xffffffff80000000ull && bi->kernel_virt % 0x200000 == 0 && bi->kernel_virt + bi->kernel_size <= 0xffffffffc0000000ull);
    CHECK((bi->flags & LOOS_BOOT_FLAG_KASLR) && !(bi->flags & LOOS_BOOT_FLAG_UEFI));
    CHECK(bi->module_count == 1 && !strcmp(bi->modules[0].name, "initrd") && bi->modules[0].size == 40000 && bi->modules[0].phys == env.files[1].addr);
    CHECK(!strcmp(bi->cmdline, "console=ttyS0"));
    CHECK(bi->fb_phys == 0xfd000000 && bi->fb_width == 1024 && bi->fb_bpp == 32);
    CHECK(r.cr3 == bi->cr3 && bi->cr3 >= env.scratch_base && bi->cr3 < env.scratch_base + env.scratch_size && !(bi->cr3 & 4095));
    CHECK(r.entry_virt >= bi->kernel_virt && r.entry_virt < bi->kernel_virt + bi->kernel_size);

    /* relocations landed in the physical image */
    struct elf_info ei; elf_parse(kernel, ksize, 1ull << 30, &ei);
    struct { uint64_t off, info; int64_t addend; } rl;
    for (uint64_t k = 0; k < ei.rela_size / sizeof rl; k++) {
        memcpy(&rl, kernel + ei.rela_off + k * sizeof rl, sizeof rl);
        uint64_t v; memcpy(&v, (uint8_t *)(uintptr_t)bi->kernel_phys + rl.off, 8);
        CHECK(v == bi->kernel_virt + (uint64_t)rl.addend);
    }

    /* page tables: per-segment W^X, exact physical backing, nothing mapped between/after segments */
    struct pt view = { .pml4 = (uint64_t *)(uintptr_t)bi->cr3 };
    for (uint32_t s = 0; s < ei.nload; s++) {
        for (uint64_t a = ei.load[s].vaddr & ~4095ull; a < ei.load[s].vaddr + ei.load[s].memsz; a += 4096) {
            uint64_t sz, e = pt_lookup(&view, bi->kernel_virt + a, &sz);
            CHECK(e && sz == 4096 && (e & PT_ADDR_MASK) == bi->kernel_phys + a);
            CHECK(!!(e & PT_W) == !!(ei.load[s].flags & ELF_PF_W));
            CHECK(!!(e & PT_NX) == !(ei.load[s].flags & ELF_PF_X));
            CHECK(!(e & PT_UC));
        }
    }
    CHECK(!pt_lookup(&view, bi->kernel_virt + ei.span, 0));
    CHECK(!pt_lookup(&view, bi->kernel_virt - 0x1000, 0));

    uint64_t sz, e = pt_lookup(&view, 0x100000, &sz);                          /* low RAM page that also holds the loader: RWX */
    CHECK(e && sz == 0x200000 && (e & PT_W) && !(e & PT_NX) && !(e & PT_UC));
    e = pt_lookup(&view, (uint64_t)(uintptr_t)arena + (10u << 20), &sz);       /* ordinary RAM: RW NX cacheable */
    CHECK(e && sz == 0x200000 && (e & PT_W) && (e & PT_NX) && !(e & PT_UC));
    e = pt_lookup(&view, 0xfee00000, &sz);                                     /* LAPIC window: uncached */
    CHECK(e && (e & PT_UC) == PT_UC && (e & PT_NX));
    e = pt_lookup(&view, 0x7e00, &sz);                                         /* loader image: still executable */
    CHECK(e && !(e & PT_NX));
    e = pt_lookup(&view, 0x400000, &sz);
    CHECK(e && (e & PT_NX));
    e = pt_lookup(&view, 0x200000000ull + 0x1000, &sz);                        /* RAM above 4 GiB */
    CHECK(e && (e & PT_W) && (e & PT_NX));
    e = pt_lookup(&view, 0xfd000000 + 4096 * 767, &sz);                        /* framebuffer: write-combining (PAT entry 4) */
    CHECK(e && (e & PT_PAT2M) && !(e & PT_UC) && (e & PT_NX));

    /* final memory map */
    const struct loos_mem_entry *m = (const void *)(uintptr_t)bi->mmap_phys;
    for (uint32_t i = 1; i < bi->mmap_count; i++) CHECK(m[i].base >= m[i - 1].base + m[i - 1].length);
    CHECK(type_at(bi, 0x800) == LOOS_MEM_RESERVED);                            /* page 0 */
    CHECK(type_at(bi, 0x7e00) == LOOS_MEM_BOOT && type_at(bi, 0x30000) == LOOS_MEM_BOOT);
    CHECK(type_at(bi, 0xf0000) == LOOS_MEM_RESERVED && type_at(bi, 0xfee00000) == LOOS_MEM_RESERVED);
    CHECK(type_at(bi, 0x300000) == 0);                                         /* ignored entry stays a hole */
    CHECK(type_at(bi, bi->kernel_phys) == LOOS_MEM_KERNEL && type_at(bi, bi->kernel_phys + ei.span - 1) == LOOS_MEM_KERNEL);
    CHECK(type_at(bi, env.files[1].addr) == LOOS_MEM_MODULE);
    CHECK(type_at(bi, env.files[0].addr) == LOOS_MEM_USABLE);                  /* the raw kernel file is reclaimable */
    CHECK(type_at(bi, env.scratch_base) == LOOS_MEM_BOOT);
    CHECK(type_at(bi, env.scratch_base + env.scratch_size - 4096) == LOOS_MEM_USABLE);       /* unused scratch is given back */
    CHECK(type_at(bi, bi->cr3) == LOOS_MEM_BOOT);
    CHECK(type_at(bi, 0x200000000ull) == LOOS_MEM_USABLE);

    /* KASLR: different seeds slide differently; "nokaslr" pins the base; every slide stays in the window */
    uint64_t seen[16]; int distinct = 0;
    for (uint64_t seed = 1; seed <= 16; seed++) {
        make_env(kernel, ksize, 0); env.seed = seed * 0x9e3779b97f4a7c15ull;
        loader_prepare(&env, &r);
        CHECK(r.status == LOADER_OK && r.bootinfo->kernel_virt + r.bootinfo->kernel_size <= 0xffffffffc0000000ull);
        int dup = 0; for (int i = 0; i < distinct; i++) if (seen[i] == r.bootinfo->kernel_virt) dup = 1;
        if (!dup) seen[distinct++] = r.bootinfo->kernel_virt;
    }
    CHECK(distinct >= 8);
    make_env(kernel, ksize, "quiet nokaslr");
    loader_prepare(&env, &r);
    CHECK(r.status == LOADER_OK && r.bootinfo->kernel_virt == 0xffffffff80000000ull && !(r.bootinfo->flags & LOOS_BOOT_FLAG_KASLR));
    make_env(kernel, ksize, "nokaslrx");                                       /* only a whole word counts */
    loader_prepare(&env, &r);
    CHECK(r.status == LOADER_OK && (r.bootinfo->flags & LOOS_BOOT_FLAG_KASLR));
}

static void test_rejects(const uint8_t *kernel, size_t ksize)
{
    struct loader_result r;
    make_env(kernel, ksize, 0); env.magic = 0;                                 loader_prepare(&env, &r); CHECK(r.status == LOADER_E_ENV);
    make_env(kernel, ksize, 0); env.nfiles = 0;                                loader_prepare(&env, &r); CHECK(r.status == LOADER_E_ENV);
    make_env(kernel, ksize, 0); env.mmap_count = 0;                            loader_prepare(&env, &r); CHECK(r.status == LOADER_E_ENV);
    make_env(kernel, ksize, 0); env.scratch_size = 4096;                       loader_prepare(&env, &r); CHECK(r.status == LOADER_E_ENV);
    make_env(kernel, ksize, 0); env.scratch_base += 7;                         loader_prepare(&env, &r); CHECK(r.status == LOADER_E_ENV);
    make_env(kernel, ksize, 0); env.scratch_size = 4u << 20;                   loader_prepare(&env, &r); CHECK(r.status == LOADER_E_MEMORY);   /* kernel is bigger */
    make_env(kernel, ksize, 0); env.scratch_base = 0x100000; env.scratch_size = 0x100000;
    loader_prepare(&env, &r); CHECK(r.status == LOADER_E_MEMORY);              /* not inside the usable region we declared */
    make_env(kernel, ksize, 0); env.files[1].addr = env.files[0].addr + 4096;  loader_prepare(&env, &r); CHECK(r.status == LOADER_E_MEMORY);   /* aliasing files */
    make_env(kernel, ksize, 0); env.files[0].size = 0;                         loader_prepare(&env, &r); CHECK(r.status == LOADER_E_MEMORY);
    make_env(kernel, ksize, 0); arena[0] = 0;                                  loader_prepare(&env, &r); CHECK(r.status == LOADER_E_ELF && r.detail == ELF_E_MAGIC);
    CHECK(loader_strerror(&r)[0] != 0);
    make_env(kernel, ksize, 0); env.files[0].size = ksize / 2;                 loader_prepare(&env, &r); CHECK(r.status == LOADER_E_ELF);
}

int main(int argc, char **argv)
{
    if (argc < 2) { printf("usage: loader_test kernel.elf\n"); return 2; }
    arena = mmap(0, ARENA, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS | MAP_32BIT, -1, 0);
    if (arena == MAP_FAILED || (uint64_t)(uintptr_t)arena + ARENA > 0x100000000ull) { printf("no low arena\n"); return 77; }
    size_t n; uint8_t *kernel = slurp(argv[1], &n);
    test_mmap();
    test_paging();
    test_elf(kernel, n);
    test_pipeline(kernel, n);
    test_rejects(kernel, n);
    if (!fails) printf("loader_test: all passed\n");
    return fails != 0;
}
