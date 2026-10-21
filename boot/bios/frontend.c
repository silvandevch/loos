/* BIOS front end (long mode, called from stage2.S): turns what real mode collected into a loader_env and hands over
 * to the shared loader core. */
#include "../common/loader.h"
#include "../common/rt.h"

#define VBEINFO  0x500
#define BOOTDATA 0x600
#define E820     0x1000
#define BTAB     0x2000
#define LOW_END  0x100000000ull

struct bootdata { uint32_t e820_count, drive, nfiles, files_end; };
struct btab_entry { char name[16]; uint64_t lba; uint32_t sectors, bytes; uint64_t dest; };
struct btab { char magic[8]; uint32_t count, reserved; char cmdline[232]; struct btab_entry e[LOADER_MAX_FILES]; };
_Static_assert(sizeof(struct btab_entry) == 40, "boot table entry");
_Static_assert(__builtin_offsetof(struct btab, e) == 248, "boot table header size is fixed by stage2.S");

extern char __loader_end[];
static struct loader_env env;

static inline void outb(uint16_t p, uint8_t v) { __asm__ volatile("outb %0,%1" : : "a"(v), "Nd"(p)); }
static inline uint8_t inb(uint16_t p) { uint8_t v; __asm__ volatile("inb %1,%0" : "=a"(v) : "Nd"(p)); return v; }

static void say(const char *s)
{
    for (; *s; s++) {
        while (!(inb(0x3fd) & 0x20)) {}
        outb(0x3f8, (uint8_t)*s);
    }
}

static void fail(const char *what)
{
    say("LoOS boot: "); say(what); say("\n");
    for (;;) __asm__ volatile("cli; hlt");
}

void bios_main(void)
{
    const struct bootdata *bd = (const struct bootdata *)BOOTDATA;
    const struct btab *tab = (const struct btab *)BTAB;
    const uint8_t *vbe = (const uint8_t *)VBEINFO;
    say("LoOS BIOS loader\n");

    memset(&env, 0, sizeof env);
    env.magic = LOADER_ENV_MAGIC;
    env.version = LOADER_ENV_VERSION;
    env.mmap_phys = E820;
    env.mmap_count = bd->e820_count;

    /* low memory owned by the loader: firmware handoff data, bounce buffer, stack and the boot-time page tables */
    env.reserved[0] = (struct loader_range){ 0x500, 0x2600 - 0x500 };
    env.reserved[1] = (struct loader_range){ 0x30000, 0x8000 };
    env.reserved[2] = (struct loader_range){ 0x40000, 0x77000 - 0x40000 };
    env.nreserved = 3;
    env.image = (struct loader_range){ 0x7e00, (uint64_t)(uintptr_t)__loader_end - 0x7e00 };

    env.nfiles = bd->nfiles;
    for (uint32_t i = 0; i < bd->nfiles && i < LOADER_MAX_FILES; i++) {
        memcpy(env.files[i].name, tab->e[i].name, sizeof env.files[i].name);
        env.files[i].name[sizeof env.files[i].name - 1] = 0;
        env.files[i].addr = tab->e[i].dest;
        env.files[i].size = tab->e[i].bytes;
    }
    memcpy(env.cmdline, tab->cmdline, sizeof tab->cmdline);

    /* scratch arena: the rest of the usable region that holds the boot files, up to 512 MiB and below 4 GiB */
    uint64_t base = ((uint64_t)bd->files_end + 0xffff) & ~0xfffull;
    const struct loos_mem_entry *m = (const struct loos_mem_entry *)E820;
    for (uint32_t i = 0; i < bd->e820_count; i++) {
        if (m[i].type != LOOS_MEM_USABLE || !(m[i].attr & 1) || m[i].base > base || base >= m[i].base + m[i].length) continue;
        uint64_t end = m[i].base + m[i].length;
        if (end > LOW_END) end = LOW_END;
        if (end > base + (512ull << 20)) end = base + (512ull << 20);
        if (end > base) { env.scratch_base = base; env.scratch_size = (end - base) & ~0xfffull; }
        break;
    }
    if (!env.scratch_size) fail("no usable RAM after the boot files");

    uint64_t fb = *(const uint32_t *)(vbe + 0x28);
    if (fb && vbe[0x19] == 32) {
        env.fb_phys = fb;
        env.fb_pitch = *(const uint16_t *)(vbe + 0x10);
        env.fb_width = *(const uint16_t *)(vbe + 0x12);
        env.fb_height = *(const uint16_t *)(vbe + 0x14);
        env.fb_bpp = 32;
        env.fb_format = vbe[0x20] == 0 ? LOOS_FB_RGBX : LOOS_FB_BGRX;   /* red field position 0 -> R in the low byte */
    }

    say("LoOS BIOS loader: jumping to the kernel\n");
    (void)loader_main(&env);                      /* only returns on failure */
    struct loader_result r;
    loader_prepare(&env, &r);
    fail(r.status != LOADER_OK ? loader_strerror(&r) : "kernel entry returned");
}
