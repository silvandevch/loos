/* IDT, kernel traps and the bugcheck (stop) path. Crash dumps are built by the Rust kdump crate (crates/kffi). */
#include "kernel.h"
#include "shell.h"
#include <string.h>

/* matches TrapFrame in crates/kffi */
struct ktrap_frame { uint64_t gpr[16], vector, error, rip, cs, rflags; };

extern const uint64_t ktrap_stubs[32];
extern char kstack_bottom[], kstack_top[], kstack_guard[];
extern uint64_t kgdt_tss[2];
int64_t loos_kbugcheck_dump(uint8_t *out, size_t cap, uint32_t code, const uint64_t p[4], const struct ktrap_frame *f, uint64_t cr2, const uint64_t *frames, size_t n);

static struct { uint16_t lo, sel; uint8_t ist, attr; uint16_t mid; uint32_t hi, zero; } __attribute__((packed)) idt[32];
static struct video *bug_video;
static uint8_t dump_buf[16384];
static int in_bugcheck, test_mode;
static struct { uint32_t r0; uint64_t rsp[3]; uint64_t r1, ist[7], r2; uint16_t r3, iomap; } __attribute__((packed)) tss;
static uint8_t ist_stack[2][8192] __attribute__((aligned(16)));    /* IST1: #DF/#NMI/#MC, IST2: #PF (stack overflow) */
static struct { struct blkdev *d; uint64_t lba; } area;              /* crash dump area, found at boot */
uint64_t kbugcheck_dump_len;                         /* dump_buf holds the last dump (for a later writer) */
const uint8_t *kbugcheck_dump_data = dump_buf;

void trap_init(const struct loos_bootinfo *bi)
{
    tss.ist[0] = (uint64_t)(uintptr_t)(ist_stack[0] + sizeof ist_stack[0]);
    tss.ist[1] = (uint64_t)(uintptr_t)(ist_stack[1] + sizeof ist_stack[1]);
    tss.iomap = sizeof tss;
    uint64_t b = (uint64_t)(uintptr_t)&tss, lim = sizeof tss - 1;
    kgdt_tss[0] = (lim & 0xffff) | (b & 0xffffff) << 16 | 0x89ull << 40 | (lim >> 16 & 0xf) << 48 | (b >> 24 & 0xff) << 56;
    kgdt_tss[1] = b >> 32;
    __asm__ volatile("ltr %w0" : : "r"(0x18));
    for (int i = 0; i < 32; i++) {
        uint64_t h = ktrap_stubs[i];
        idt[i].lo = (uint16_t)h; idt[i].mid = (uint16_t)(h >> 16); idt[i].hi = (uint32_t)(h >> 32);
        idt[i].sel = 0x08; idt[i].attr = 0x8E; idt[i].zero = 0;
        idt[i].ist = (i == 2 || i == 8 || i == 18) ? 1 : i == 14 ? 2 : 0;
    }
    struct { uint16_t lim; uint64_t base; } __attribute__((packed)) p = { sizeof idt - 1, (uint64_t)(uintptr_t)idt };
    __asm__ volatile("lidt %0" : : "m"(p));
    if (vm_unmap_page(bi->cr3, (uint64_t)(uintptr_t)kstack_guard) != 0) serial_puts("trap: no stack guard page\n");
    for (const char *c = bi->cmdline; *c; c++) if ((c == bi->cmdline || c[-1] == ' ') && !strncmp(c, "loos.test", 9)) test_mode = 1;
}

void bugcheck_set_video(struct video *v) { bug_video = v; }

static size_t walk(uint64_t rbp, uint64_t rip, uint64_t *out, size_t max)
{
    uint64_t lo = (uint64_t)(uintptr_t)kstack_bottom, hi = (uint64_t)(uintptr_t)kstack_top;
    size_t n = 0;
    out[n++] = rip;
    while (n < max && rbp >= lo && rbp + 16 <= hi && !(rbp & 7)) {
        uint64_t *fp = (uint64_t *)(uintptr_t)rbp;
        if (!fp[1]) break;
        out[n++] = fp[1];
        if (fp[0] <= rbp) break;                         /* frames must move up the stack */
        rbp = fp[0];
    }
    return n;
}

static void hexs(char *o, uint64_t v) { for (int i = 15; i >= 0; i--) { o[i] = "0123456789ABCDEF"[v & 15]; v >>= 4; } o[16] = 0; }

/* ---- crash dump area on disk (see kernel.h) ---- */
static uint32_t rd32(const uint8_t *p) { return p[0] | p[1] << 8 | p[2] << 16 | (uint32_t)p[3] << 24; }
static uint64_t rd64(const uint8_t *p) { return rd32(p) | (uint64_t)rd32(p + 4) << 32; }
static void wr32(uint8_t *p, uint32_t v) { for (int i = 0; i < 4; i++) p[i] = (uint8_t)(v >> 8 * i); }
static uint8_t sec[512];

static int is_area(struct blkdev *d, uint64_t start)
{
    if (start <= DUMP_AREA_SECTORS || start >= d->sectors || d->read(d, start - DUMP_AREA_SECTORS, 1, sec)) return 0;
    if (memcmp(sec, "LOOSDUMP-AREA", 13) || sec[16] != 1) return 0;
    area.d = d; area.lba = start - DUMP_AREA_SECTORS;
    return 1;
}

/* LoOS data partition from the MBR or GPT; the area is the 1 MiB before it, marked by the installer. */
static void find_area(void)
{
    for (int i = 0; i < blk_count(); i++) {
        struct blkdev *d = blk_get(i);
        if (!d->sector_size_ok || d->read(d, 0, 1, sec) || sec[510] != 0x55 || sec[511] != 0xAA) continue;
        uint32_t starts[4]; int gpt = sec[450] == 0xEE;
        for (int k = 0; k < 4; k++) starts[k] = rd32(sec + 446 + 16 * k + 8);
        if (!gpt) { for (int k = 0; k < 4; k++) if (is_area(d, starts[k])) return; continue; }
        if (d->read(d, 1, 1, sec) || memcmp(sec, "EFI PART", 8)) continue;
        uint64_t elba = rd64(sec + 72); uint32_t n = rd32(sec + 80), esz = rd32(sec + 84);
        if (esz < 128 || esz > 512 || n > 128) continue;
        for (uint32_t k = 0; k < n; k++) {
            if (d->read(d, elba + k * esz / 512, 1, sec)) break;
            uint64_t first = rd64(sec + (k * esz) % 512 + 32);
            if (first && is_area(d, first)) return;
        }
    }
}

uint32_t bugcheck_boot_report(void)
{
    find_area();
    if (!area.d) { serial_puts("crashdump: no dump area (live media)\n"); return 0; }
    serial_puts("crashdump: area on "); serial_puts(area.d->name); serial_puts(" at "); serial_dec(area.lba); serial_puts("\n");
    if (area.d->read(area.d, area.lba, 1, sec)) return 0;
    uint32_t len = rd32(sec + 24), code = rd32(sec + 28);
    if (!len) return 0;
    char name[64]; loos_kbugcheck_name(code, name, sizeof name);
    serial_puts("crashdump: the last session stopped with "); serial_hex(code); serial_puts(" "); serial_puts(name);
    serial_puts(", dump "); serial_dec(len); serial_puts(" bytes kept on disk\n");
    if (len <= sizeof dump_buf && !area.d->read(area.d, area.lba + 1, (len + 511) / 512, dump_buf)) kbugcheck_dump_len = len;
    wr32(sec + 24, 0); area.d->write(area.d, area.lba, 1, sec);   /* report once */
    return code;
}

/* ---- the stop screen ---- */
static void bsod(const char *name, uint32_t code, uint64_t addr, int pct, int restarting)
{
    if (!bug_video) return;
    struct video *v = bug_video;
    shell_fb fb = { v->back, v->w, v->h, v->w, v->w >= 2560 ? 150 : 100 };
    shell_rect s = { 0, 0, fb_lw(&fb), fb_lh(&fb) };
    int x = s.w / 10, y = s.h / 6;
    shell_font face = { &loos_font_regular, 120 }, big = { &loos_font_regular, 26 }, mid = { &loos_font_regular, 18 }, small = { &loos_font_regular, 14 };
    fb_fill(&fb, s, s, 0xFF0A5BC4u);
    fb_text_ex(&fb, s, x, y, ":(", 0xFFFFFFFFu, face);
    fb_text_ex(&fb, s, x, y + 170, "LoOS ran into a problem and has to restart.", 0xFFFFFFFFu, big);
    fb_text_ex(&fb, s, x, y + 206, "We're saving some error info on this computer. Nothing leaves it.", 0xFFFFFFFFu, big);
    char line[96]; int n = 0;
    if (restarting) { memcpy(line, "Restarting...", 14); }
    else { if (pct >= 100) pct = 100; if (pct >= 100) line[n++] = '1'; if (pct >= 10) line[n++] = (char)('0' + pct / 10 % 10); line[n++] = (char)('0' + pct % 10); memcpy(line + n, "% complete", 11); }
    fb_text_ex(&fb, s, x, y + 262, line, 0xFFFFFFFFu, big);
    char h[17]; hexs(h, code);
    memcpy(line, "Stop code: ", 11); n = 11; for (const char *c = name; *c && n < 70; c++) line[n++] = *c;
    memcpy(line + n, " (0x", 4); n += 4; memcpy(line + n, h + 8, 8); n += 8; line[n++] = ')'; line[n] = 0;
    fb_text_ex(&fb, s, x, y + 330, line, 0xFFFFFFFFu, mid);
    hexs(h, addr); memcpy(line, "Faulting address: 0x", 20); memcpy(line + 20, h, 17);
    fb_text_ex(&fb, s, x, y + 358, line, 0xFFFFFFFFu, mid);
    fb_text_ex(&fb, s, x, y + 400, "If this keeps happening, press F8 while LoOS starts and pick Safe mode.", 0xFFDCE8F8u, small);
    video_present(v, 0, 0, v->w, v->h);
}

static void reboot(void)
{
    for (int i = 0; i < 100000 && (inb(0x64) & 2); i++) { }
    outb(0x64, 0xFE);                                     /* keyboard controller reset */
    struct { uint16_t l; uint64_t b; } __attribute__((packed)) z = { 0, 0 };
    __asm__ volatile("lidt %0; int3" : : "m"(z));          /* fallback: triple fault */
}

__attribute__((noreturn)) void kbugcheck(uint32_t code, const uint64_t p[4], const struct ktrap_frame *f, uint64_t cr2)
{
    __asm__ volatile("cli");
    if (in_bugcheck++) { outb(0xf4, 0x13); for (;;) __asm__ volatile("hlt"); }   /* nested: stop now */
    char name[64]; loos_kbugcheck_name(code, name, sizeof name);
    serial_puts("\n*** STOP "); serial_hex(code); serial_puts(" "); serial_puts(name); serial_puts("\n    params");
    for (int i = 0; i < 4; i++) { serial_puts(" "); serial_hex(p[i]); }
    serial_puts("\n");
    uint64_t frames[32]; size_t nf = 0;
    if (f) {
        serial_puts("    rip "); serial_hex(f->rip); serial_puts(" rsp "); serial_hex(f->gpr[4]);
        serial_puts(" err "); serial_hex(f->error); serial_puts("\n");
        nf = walk(f->gpr[5], f->rip, frames, 32);
        for (size_t i = 0; i < nf; i++) { serial_puts("    #"); serial_dec(i); serial_puts(" "); serial_hex(frames[i]); serial_puts("\n"); }
        int64_t n = loos_kbugcheck_dump(dump_buf, sizeof dump_buf, code, p, f, cr2, frames, nf);
        kbugcheck_dump_len = n > 0 ? (uint64_t)n : 0;
        serial_puts("    dump "); serial_dec(kbugcheck_dump_len); serial_puts(" bytes\n");
    }
    uint64_t addr = f ? f->rip : 0;
    bsod(name, code, addr, 0, 0);
    uint32_t len = (uint32_t)kbugcheck_dump_len, secs = (len + 511) / 512, done = 0;
    if (area.d && len && secs < DUMP_AREA_SECTORS) {               /* data first, then the header: a torn write is never "valid" */
        while (done < secs) {
            uint32_t c = secs - done < 8 ? secs - done : 8;
            if (area.d->write(area.d, area.lba + 1 + done, c, dump_buf + done * 512)) break;
            done += c;
            bsod(name, code, addr, (int)(done * 95 / secs), 0);
        }
        if (done == secs && !area.d->read(area.d, area.lba, 1, sec)) {
            wr32(sec + 24, len); wr32(sec + 28, code);
            if (!area.d->write(area.d, area.lba, 1, sec)) serial_puts("    dump saved to disk\n");
        }
    }
    bsod(name, code, addr, 100, 0);
    serial_puts("LoOS kernel: bugcheck\n");
    if (test_mode) { outb(0xf4, 0x13); for (;;) __asm__ volatile("hlt"); }   /* QEMU exit status 39 */
    for (uint32_t t = time_ms(); time_ms() - t < 8000; ) __asm__ volatile("pause");
    bsod(name, code, addr, 100, 1);
    reboot();
    for (;;) __asm__ volatile("hlt");
}

static uint64_t nt_status(uint64_t v)
{
    switch (v) {
    case 0: return 0xC0000094; case 3: return 0x80000003; case 6: return 0xC000001D;
    case 13: case 14: return 0xC0000005; default: return 0xC0000005;
    }
}

void ktrap(struct ktrap_frame *f)
{
    if (f->vector == 3) { serial_puts("trap: breakpoint at "); serial_hex(f->rip); serial_puts("\n"); return; }
    uint64_t cr2 = 0, p[4] = { 0 };
    uint32_t code;
    if (f->vector == 14) {
        __asm__ volatile("mov %%cr2, %0" : "=r"(cr2));
        code = 0x50; p[0] = cr2; p[1] = (f->error >> 1) & 1; p[2] = f->rip; p[3] = f->error;
    } else if (f->vector == 8 || f->vector == 18) {
        code = 0x7F; p[0] = f->vector;
    } else {
        code = 0x1E; p[0] = nt_status(f->vector); p[1] = f->rip; p[2] = f->error;
    }
    kbugcheck(code, p, f, cr2);
}

static volatile int depth_limit = 1 << 30;
static int __attribute__((noinline)) recurse(int n) { volatile char pad[512]; pad[0] = (char)n; return n > depth_limit ? 0 : recurse(n + 1) + pad[0]; }

/* loos.bugcheck=<pf|ud|gp|manual>: deliberate crash for the boot tests */
void bugcheck_selftest(const char *v)
{
    if (!v) return;
    serial_puts("bugcheck: self-test "); serial_puts(v); serial_puts("\n");
    if (!strncmp(v, "pf", 2)) { volatile uint64_t *bad = (volatile uint64_t *)(uintptr_t)0xFFFF800000000000ull; (void)*bad; }
    if (!strncmp(v, "ud", 2)) __asm__ volatile("ud2");
    if (!strncmp(v, "stack", 5)) recurse(1);
    if (!strncmp(v, "gp", 2)) { uint64_t nc = 0x8000000000000000ull; __asm__ volatile("movq (%0), %%rax" : : "r"(nc) : "rax"); }
    uint64_t p[4] = { 0 };
    kbugcheck(0xE2, p, 0, 0);
}
