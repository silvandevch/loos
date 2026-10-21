/* Stage-0 kernel: exercises everything the loader promises (bootinfo, memory map, KASLR'd higher-half image with
 * W^X, modules, ACPI, framebuffer) and brings up the other CPUs. Exits QEMU through isa-debug-exit when done. */
#include "kernel.h"
#include <string.h>

extern const unsigned char k_blob[];                 /* a multi-megabyte read-only blob: proves large kernels load */
extern const unsigned long k_blob_size;
static unsigned long k_counter = 1;                   /* a writable object for the W^X check */
static const char k_rodata_probe[] = "rodata";

static uint32_t crc32(const uint8_t *p, uint64_t n)
{
    uint32_t c = 0xffffffffu;
    while (n--) {
        c ^= *p++;
        for (int i = 0; i < 8; i++) c = (c >> 1) ^ (0xedb88320u & (0u - (c & 1)));
    }
    return ~c;
}

void kmain(const struct loos_bootinfo *bi);
static int check_wx(const struct loos_bootinfo *bi)
{
    uint64_t ps;
    uint64_t t = vm_pte(bi->cr3, (uint64_t)(uintptr_t)&kmain, &ps);
    uint64_t r = vm_pte(bi->cr3, (uint64_t)(uintptr_t)k_rodata_probe, &ps);
    uint64_t d = vm_pte(bi->cr3, (uint64_t)(uintptr_t)&k_counter, &ps);
    int text_ok = t && !(t & (1ull << 63)) && !(t & 2);
    int ro_ok = r && (r & (1ull << 63)) && !(r & 2);
    int data_ok = d && (d & (1ull << 63)) && (d & 2);
    serial_puts("W^X: text "); serial_puts(text_ok ? "r-x" : "BAD");
    serial_puts(", rodata "); serial_puts(ro_ok ? "r--" : "BAD");
    serial_puts(", data "); serial_puts(data_ok ? "rw-" : "BAD");
    serial_puts(text_ok && ro_ok && data_ok ? " ok\n" : " FAILED\n");
    return text_ok && ro_ok && data_ok;
}

static void die(const char *msg)
{
    serial_puts("LoOS kernel: "); serial_puts(msg); serial_puts("\n");
    outb(0xf4, 0x11);
    for (;;) __asm__ volatile("hlt");
}

/* "name" or "name=value" token in the command line; returns a pointer to the value (or "") or NULL */
static const char *cmdline_get(const struct loos_bootinfo *bi, const char *name)
{
    size_t n = 0; while (name[n]) n++;
    for (const char *p = bi->cmdline; *p; p++) {
        if (p != bi->cmdline && p[-1] != ' ') continue;
        size_t i = 0; while (i < n && p[i] == name[i]) i++;
        if (i == n && (p[n] == 0 || p[n] == ' ' || p[n] == '=')) return p[n] == '=' ? p + n + 1 : p + n;
    }
    return 0;
}
static int cmdline_int(const char *v) { int r = 0; while (v && *v >= '0' && *v <= '9') r = r * 10 + (*v++ - '0'); return r; }

int desktop_auto_scale(int w, int h);

void kmain(const struct loos_bootinfo *bi)
{
    serial_puts("LoOS kernel: entered at "); serial_hex((uint64_t)(uintptr_t)&kmain); serial_puts("\n");
    if (bi->magic != LOOS_BOOT_MAGIC || bi->version != LOOS_BOOT_VERSION || bi->size != sizeof *bi) die("BAD BOOTINFO");
    if ((uint64_t)(uintptr_t)&kmain < bi->kernel_virt || (uint64_t)(uintptr_t)&kmain >= bi->kernel_virt + bi->kernel_size)
        die("entry is outside the reported kernel image");

    kalloc_init(bi);
    time_init();
    int test_mode = cmdline_get(bi, "loos.test") != 0;
    int scale = cmdline_int(cmdline_get(bi, "scale"));
    struct video vid; int have_video = 0;
    if (bi->fb_phys) {
        serial_puts("framebuffer "); serial_dec(bi->fb_width); serial_puts("x"); serial_dec(bi->fb_height);
        serial_puts("x"); serial_dec(bi->fb_bpp); serial_puts(" at "); serial_hex(bi->fb_phys); serial_puts("\n");
        if (video_init(&vid, bi) < 0) serial_puts("desktop: framebuffer not usable (needs 32 bpp, 640x400..8192x8192, enough RAM)\n");
        else {
            have_video = 1;
            if (!scale) scale = desktop_auto_scale(vid.w, vid.h);
            splash_start(&vid, scale);
            splash_progress(8, "Starting");
        }
    }

    serial_puts("boot: "); serial_puts(bi->flags & LOOS_BOOT_FLAG_UEFI ? "UEFI" : "BIOS");
    serial_puts(", KASLR "); serial_puts(bi->flags & LOOS_BOOT_FLAG_KASLR ? "on" : "off");
    serial_puts(", kernel phys "); serial_hex(bi->kernel_phys); serial_puts(" virt "); serial_hex(bi->kernel_virt);
    serial_puts(" size "); serial_dec(bi->kernel_size); serial_puts("\n");
    serial_puts("blob: "); serial_dec(k_blob_size); serial_puts(" bytes, crc32 ");
    serial_hex(crc32(k_blob, k_blob_size)); serial_puts("\n");

    const struct loos_mem_entry *e = (const struct loos_mem_entry *)(uintptr_t)bi->mmap_phys;
    uint64_t usable = 0, last_end = 0; int sorted = 1;
    for (uint32_t i = 0; i < bi->mmap_count; i++) {
        serial_puts("  mem "); serial_hex(e[i].base); serial_puts(" len "); serial_hex(e[i].length);
        serial_puts(" type "); serial_dec(e[i].type); serial_puts("\n");
        if (e[i].base < last_end) sorted = 0;
        last_end = e[i].base + e[i].length;
        if (e[i].type == LOOS_MEM_USABLE) usable += e[i].length;
    }
    if (!sorted) die("memory map is not sorted/disjoint");
    serial_puts("memory: "); serial_dec(bi->mmap_count); serial_puts(" ranges, usable "); serial_dec(usable >> 20); serial_puts(" MiB\n");
    if (have_video) splash_progress(22, "Memory");

    for (uint32_t i = 0; i < bi->module_count; i++) {
        const struct loos_boot_module *m = &bi->modules[i];
        serial_puts("module: "); serial_puts(m->name); serial_puts(" "); serial_dec(m->size); serial_puts(" bytes, crc32 ");
        serial_hex(crc32((const uint8_t *)(uintptr_t)m->phys, m->size)); serial_puts("\n");
    }
    if (bi->cmdline[0]) { serial_puts("cmdline: "); serial_puts(bi->cmdline); serial_puts("\n"); }

    int wx = check_wx(bi);
    k_counter++;                                                 /* the write must succeed on the RW data page */
    if (have_video) splash_progress(38, "Memory protection");

    struct acpi_cpus cpus;
    serial_puts("ACPI: RSDP "); serial_hex(bi->rsdp_phys);
    if (acpi_parse_madt(bi->rsdp_phys, &cpus) == 0) { serial_puts(", MADT lists "); serial_dec(cpus.count); serial_puts(" CPUs\n"); }
    else { serial_puts(", no MADT\n"); cpus.count = 1; }
    if (have_video) splash_progress(52, "Processors");
    uint32_t online = smp_start_aps(bi, &cpus);
    serial_puts("SMP: "); serial_dec(online); serial_puts(" CPUs online\n");
    (void)wx;

    /* ---- disks, installer and login ---- */
    install_set_bootinfo(bi);
    int ndisks = blk_init();
    serial_puts("disks: "); serial_dec((uint64_t)ndisks); serial_puts("\n");
    for (int i = 0; i < ndisks; i++) {
        serial_puts("  "); serial_puts(blk_get(i)->name); serial_puts(": "); serial_puts(blk_get(i)->model);
        serial_puts(", "); serial_dec(blk_get(i)->sectors >> 11); serial_puts(" MiB\n");
    }
    static char cfg_text[4096];
    int cfg_len = install_read_config(cfg_text, sizeof cfg_text);
    const char *accounts = 0;
    for (int i = 0; i + 11 <= cfg_len; i++) if (!memcmp(cfg_text + i, "[accounts]\n", 11)) { accounts = cfg_text + i + 11; break; }
    int installer_mode = cmdline_get(bi, "installer") != 0;
    serial_puts("install: media "); serial_puts(installer_mode ? "is installer" : accounts ? "is an installed system" : "is a live system"); serial_puts("\n");

    const char *auto_disk = cmdline_get(bi, "installer.auto");
    if (auto_disk && installer_mode) {                              /* unattended install (automated tests) */
        char user[32] = "tester", pass[64] = "correct horse9", host[32] = "loos-test";
        const char *v;
        if ((v = cmdline_get(bi, "installer.user"))) { int n = 0; while (v[n] && v[n] != ' ' && n < 31) { user[n] = v[n]; n++; } user[n] = 0; }
        if ((v = cmdline_get(bi, "installer.pass"))) { int n = 0; while (v[n] && v[n] != ' ' && n < 63) { pass[n] = v[n]; n++; } pass[n] = 0; }
        install_unattended(cmdline_int(auto_disk), user, pass, host);
        serial_puts("LoOS kernel: stage 0 done\n");
        outb(0xf4, 0x10);
        for (;;) __asm__ volatile("hlt");
    }

    int rc = -1;
    if (have_video) {
        splash_progress(78, "Starting the desktop");
        splash_progress(100, "Ready");
        uint32_t *saved = test_mode ? 0 : kalloc((uint64_t)vid.w * (uint64_t)vid.h * 4, 4096);
        if (saved) { for (uint64_t i = 0; i < (uint64_t)vid.w * (uint64_t)vid.h; i++) saved[i] = vid.back[i]; }
        int go_desktop = 1;
        if (installer_mode) {
            rc = shellhost_installer(&vid, scale, test_mode);
            if (saved && rc >= 0) video_fade(&vid, saved, 400);
            if (rc == 0) go_desktop = 0;                           /* test mode: UI rendered, done */
            if (rc == 1) {                                         /* "Try LoOS without installing": fall through to the live desktop */
                for (uint64_t i = 0; saved && i < (uint64_t)vid.w * (uint64_t)vid.h; i++) saved[i] = vid.back[i];
            }
        } else if (accounts) {
            char user[32] = "";
            rc = shellhost_login(&vid, scale, accounts, user, sizeof user, test_mode);
            if (rc == 0 && test_mode) {                            /* self-test of the stored account: wrong password refused, right one accepted */
                const char *lt = cmdline_get(bi, "login.test");
                if (lt) {
                    char u[32], p[64]; int n = 0;
                    while (lt[n] && lt[n] != ':' && n < 31) { u[n] = lt[n]; n++; } u[n] = 0;
                    const char *pw = lt[n] == ':' ? lt + n + 1 : lt; int m = 0;
                    while (pw[m] && pw[m] != ' ' && m < 63) { p[m] = pw[m]; m++; } p[m] = 0;
                    shellhost_login_selftest(u, p);
                }
                go_desktop = 0;
            } else if (rc == 0) {
                serial_puts("login: signed in as "); serial_puts(user); serial_puts("\n");
                if (saved) video_fade(&vid, saved, 300);
                for (uint64_t i = 0; saved && i < (uint64_t)vid.w * (uint64_t)vid.h; i++) saved[i] = vid.back[i];
                shellhost_set_config(cfg_text);
            }
        } else if (cfg_len) shellhost_set_config(cfg_text);
        if (go_desktop) {
            rc = shellhost_run(bi, &vid, test_mode, scale);
            if (rc == 1) {
                if (saved) video_fade(&vid, saved, 500);
                else video_present(&vid, 0, 0, vid.w, vid.h);
                serial_puts("LoOS kernel: desktop up\n");
                shellhost_loop(&vid);
            }
        }
        if (rc < 0) serial_puts("desktop: failed to start\n");
        else video_present(&vid, 0, 0, vid.w, vid.h);
    }
    serial_puts("LoOS kernel: stage 0 done\n");
    outb(0xf4, 0x10);                                            /* QEMU isa-debug-exit; harmless elsewhere */
    for (;;) __asm__ volatile("hlt");
}
