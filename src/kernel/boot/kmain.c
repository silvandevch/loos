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


/* Rust subsystems (crates/kffi/src/sys.rs) */
extern uint32_t loos_win_boot(const uint8_t *initrd, size_t len);
extern int32_t loos_win_gui(const uint8_t *initrd, size_t len, const char *image, uint32_t *back, int32_t w, int32_t h, const char *keys, const char *open, const char *save);
static struct video *gui_vid;
void gui_present(void) { video_present(gui_vid, 0, 0, gui_vid->w, gui_vid->h); }
static void cmdline_word(const char *v, char *out, size_t cap)    /* copies the value up to the next space */
{
    size_t i = 0; if (v) while (v[i] && v[i] != ' ' && i + 1 < cap) { out[i] = v[i]; i++; }
    out[i] = 0;
}
extern uint32_t loos_smss_selftest(void);
extern uint32_t loos_comp_selftest(void);
extern uint32_t loos_net_selftest(void);
extern int32_t loos_xhci_probe(uint64_t mmio, void *dma_virt, uint64_t dma_phys, uint64_t dma_len, uint32_t *out);
extern int32_t loos_net_attach_e1000(uint64_t mmio, void *dma_virt, uint64_t dma_phys, uint64_t dma_len, uint32_t ip, uint32_t gw);
extern void loos_net_poll(uint64_t now_ms);
extern int32_t loos_net_ping(uint32_t ip, uint16_t seq, int32_t send, uint64_t now_ms);
extern uint32_t loos_gpu_probe(uint64_t mmio);
#define XHCI_ARENA 0x40000u
#define E1000_ARENA 0x11000u
static void report_selftest(const char *name, uint32_t got, uint32_t want)
{
    serial_puts(name); serial_puts(got == want ? ": ok (" : ": FAILED (");
    serial_hex(got); serial_puts(")\n");
}
extern int32_t loos_net_config(uint32_t *out);
static void net_ip(uint32_t ip) { for (int k = 24; k >= 0; k -= 8) { serial_dec((ip >> k) & 255); if (k) serial_puts("."); } }
/* Lists the PCI network controllers as "8086:100f" words, for the browser's error page. */
size_t net_nic_summary(char *out, size_t cap)
{
    static struct pci_dev devs[64];
    int n = pci_scan(devs, 64);
    size_t o = 0;
    static const char hex[] = "0123456789abcdef";
    for (int i = 0; i < n; i++) {
        if (devs[i].class_code != 0x02 || o + 11 >= cap) continue;
        uint16_t v[2] = { devs[i].vendor, devs[i].device };
        for (int k = 0; k < 2; k++) { for (int sh = 12; sh >= 0; sh -= 4) out[o++] = hex[(v[k] >> sh) & 15]; out[o++] = k ? ' ' : ':'; }
    }
    if (o) o--;
    out[o] = 0;
    return o;
}
static void net_gpu_probe(void)
{
    static struct pci_dev devs[64];
    int n = pci_scan(devs, 64);
    for (int i = 0; i < n; i++) {
        uint64_t size = 0, bar;
        if (devs[i].class_code == 0x03 && devs[i].vendor == 0x1234 && devs[i].device == 0x1111) {   /* Bochs / QEMU std VGA */
            bar = pci_bar(&devs[i], 2, &size);
            if (!bar || vm_map_mmio(bar, size ? size : 0x1000) < 0) continue;
            serial_puts("gpu: bochs vga, modes "); serial_dec(loos_gpu_probe(bar)); serial_puts("\n");
        }
        if (devs[i].class_code == 0x02 && devs[i].vendor == 0x8086 && (devs[i].device == 0x100e || devs[i].device == 0x100f || devs[i].device == 0x10d3)) {   /* QEMU 82540EM, VMware 82545EM, e1000e 82574L */
            bar = pci_bar(&devs[i], 0, &size);
            if (!bar || !size || vm_map_mmio(bar, size) < 0) continue;
            pci_enable(&devs[i]);
            void *dma = kalloc(E1000_ARENA, 4096);
            if (!dma) continue;
            memset(dma, 0, E1000_ARENA);
            if (loos_net_attach_e1000(bar, dma, (uint64_t)(uintptr_t)dma, E1000_ARENA, 0, 0) < 0) { serial_puts("net: e1000 init failed\n"); continue; }
            uint32_t cfg[4] = { 0 };
            for (uint32_t t0 = time_ms(); !loos_net_config(cfg) && time_ms() - t0 < 6000;) loos_net_poll(time_ms());   /* DHCP; the main loops keep polling */
            if (!cfg[0]) { serial_puts("net: e1000 up, waiting for DHCP\n"); continue; }
            serial_puts("net: e1000 "); net_ip(cfg[0]); serial_puts("/"); serial_dec(cfg[1]); serial_puts(", gateway "); net_ip(cfg[2]); serial_puts(", dns "); net_ip(cfg[3]);
            int ok = 0;
            for (uint32_t t0 = time_ms(), last = t0 - 1000; cfg[2] && !ok && time_ms() - t0 < 4000;) {   /* link may take a moment: re-ask */
                if (time_ms() - last >= 500) { last = time_ms(); loos_net_ping(cfg[2], 1, 1, time_ms()); }
                loos_net_poll(time_ms()); ok = loos_net_ping(cfg[2], 1, 0, time_ms());
            }
            serial_puts(", ping gateway "); serial_puts(ok == 1 ? "reply\n" : "NO REPLY\n");
        }
    }
}
extern int32_t loos_net_fetch(const char *url, uint8_t *out, size_t cap, size_t *out_len, size_t *raw_len, char *final_url, size_t final_cap);
extern size_t loos_net_last_error(char *out, size_t cap);
static void web_selftest(const char *url)                   /* loos.fetch=<url>: proves DNS + TCP + TLS + HTTP end to end on the serial log */
{
    static uint8_t out[3 * 1024 * 1024];
    static char fin[256], err[256];
    size_t n = 0, raw = 0;
    uint32_t t0 = time_ms();
    int rc = loos_net_fetch(url, out, sizeof out, &n, &raw, fin, sizeof fin);
    serial_puts("web: "); serial_puts(url);
    if (rc == 1) { serial_puts(" -> "); serial_puts(fin); serial_puts(", "); serial_dec(raw); serial_puts(" bytes in "); serial_dec(time_ms() - t0); serial_puts(" ms\n"); }
    else { loos_net_last_error(err, sizeof err); serial_puts(rc == 0 ? " FAILED (no network): " : " FAILED: "); serial_puts(err); serial_puts("\n"); }
}

static void usb_probe(void)
{
    static struct pci_dev devs[64];
    int n = pci_scan(devs, 64);
    for (int i = 0; i < n; i++) {
        if (devs[i].class_code != 0x0c || devs[i].subclass != 0x03 || devs[i].prog_if != 0x30) continue;
        uint64_t size = 0, bar = pci_bar(&devs[i], 0, &size);
        if (!bar || !size || size > (16u << 20)) continue;
        if (vm_map_mmio(bar, size) < 0) { serial_puts("usb: xHCI BAR map failed\n"); continue; }
        pci_enable(&devs[i]);
        void *dma = kalloc(XHCI_ARENA, 4096);
        if (!dma) { serial_puts("usb: no memory for xHCI\n"); continue; }
        memset(dma, 0, XHCI_ARENA);
        uint32_t c[4] = { 0 };
        int32_t r = loos_xhci_probe(bar, dma, (uint64_t)(uintptr_t)dma, XHCI_ARENA, c);
        if (r < 0) { serial_puts("usb: xHCI init failed, code "); serial_dec((uint64_t)(-r)); serial_puts("\n"); continue; }
        serial_puts("usb: xHCI "); serial_hex(devs[i].vendor); serial_puts(":"); serial_hex(devs[i].device);
        serial_puts(", devices "); serial_dec((uint64_t)r); serial_puts(" (keyboards "); serial_dec(c[0]);
        serial_puts(", mice "); serial_dec(c[1]); serial_puts(", storage "); serial_dec(c[2]);
        serial_puts(", sectors "); serial_dec(c[3]); serial_puts(")\n");
    }
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

    trap_init(bi);
    kalloc_init(bi);
    time_init();
    int test_mode = cmdline_get(bi, "loos.test") != 0;
    int safe_mode = cmdline_get(bi, "safemode") != 0;           /* one CPU, no splash, 100% scale */
    install_set_bootinfo(bi);
    int ndisks = blk_init();
    uint32_t crashed = bugcheck_boot_report();
    int scale = cmdline_int(cmdline_get(bi, "scale"));
    struct video vid; int have_video = 0;
    if (bi->fb_phys) {
        serial_puts("framebuffer "); serial_dec(bi->fb_width); serial_puts("x"); serial_dec(bi->fb_height);
        serial_puts("x"); serial_dec(bi->fb_bpp); serial_puts(" at "); serial_hex(bi->fb_phys); serial_puts("\n");
        if (video_init(&vid, bi) < 0) serial_puts("desktop: framebuffer not usable (needs 32 bpp, 640x400..8192x8192, enough RAM)\n");
        else {
            have_video = 1;
            bugcheck_set_video(&vid);
            const char *pick = cmdline_get(bi, "bootmenu.auto");
            if (bootmenu_run(&vid, (struct loos_bootinfo *)bi, cmdline_get(bi, "bootmenu") != 0, test_mode ? 0 : 1500, crashed, pick ? cmdline_int(pick) : -1)) safe_mode = 1;
            if (safe_mode) scale = 100;
            if (!scale) scale = desktop_auto_scale(vid.w, vid.h);
            if (!safe_mode) splash_start(&vid, scale);
            splash_progress(8, "Starting");
        }
    }

    if (safe_mode) serial_puts("boot: SAFE MODE\n");
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

    const char *lin_cmd = cmdline_get(bi, "loos.lin");          /* run a Linux program from the initrd: loos.lin=/bin/prog,arg,... */
    if (lin_cmd && *lin_cmd) {
        if (cmdline_get(bi, "loos.net")) net_gpu_probe();                    /* NIC + DHCP before the program starts */
        const uint8_t *rd = 0; size_t rdlen = 0;
        for (uint32_t i = 0; i < bi->module_count; i++)
            if (!strncmp(bi->modules[i].name, "initrd", 6)) { rd = (const uint8_t *)(uintptr_t)bi->modules[i].phys; rdlen = bi->modules[i].size; }
        loos_lin_boot(rd, rdlen, lin_cmd, have_video ? vid.back : 0, have_video ? vid.w : 0, have_video ? vid.h : 0);
        if (have_video) video_present(&vid, 0, 0, vid.w, vid.h);
        if (cmdline_get(bi, "loos.lin.stop")) {
            serial_puts("lin: gui proof complete\n");
            outb(0xf4, 0x10);
            for (;;) __asm__ volatile("hlt");
        }
    }
    if (cmdline_get(bi, "loos.gui") && have_video) {
        if (cmdline_get(bi, "loos.net")) net_gpu_probe();                    /* bring the e1000 up before the program starts */             /* one Windows GUI program from the winroot initrd on the framebuffer */
        const uint8_t *rd = 0; size_t rdlen = 0;
        for (uint32_t i = 0; i < bi->module_count; i++)
            if (!strncmp(bi->modules[i].name, "winroot", 7)) { rd = (const uint8_t *)(uintptr_t)bi->modules[i].phys; rdlen = bi->modules[i].size; }
        char image[64], keys[256], gopen[128], gsave[128];
        cmdline_word(cmdline_get(bi, "loos.gui"), image, sizeof image);
        cmdline_word(cmdline_get(bi, "loos.gui.keys"), keys, sizeof keys);
        cmdline_word(cmdline_get(bi, "loos.gui.open"), gopen, sizeof gopen);
        cmdline_word(cmdline_get(bi, "loos.gui.save"), gsave, sizeof gsave);
        gui_vid = &vid;
        loos_win_gui(rd, rdlen, image, vid.back, vid.w, vid.h, keys, gopen, gsave);
    }
    if (cmdline_get(bi, "loos.win")) {                          /* session manager + real PE processes from the initrd (Windows/System32 images) */
        const uint8_t *rd = 0; size_t rdlen = 0;
        for (uint32_t i = 0; i < bi->module_count; i++)
            if (!strncmp(bi->modules[i].name, "winroot", 7)) { rd = (const uint8_t *)(uintptr_t)bi->modules[i].phys; rdlen = bi->modules[i].size; }
        uint32_t wr = loos_win_boot(rd, rdlen);
        serial_puts(wr == 0x1f ? "win: smss ok (" : "win: smss FAILED ("); serial_hex(wr); serial_puts(")\n");
    }

    int wx = check_wx(bi);
    k_counter++;                                                 /* the write must succeed on the RW data page */
    if (have_video) splash_progress(38, "Memory protection");

    struct acpi_cpus cpus;
    serial_puts("ACPI: RSDP "); serial_hex(bi->rsdp_phys);
    if (acpi_parse_madt(bi->rsdp_phys, &cpus) == 0) { serial_puts(", MADT lists "); serial_dec(cpus.count); serial_puts(" CPUs\n"); }
    else { serial_puts(", no MADT\n"); cpus.count = 1; }
    if (have_video) splash_progress(52, "Processors");
    uint32_t online = safe_mode ? 1 : smp_start_aps(bi, &cpus);
    serial_puts("SMP: "); serial_dec(online); serial_puts(" CPUs online\n");
    uint64_t sys = loos_kexec_init(online);
    uint64_t a[6] = { 0 }, child = 0;
    int st = loos_kexec_syscall(6, a, &child);                     /* CreateProcess: exercises Ob + Ps */
    serial_puts("executive: System pid "); serial_dec(sys); serial_puts(st == 0 ? ", syscalls ok\n" : ", syscalls FAILED\n");
    (void)wx;
    report_selftest("smss", loos_smss_selftest(), 0xf);
    report_selftest("compositor", loos_comp_selftest(), 0xf);
    report_selftest("net", loos_net_selftest(), 0xf);
    usb_probe();
    net_gpu_probe();
    { const char *fu = cmdline_get(bi, "loos.fetch"); if (fu) { char url[256]; cmdline_word(fu, url, sizeof url); web_selftest(url); } }

    /* ---- disks, installer and login ---- */
    serial_puts("disks: "); serial_dec((uint64_t)ndisks); serial_puts("\n");
    bugcheck_selftest(cmdline_get(bi, "loos.bugcheck"));
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
    if (auto_disk && installer_mode) {                              /* unattended install (automated tests and LoOS Switch) */
        char user[32] = "tester", pass[64] = "correct horse9", host[32] = "loos-test";
        const char *v;
        if ((v = cmdline_get(bi, "installer.user"))) { int n = 0; while (v[n] && v[n] != ' ' && n < 31) { user[n] = v[n]; n++; } user[n] = 0; }
        if ((v = cmdline_get(bi, "installer.pass"))) { int n = 0; while (v[n] && v[n] != ' ' && n < 63) { pass[n] = v[n]; n++; } pass[n] = 0; }
        if ((v = cmdline_get(bi, "installer.host"))) { int n = 0; while (v[n] && v[n] != ' ' && n < 31) { host[n] = v[n]; n++; } host[n] = 0; }
        int disk = cmdline_int(auto_disk);
        /* Optional exact-size selector: never guesses a disk index when the
         * app has a specific target in mind. */
        const char *want_bytes = cmdline_get(bi, "installer.auto.bytes");
        if (want_bytes) {
            uint64_t want = 0; for (const char *p = want_bytes; *p >= '0' && *p <= '9'; p++) want = want * 10 + (uint64_t)(*p - '0');
            int found = -1;
            for (int i = 0; i < ndisks; i++) {
                if (install_is_boot_media(i)) continue;
                if (blk_get(i)->sectors * 512 == want) { if (found >= 0) { found = -1; break; } found = i; }
            }
            disk = found;
            if (found < 0) { serial_puts("install: no unique non-media disk with the requested size\n"); for (;;) __asm__ volatile("hlt"); }
        }
        install_unattended(disk, user, pass, host);
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
