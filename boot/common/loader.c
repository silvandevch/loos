#include "loader.h"
#include "elf.h"
#include "mmap.h"
#include "paging.h"

#define KERNEL_WINDOW_BASE 0xffffffff80000000ull
#define KERNEL_WINDOW_SIZE 0x40000000ull            /* 1 GiB: the top of the address space, reachable with -mcmodel=kernel too */
#define LOW_IDENTITY       0x100000000ull           /* identity-map at least the first 4 GiB */
#define MAX_IDENTITY       0x8000000000ull          /* PML4 slot 0 covers 512 GiB */
#define HUGE               0x200000ull

static inline void *P(uint64_t a) { return (void *)(uintptr_t)a; }

static bool valid_range(uint64_t base, uint64_t length)
{
    return length <= UINT64_MAX - base;
}

static bool str_has_word(const char *s, const char *w)
{
    size_t wl = strlen(w);
    if (wl == 0 || wl > LOOS_BOOT_CMDLINE) return false;
    for (size_t i = 0; i < LOOS_BOOT_CMDLINE && s[i]; i++) {
        bool start = i == 0 || s[i - 1] == ' ';
        if (start && i <= LOOS_BOOT_CMDLINE - wl &&
            memcmp(s + i, w, wl) == 0 &&
            (i + wl == LOOS_BOOT_CMDLINE || s[i + wl] == 0 || s[i + wl] == ' ')) {
            return true;
        }
    }
    return false;
}

static uint64_t random_seed(const struct loader_env *env)
{
    if (env->seed) return env->seed;
#if defined(__x86_64__) && !defined(LOADER_HOST_TEST)
    uint32_t lo, hi;
    __asm__ volatile("rdtsc" : "=a"(lo), "=d"(hi));
    uint64_t v = ((uint64_t)hi << 32) | lo, r = 0;
    uint32_t a = 1, b, c, d;
    __asm__ volatile("cpuid" : "+a"(a), "=b"(b), "=c"(c), "=d"(d));
    if (c & (1u << 30)) {                               /* RDRAND: mix in a few draws when present */
        for (int i = 0; i < 4; i++) {
            uint64_t x; unsigned char ok;
            __asm__ volatile("rdrand %0; setc %1" : "=r"(x), "=qm"(ok));
            if (ok) r ^= x * 0x9e3779b97f4a7c15ull + (uint64_t)i;
        }
    }
    return v * 0x9e3779b97f4a7c15ull ^ r;
#else
    return 0x12345678abcdefull;
#endif
}

static uint8_t sum8(const uint8_t *p, size_t n) { uint8_t s = 0; while (n--) s = (uint8_t)(s + *p++); return s; }

static uint64_t scan_rsdp_range(uint64_t base, uint64_t len)
{
    for (uint64_t a = base; a + 20 <= base + len; a += 16) {
        const uint8_t *p = P(a);
        if (memcmp(p, "RSD PTR ", 8) != 0 || sum8(p, 20) != 0) continue;
        if (p[15] >= 2 && (a + 36 > base + len || sum8(p, 36) != 0)) continue;
        return a;
    }
    return 0;
}

static uint64_t find_rsdp_bios(void)
{
    uint16_t ebda_seg = *(const volatile uint16_t *)P(0x40e);
    if (ebda_seg >= 0x80) {
        uint64_t r = scan_rsdp_range((uint64_t)ebda_seg << 4, 1024);
        if (r) return r;
    }
    return scan_rsdp_range(0xe0000, 0x20000);
}

static bool overlaps_ram(const struct mm *m, uint64_t base, uint64_t len)
{
    if (!valid_range(base, len)) return true;
    for (uint32_t i = 0; i < m->n; i++) {
        uint32_t t = m->e[i].type;
        bool ram = t == LOOS_MEM_USABLE || t == LOOS_MEM_BOOT || t == LOOS_MEM_ACPI_RECLAIM || t == LOOS_MEM_KERNEL || t == LOOS_MEM_MODULE;
        if (ram && m->e[i].base < base + len && base < m->e[i].base + m->e[i].length) return true;
    }
    return false;
}

static bool disjoint(uint64_t a, uint64_t al, uint64_t b, uint64_t bl)
{
    if (!valid_range(a, al) || !valid_range(b, bl)) return false;
    return a + al <= b || b + bl <= a;
}

/* CPUID.1:EDX[16]: the page attribute table exists. Without it the framebuffer stays uncached. */
static bool cpu_has_pat(void)
{
#if defined(__x86_64__) && !defined(LOADER_HOST_TEST)
    uint32_t a = 1, b, c, d;
    __asm__ volatile("cpuid" : "+a"(a), "=b"(b), "=c"(c), "=d"(d));
    return (d >> 16) & 1;
#else
    return true;
#endif
}

static bool map_huge(struct pt *p, uint64_t addr, uint64_t flags)
{
    if (pt_lookup(p, addr, NULL)) return true;            /* already covered */
    return pt_map2m(p, addr, addr, flags);
}

void loader_prepare(struct loader_env *env, struct loader_result *out)
{
    if (out == NULL) return;
    memset(out, 0, sizeof *out);
    out->status = LOADER_E_ENV;
    if (env == NULL) return;
    if (env->magic != LOADER_ENV_MAGIC || env->version != LOADER_ENV_VERSION) return;
    if (env->nfiles < 1 || env->nfiles > LOADER_MAX_FILES || env->nreserved > LOADER_MAX_RESERVED) return;
    if (env->mmap_count == 0 || env->mmap_count > LOADER_MAX_RAW_MMAP || !env->mmap_phys) return;
    if ((env->scratch_base | env->scratch_size) & (PAGE_SIZE - 1) || env->scratch_size < 64 * 1024 ||
        env->scratch_size > LOW_IDENTITY || env->scratch_base > LOW_IDENTITY - env->scratch_size ||
        !valid_range(env->image.base, env->image.length))
        return;
    for (uint32_t i = 0; i < env->nreserved; i++) {
        if (!valid_range(env->reserved[i].base, env->reserved[i].length)) {
            out->status = LOADER_E_MEMORY;
            return;
        }
    }

    static struct mm mm;
    mm_init(&mm);
    const struct loos_mem_entry *raw = P(env->mmap_phys);
    for (uint32_t i = 0; i < env->mmap_count; i++) {
        if (!(raw[i].attr & 1)) continue;                    /* ACPI 3.0: bit 0 clear = ignore */
        uint32_t t = raw[i].type;
        if (t < LOOS_MEM_USABLE || t > LOOS_MEM_BAD) t = LOOS_MEM_RESERVED;
        mm_add(&mm, raw[i].base, raw[i].length, t);
    }
    if (!mm_normalize(&mm)) { out->status = LOADER_E_MMAP; return; }

    /* everything the front end placed in RAM must really be usable RAM, and must not alias */
    if (!mm_covered_by(&mm, env->scratch_base, env->scratch_size, LOOS_MEM_USABLE)) { out->status = LOADER_E_MEMORY; return; }
    for (uint32_t i = 0; i < env->nfiles; i++) {
        const struct loader_file *f = &env->files[i];
        if (f->size == 0 || !valid_range(f->addr, f->size) || !mm_covered_by(&mm, f->addr, f->size, LOOS_MEM_USABLE) ||
            !disjoint(f->addr, f->size, env->scratch_base, env->scratch_size)) {
            out->status = LOADER_E_MEMORY;
            return;
        }
        for (uint32_t j = 0; j < i; j++)
            if (!disjoint(f->addr, f->size, env->files[j].addr, env->files[j].size)) { out->status = LOADER_E_MEMORY; return; }
    }

    /* kernel image */
    struct elf_info ei;
    int rc = elf_parse(P(env->files[0].addr), env->files[0].size, KERNEL_WINDOW_SIZE, &ei);
    if (rc != ELF_OK) { out->status = LOADER_E_ELF; out->detail = rc; return; }

    uint64_t bi_size = ALIGN_UP(sizeof(struct loos_bootinfo) + (uint64_t)MM_CAP * sizeof(struct loos_mem_entry), PAGE_SIZE);
    if (ei.span > env->scratch_size || bi_size > env->scratch_size - ei.span ||
        16 * PAGE_SIZE > env->scratch_size - ei.span - bi_size) {
        out->status = LOADER_E_MEMORY;
        return;
    }
    uint64_t kernel_phys = env->scratch_base + bi_size;
    uint64_t pt_base = kernel_phys + ei.span;

    bool kaslr = !str_has_word(env->cmdline, "nokaslr");
    uint64_t vbase = KERNEL_WINDOW_BASE;
    if (kaslr) {
        uint64_t slots = (KERNEL_WINDOW_SIZE - ei.span) / HUGE;
        if (slots) vbase += (random_seed(env) % slots) * HUGE;
    }
    rc = elf_load(&ei, P(env->files[0].addr), env->files[0].size, P(kernel_phys), vbase);
    if (rc != ELF_OK) { out->status = LOADER_E_ELF; out->detail = rc; return; }

    /* final memory map */
    mm_add(&mm, 0, PAGE_SIZE, LOOS_MEM_RESERVED);
    for (uint32_t i = 0; i < env->nreserved; i++) mm_add(&mm, env->reserved[i].base, env->reserved[i].length, LOOS_MEM_BOOT);
    mm_add(&mm, env->image.base, env->image.length, LOOS_MEM_BOOT);
    mm_add(&mm, kernel_phys, ALIGN_UP(ei.span, PAGE_SIZE), LOOS_MEM_KERNEL);
    for (uint32_t i = 1; i < env->nfiles; i++)
        mm_add(&mm, ALIGN_DOWN(env->files[i].addr, PAGE_SIZE), ALIGN_UP(env->files[i].size + (env->files[i].addr & (PAGE_SIZE - 1)), PAGE_SIZE), LOOS_MEM_MODULE);
    if (!mm_normalize(&mm)) { out->status = LOADER_E_MMAP; return; }

    /* page tables */
    struct pt pt;
    if (!pt_init(&pt, P(pt_base), env->scratch_base + env->scratch_size - pt_base)) { out->status = LOADER_E_PAGING; return; }
    bool wc = cpu_has_pat();                                     /* framebuffer as write-combining: ~10x faster than uncached */
    uint64_t fb_len = env->fb_phys ? (uint64_t)env->fb_pitch * env->fb_height : 0;
    if (!valid_range(env->fb_phys, fb_len)) { out->status = LOADER_E_MEMORY; return; }
    for (uint64_t a = 0; a < LOW_IDENTITY; a += HUGE) {
        uint64_t flags = PT_W | PT_NX;
        if (fb_len && !disjoint(a, HUGE, env->fb_phys, fb_len)) flags |= wc ? PT_PAT2M : PT_UC;
        else if (!overlaps_ram(&mm, a, HUGE)) flags |= PT_UC;
        if (env->image.length && !disjoint(a, HUGE, env->image.base, env->image.length)) flags &= ~PT_NX;
        if (!pt_map2m(&pt, a, a, flags)) { out->status = LOADER_E_PAGING; return; }
    }
    for (uint32_t i = 0; i < mm.n; i++) {                    /* RAM above 4 GiB, best effort within the arena */
        uint32_t t = mm.e[i].type;
        if (t != LOOS_MEM_USABLE && t != LOOS_MEM_BOOT && t != LOOS_MEM_ACPI_RECLAIM && t != LOOS_MEM_KERNEL && t != LOOS_MEM_MODULE) continue;
        uint64_t a = ALIGN_DOWN(mm.e[i].base, HUGE), e = ALIGN_UP(mm.e[i].base + mm.e[i].length, HUGE);
        if (a < LOW_IDENTITY) a = LOW_IDENTITY;
        for (; a < e && a < MAX_IDENTITY; a += HUGE) {
            uint64_t fl = PT_W | PT_NX;
            if (env->image.length && !disjoint(a, HUGE, env->image.base, env->image.length)) fl &= ~PT_NX;   /* firmware may load the loader above 4 GiB */
            if (!map_huge(&pt, a, fl)) break;
        }
    }
    if (env->fb_phys) {
        uint64_t len = (uint64_t)env->fb_pitch * env->fb_height;
        uint64_t a = ALIGN_DOWN(env->fb_phys, HUGE), e = ALIGN_UP(env->fb_phys + len, HUGE);
        for (; a < e && a < MAX_IDENTITY; a += HUGE)
            if (!map_huge(&pt, a, PT_W | PT_NX | (wc ? PT_PAT2M : PT_UC))) { out->status = LOADER_E_PAGING; return; }
    }
    for (uint32_t k = 0; k < ei.nload; k++) {
        const struct elf_seg *s = &ei.load[k];
        uint64_t flags = (s->flags & ELF_PF_W ? PT_W : 0) | (s->flags & ELF_PF_X ? 0 : PT_NX);
        uint64_t a = ALIGN_DOWN(s->vaddr, PAGE_SIZE), e = ALIGN_UP(s->vaddr + s->memsz, PAGE_SIZE);
        for (; a < e; a += PAGE_SIZE)
            if (!pt_map4k(&pt, vbase + a, kernel_phys + a, flags)) { out->status = LOADER_E_PAGING; return; }
    }

    /* only the part of the scratch arena actually used stays reserved; the rest goes back to the kernel as usable RAM */
    mm_add(&mm, env->scratch_base, ALIGN_UP(pt_base + pt.used, PAGE_SIZE) - env->scratch_base, LOOS_MEM_BOOT);
    if (!mm_normalize(&mm)) { out->status = LOADER_E_MMAP; return; }

    /* bootinfo */
    struct loos_bootinfo *bi = P(env->scratch_base);
    memset(bi, 0, sizeof *bi);
    bi->magic = LOOS_BOOT_MAGIC;
    bi->version = LOOS_BOOT_VERSION;
    bi->size = sizeof *bi;
    bi->flags = ((env->flags & LOADER_ENV_UEFI) ? LOOS_BOOT_FLAG_UEFI : 0) | (kaslr ? LOOS_BOOT_FLAG_KASLR : 0);
    struct loos_mem_entry *mmv = (struct loos_mem_entry *)((uint8_t *)bi + sizeof *bi);
    memcpy(mmv, mm.e, (size_t)mm.n * sizeof *mmv);
    bi->mmap_count = mm.n;
    bi->mmap_phys = (uint64_t)(uintptr_t)mmv;
    bi->rsdp_phys = env->rsdp_phys ? env->rsdp_phys : ((env->flags & LOADER_ENV_NO_BIOS_SCAN) ? 0 : find_rsdp_bios());
    bi->kernel_phys = kernel_phys;
    bi->kernel_size = ei.span;
    bi->kernel_virt = vbase;
    bi->cr3 = (uint64_t)(uintptr_t)pt.pml4;
    bi->fb_phys = env->fb_phys;
    bi->fb_pitch = env->fb_pitch; bi->fb_width = env->fb_width; bi->fb_height = env->fb_height;
    bi->fb_bpp = env->fb_bpp; bi->fb_format = env->fb_format;
    bi->module_count = env->nfiles - 1;
    for (uint32_t i = 1; i < env->nfiles; i++) {
        struct loos_boot_module *m = &bi->modules[i - 1];
        memcpy(m->name, env->files[i].name, sizeof m->name);
        m->name[sizeof m->name - 1] = 0;
        m->phys = env->files[i].addr;
        m->size = env->files[i].size;
    }
    memcpy(bi->cmdline, env->cmdline, sizeof bi->cmdline);
    bi->cmdline[sizeof bi->cmdline - 1] = 0;

    out->bootinfo = bi;
    out->cr3 = bi->cr3;
    out->entry_virt = vbase + ei.entry;
    out->status = LOADER_OK;
}

const char *loader_strerror(const struct loader_result *r)
{
    switch (r->status) {
    case LOADER_OK: return "ok";
    case LOADER_E_ENV: return "invalid loader environment";
    case LOADER_E_MEMORY: return "boot files or scratch area are not inside usable RAM";
    case LOADER_E_ELF: return elf_strerror(r->detail);
    case LOADER_E_PAGING: return "page table arena exhausted";
    default: return "unusable memory map";
    }
}

#ifndef LOADER_HOST_TEST
__attribute__((noreturn)) static void enter_kernel(uint64_t cr3, uint64_t entry, uint64_t bootinfo)
{
    __asm__ volatile(
        "cli\n\t"
        "movl $0x277, %%ecx\n\t"                   /* IA32_PAT: entry 4 = write-combining (0x01), the rest as reset defaults */
        "movl $0x00070406, %%eax\n\t"
        "movl $0x00070401, %%edx\n\t"
        "wrmsr\n\t"
        "movl $0xc0000080, %%ecx\n\t"             /* EFER.NXE: the tables use NX */
        "rdmsr\n\t"
        "orl $0x800, %%eax\n\t"
        "wrmsr\n\t"
        "movq %%cr0, %%rax\n\t"
        "orq $0x10000, %%rax\n\t"                 /* CR0.WP: ring 0 honours read-only pages */
        "movq %%rax, %%cr0\n\t"
        "movq %0, %%cr3\n\t"
        "jmp *%1\n\t"
        : : "r"(cr3), "r"(entry), "D"(bootinfo) : "rax", "rcx", "rdx", "memory");
    __builtin_unreachable();
}

int loader_main(struct loader_env *env)
{
    struct loader_result r;
    loader_prepare(env, &r);
    if (r.status != LOADER_OK) return r.status;
    enter_kernel(r.cr3, r.entry_virt, (uint64_t)(uintptr_t)r.bootinfo);
}
#endif
