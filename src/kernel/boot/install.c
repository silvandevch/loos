/* The LoOS installer backend.
 *
 * Source: the installation media's own files, delivered by the loader as boot modules (BIOS: the whole image as "bootimg",
 * UEFI: "kernel.elf", "efi", "boot.cfg" and any extra modules), so installing works from any media the firmware can boot
 * (USB, CD, disk) - the kernel never has to read the media. Targets: any ATA, AHCI or NVMe disk (blk.c).
 *
 * BIOS install:  MBR | stage 2 | boot table | config | boot files | gap | btrfs data partition (MBR partition entry)
 * UEFI install:  GPT | ESP (FAT32: EFI\BOOT\BOOTX64.EFI, LOOS\KERNEL.ELF, BOOT.CFG, CONFIG.BIN, modules) | btrfs data partition
 * Both: the new account (PBKDF2 hash only) and machine settings go into the config module, everything written is read back
 * and verified, and the old disk contents (partition tables, signatures) are wiped first. */
#include "kernel.h"
#include "installer.h"
#include "session.h"
#include "kdf.h"
#include "diskfmt.h"
#include "btrfs.h"
#include <stdio.h>
#include <string.h>

#define TABLE_LBA 65
#define TABLE_SECTORS 2
#define CONFIG_LBA 67
#define CONFIG_SECTORS 8
#define CONFIG_BYTES (CONFIG_SECTORS * 512)
#define CHUNK 128
#define MIN_DISK_SECTORS (2ull * 1024 * 1024 * 1024 / 512)
#define ESP_SECTORS (300ull * 1024 * 1024 / 512)
#define PBKDF2_MIN_ITERS 10000u
#define PBKDF2_MAX_ITERS 1000000u
#define PBKDF2_TARGET_US 300000u                     /* aim for ~0.3 s per sign-in on this machine */
#define WIPE_HEAD_SECTORS 4096u                      /* 2 MiB at the start and 1 MiB at the end of the disk */
#define WIPE_TAIL_SECTORS 2048u

static const struct loos_bootinfo *g_bi;
void install_set_bootinfo(const struct loos_bootinfo *bi) { g_bi = bi; }

static const struct loos_boot_module *find_module(const char *name)
{
    if (!g_bi) return 0;
    for (uint32_t i = 0; i < g_bi->module_count; i++) if (!strcmp(g_bi->modules[i].name, name)) return &g_bi->modules[i];
    return 0;
}
static int is_uefi(void) { return g_bi && (g_bi->flags & LOOS_BOOT_FLAG_UEFI); }

/* Installed-system configuration text delivered as the "config" module; 0 when this is not an installed system. */
int install_read_config(char *out, size_t cap)
{
    const struct loos_boot_module *m = find_module("config");
    if (!m || m->size < 9) return 0;
    const char *c = (const char *)(uintptr_t)m->phys;
    if (memcmp(c, "LOOSCFG1\n", 9) != 0) return 0;
    size_t n = 0;
    while (n < m->size && c[n]) n++;
    if (n >= cap) n = cap - 1;
    memcpy(out, c, n); out[n] = 0;
    return (int)n;
}

/* ---------------------------------------------------------------------------------------------- helpers */
static uint32_t le32(const uint8_t *p) { return (uint32_t)p[0] | (uint32_t)p[1] << 8 | (uint32_t)p[2] << 16 | (uint32_t)p[3] << 24; }
static void put32(uint8_t *p, uint32_t v) { p[0] = (uint8_t)v; p[1] = (uint8_t)(v >> 8); p[2] = (uint8_t)(v >> 16); p[3] = (uint8_t)(v >> 24); }

static void guid_to_bytes(const char *text, uint8_t out[16])        /* "8-4-4-4-12" hex text -> mixed-endian on-disk GUID */
{
    uint8_t raw[16]; int n = 0, hi = -1;
    for (const char *p = text; *p && n < 16; p++) {
        int v = *p >= '0' && *p <= '9' ? *p - '0' : *p >= 'a' && *p <= 'f' ? *p - 'a' + 10 : *p >= 'A' && *p <= 'F' ? *p - 'A' + 10 : -1;
        if (v < 0) continue;
        if (hi < 0) hi = v; else { raw[n++] = (uint8_t)(hi << 4 | v); hi = -1; }
    }
    static const uint8_t order[16] = { 3, 2, 1, 0, 5, 4, 7, 6, 8, 9, 10, 11, 12, 13, 14, 15 };
    for (int i = 0; i < 16; i++) out[i] = raw[order[i]];
}

/* Drops every boot-time installer token (and any value attached to it) from
 * the command line stored in the installed system. This is what prevents a
 * one-time password from surviving on the installed ESP. */
static void cmdline_strip_installer(char *cmd, size_t cap)
{
    char out[256]; size_t n = 0;
    for (size_t i = 0; cmd[i] && i < cap;) {
        size_t j = i; while (cmd[j] && cmd[j] != ' ' && j < cap) j++;
        size_t len = j - i;
        int installer = (len == 9 && !memcmp(cmd + i, "installer", 9)) || (len > 10 && !memcmp(cmd + i, "installer.", 10));
        if (!installer && len && n + len + 2 < sizeof out) {
            if (n) out[n++] = ' ';
            memcpy(out + n, cmd + i, len); n += len;
        }
        i = j; while (cmd[i] == ' ' && i < cap) i++;
    }
    memset(cmd, 0, cap); memcpy(cmd, out, n);
}

/* ---------------------------------------------------------------------------------------------- which disk is our media? */
static int is_boot_media(struct blkdev *d);
int install_is_boot_media(int disk) { return disk >= 0 && disk < blk_count() && is_boot_media(blk_get(disk)); }
static int is_boot_media(struct blkdev *d)
{
    static uint8_t sec[512 * 2];
    if (!is_uefi()) {                                                       /* BIOS: an installer boot table is the tell-tale */
        if (d->sectors < TABLE_LBA + 2 || d->read(d, TABLE_LBA, 2, sec) != 0 || memcmp(sec, "LOOSBTAB", 8) != 0) return 0;
        char cmd[232]; memcpy(cmd, sec + 16, 232); cmd[231] = 0;
        for (const char *p = cmd; *p; p++) if ((p == cmd || p[-1] == ' ') && !memcmp(p, "installer", 9) && (p[9] == 0 || p[9] == ' ')) return 1;
        return 0;
    }
    const char *tok = 0;                                                    /* UEFI: the loader passes the ESP's partition GUID */
    for (const char *p = g_bi ? g_bi->cmdline : ""; *p; p++) if ((p == g_bi->cmdline || p[-1] == ' ') && !memcmp(p, "loos.espguid=", 13)) { tok = p + 13; break; }
    if (!tok) return 0;
    uint8_t want[16]; guid_to_bytes(tok, want);
    static uint8_t ents[512 * 4];
    if (d->sectors < 64 || d->read(d, 2, 4, ents) != 0) return 0;
    for (int i = 0; i < 16; i++) if (memcmp(ents + 128 * i + 16, want, 16) == 0) return 1;
    return 0;
}

/* ---------------------------------------------------------------------------------------------- backend: disk list */
static int be_list(void *u, inst_disk *out, int max)
{
    (void)u;
    int n = 0;
    for (int i = 0; i < blk_count() && n < max; i++) {
        struct blkdev *d = blk_get(i);
        inst_disk *o = &out[n];
        memset(o, 0, sizeof *o);
        snprintf(o->name, sizeof o->name, "Disk %d (%s)", n + 1, d->name);
        snprintf(o->model, sizeof o->model, "%s", d->model);
        o->bytes = d->sectors * 512;
        o->boot_media = is_boot_media(d);
        o->too_small = d->sectors < MIN_DISK_SECTORS || !d->sector_size_ok;
        n++;                                                                  /* UI index == blk index */
    }
    return n;
}

/* ---------------------------------------------------------------------------------------------- state machine */
enum { PH_ACCOUNT, PH_WIPE, PH_COPY, PH_GPT, PH_ESP_FORMAT, PH_ESP_FILES, PH_ESP_FINISH, PH_BTRFS, PH_VERIFY, PH_DONE, PH_FAILED };
#define MAX_FILES 12

struct esp_file { char name[12]; uint32_t dir_kind; const uint8_t *data; uint64_t size; uint32_t first; };   /* dir_kind: 0 root, 1 EFI\BOOT, 2 LOOS */

static struct {
    int phase, vstage;
    struct blkdev *dst;
    inst_config cfg;
    uint8_t config[CONFIG_BYTES];
    uint64_t pos, end_lba, part_start, part_sectors, disk_sectors;
    /* BIOS */
    const uint8_t *image; uint64_t image_sectors;
    uint8_t table[TABLE_SECTORS * 512];
    /* UEFI */
    struct fat32 fat; uint32_t *fat_table; uint32_t dir_efi_boot, dir_loos;
    struct esp_file files[MAX_FILES]; int nfiles, file_i;
    char cfgtext[2048];
    uint8_t guid_disk[16], guid_esp[16], guid_data[16];
    uint64_t esp_first, data_first, data_last;
    uint8_t chunk[CHUNK * 512], check[CHUNK * 512];
} S;

static int wcb(void *o, uint64_t off, const void *d, size_t n)
{
    (void)o;
    if ((off | n) & 511) return -1;
    return S.dst->write(S.dst, off / 512, (uint32_t)(n / 512), d);
}

static uint32_t calibrate_iterations(void)                                   /* PBKDF2 cost from a short timing run */
{
    uint8_t out[32];
    uint64_t t0 = time_us();
    loos_pbkdf2_sha256("calibration", 11, "0123456789abcdef", 16, 2000, out, sizeof out);
    uint64_t dt = time_us() - t0;
    if (dt == 0) dt = 1;
    uint64_t it = 2000ull * PBKDF2_TARGET_US / dt;
    if (it < PBKDF2_MIN_ITERS) it = PBKDF2_MIN_ITERS;
    if (it > PBKDF2_MAX_ITERS) it = PBKDF2_MAX_ITERS;
    return (uint32_t)(it / 1000 * 1000);
}
static void rng(uint8_t *out, size_t n, void *user) { (void)user; krandom(out, n); }
static void new_uuid(uint8_t u[16]) { krandom(u, 16); u[6] = (uint8_t)((u[6] & 0x0f) | 0x40); u[8] = (uint8_t)((u[8] & 0x3f) | 0x80); }

static int fail(char *err, size_t cap, const char *msg) { snprintf(err, cap, "%s (step %d, sector %u)", msg, S.phase, (unsigned)S.pos); S.phase = PH_FAILED; return -1; }

/* BIOS: the sectors that differ from the media image - LBA 0 (partition entry), the boot table, the config area */
static void bios_patch(uint64_t lba, uint32_t count, uint8_t *buf)
{
    for (uint32_t i = 0; i < count; i++) {
        uint64_t s = lba + i; uint8_t *p = buf + (uint64_t)i * 512;
        if (s == 0) {
            memset(p + 446, 0, 64);
            p[446] = 0x00; p[447] = 0xfe; p[448] = 0xff; p[449] = 0xff; p[450] = 0x83; p[451] = 0xfe; p[452] = 0xff; p[453] = 0xff;
            put32(p + 454, (uint32_t)S.part_start); put32(p + 458, (uint32_t)S.part_sectors);
            p[510] = 0x55; p[511] = 0xaa;
        } else if (s >= TABLE_LBA && s < TABLE_LBA + TABLE_SECTORS) memcpy(p, S.table + (s - TABLE_LBA) * 512, 512);
        else if (s >= CONFIG_LBA && s < CONFIG_LBA + CONFIG_SECTORS) memcpy(p, S.config + (s - CONFIG_LBA) * 512, 512);
    }
}

static int add_file(const char *name, uint32_t dir_kind, const uint8_t *data, uint64_t size)
{
    if (S.nfiles >= MAX_FILES) return -1;
    struct esp_file *f = &S.files[S.nfiles++];
    memset(f, 0, sizeof *f);
    snprintf(f->name, sizeof f->name, "%s", name);
    f->dir_kind = dir_kind; f->data = data; f->size = size;
    return 0;
}

/* BOOT.CFG for the installed system: keep kernel= and cmdline= (without "installer"), keep ordinary modules at their new
 * place, drop the installer-only modules, and always load the config module. */
static void build_installed_cfg(const char *media, char *out, size_t cap)
{
    size_t n = 0;
    out[0] = 0;
    for (const char *p = media; *p;) {
        const char *e = p; while (*e && *e != '\n') e++;
        size_t len = (size_t)(e - p);
        char line[240];
        if (len && p[len - 1] == '\r') len--;
        if (len >= sizeof line) len = sizeof line - 1;
        memcpy(line, p, len); line[len] = 0;
        int keep = 1;
        if (!strncmp(line, "module=", 7)) {
            const char *name = line + 7;
            keep = strncmp(name, "efi:", 4) && strncmp(name, "kernel.elf:", 11) && strncmp(name, "boot.cfg:", 9) && strncmp(name, "config:", 7);
        } else if (!strncmp(line, "cmdline=", 8)) {
            cmdline_strip_installer(line + 8, sizeof line - 8);
        }
        if (keep && line[0]) n += (size_t)snprintf(out + n, cap - n, "%s\n", line);
        p = *e ? e + 1 : e;
    }
    snprintf(out + n, cap - n, "module=config:\\LOOS\\CONFIG.BIN\n");
}

static bool be_begin(void *u, const inst_config *cfg, char *err, size_t cap)
{
    (void)u;
    memset(&S, 0, sizeof S);
    S.cfg = *cfg;
    if (cfg->disk < 0 || cfg->disk >= blk_count()) { snprintf(err, cap, "The selected disk cannot be used."); return false; }
    S.dst = blk_get(cfg->disk);
    if (is_boot_media(S.dst)) { snprintf(err, cap, "That disk holds the installation media."); return false; }
    if (S.dst->sectors < MIN_DISK_SECTORS || !S.dst->sector_size_ok) { snprintf(err, cap, "The disk is too small or uses 4K sectors."); return false; }
    S.disk_sectors = S.dst->sectors;

    if (!is_uefi()) {
        const struct loos_boot_module *m = find_module("bootimg");
        if (!m || m->size < (TABLE_LBA + 2 + CONFIG_SECTORS) * 512ull) { snprintf(err, cap, "This media cannot install: its image was not loaded."); return false; }
        S.image = (const uint8_t *)(uintptr_t)m->phys; S.image_sectors = m->size / 512;
        memcpy(S.table, S.image + (uint64_t)TABLE_LBA * 512, sizeof S.table);
        if (memcmp(S.table, "LOOSBTAB", 8) != 0) { snprintf(err, cap, "The installation image is damaged."); return false; }
        uint32_t nfiles = le32(S.table + 8);
        if (nfiles < 1 || nfiles > 9) { snprintf(err, cap, "The installation image is damaged."); return false; }
        for (uint32_t i = 0; i < nfiles; i++) {                              /* drop the "bootimg" entry from the installed table */
            uint8_t *e = S.table + 248 + 40 * i;
            if (!memcmp(e, "bootimg", 8)) { memmove(e, e + 40, 40 * (nfiles - i - 1)); memset(S.table + 248 + 40 * (nfiles - 1), 0, 40); put32(S.table + 8, --nfiles); break; }
        }
        cmdline_strip_installer((char *)S.table + 16, 232);
        uint64_t end = CONFIG_LBA + CONFIG_SECTORS;
        for (uint32_t i = 0; i < nfiles; i++) {
            const uint8_t *e = S.table + 248 + 40 * i;
            uint64_t lba = le32(e + 16), n = le32(e + 24);
            if (lba + n > end) end = lba + n;
        }
        if (end > S.image_sectors) { snprintf(err, cap, "The installation image is truncated."); return false; }
        S.end_lba = end;
        S.part_start = (end + 2048 + 2047) & ~2047ull;
        if (S.part_start + (LOOS_BTRFS_MIN_DEVICE_BYTES / 512) > S.disk_sectors || S.part_start > 0xffffffffull) { snprintf(err, cap, "The disk is too small for LoOS."); return false; }
        S.part_sectors = S.disk_sectors - S.part_start;
        if (S.part_sectors > 0xffffffffull) S.part_sectors = 0xffffffffull;
        S.data_first = S.part_start; S.data_last = S.part_start + S.part_sectors - 1;
    } else {
        const struct loos_boot_module *k = find_module("kernel.elf"), *efi = find_module("efi"), *cfgm = find_module("boot.cfg");
        if (!k || !efi || !cfgm) { snprintf(err, cap, "This media cannot install: its boot files were not loaded."); return false; }
        char media_cfg[1024]; size_t cl = cfgm->size < sizeof media_cfg - 1 ? (size_t)cfgm->size : sizeof media_cfg - 1;
        memcpy(media_cfg, (const void *)(uintptr_t)cfgm->phys, cl); media_cfg[cl] = 0;
        build_installed_cfg(media_cfg, S.cfgtext, sizeof S.cfgtext);
        add_file("BOOTX64.EFI", 1, (const uint8_t *)(uintptr_t)efi->phys, efi->size);
        add_file("KERNEL.ELF", 2, (const uint8_t *)(uintptr_t)k->phys, k->size);
        add_file("BOOT.CFG", 2, (const uint8_t *)S.cfgtext, strlen(S.cfgtext));
        add_file("CONFIG.BIN", 2, S.config, CONFIG_BYTES);
        for (uint32_t i = 0; i < g_bi->module_count; i++) {                  /* ordinary modules ride along as LOOS\\<NAME>.BIN */
            const struct loos_boot_module *m = &g_bi->modules[i];
            if (!strcmp(m->name, "efi") || !strcmp(m->name, "kernel.elf") || !strcmp(m->name, "boot.cfg") || !strcmp(m->name, "config")) continue;
            char nm[12]; size_t k2 = 0;
            for (const char *c = m->name; *c && k2 < 8; c++) if ((*c >= 'a' && *c <= 'z') || (*c >= 'A' && *c <= 'Z') || (*c >= '0' && *c <= '9')) nm[k2++] = (char)(*c >= 'a' && *c <= 'z' ? *c - 32 : *c);
            memcpy(nm + k2, ".BIN", 5);
            if (add_file(nm, 2, (const uint8_t *)(uintptr_t)m->phys, m->size)) { snprintf(err, cap, "Too many boot modules."); return false; }
        }
        S.esp_first = gpt_first_usable_lba();
        S.data_first = (S.esp_first + ESP_SECTORS + 2048 + 2047) & ~2047ull;   /* >= 1 MiB gap: crash dump area */
        S.data_last = gpt_last_usable_lba(S.disk_sectors);
        if ((S.data_last - S.data_first + 1) * 512 < LOOS_BTRFS_MIN_DEVICE_BYTES) { snprintf(err, cap, "The disk is too small for LoOS."); return false; }
        new_uuid(S.guid_disk); new_uuid(S.guid_esp); new_uuid(S.guid_data);
    }
    S.phase = PH_ACCOUNT;
    return true;
}

struct memsrc { const uint8_t *p; };
static int mem_read(void *o, uint64_t off, void *buf, size_t n) { memcpy(buf, ((struct memsrc *)o)->p + off, n); return 0; }

/* btrfs offsets are relative to the data partition */
static int btrfs_wcb(void *o, uint64_t off, const void *d, size_t n) { (void)o; return wcb(0, S.data_first * 512 + off, d, n); }

static uint32_t dir_for(int kind) { return kind == 1 ? S.dir_efi_boot : kind == 2 ? S.dir_loos : 2; }

static int be_step(void *u, int *percent, char *msg, size_t mcap, char *err, size_t ecap)
{
    (void)u;
    struct blkdev *dst = S.dst;
    switch (S.phase) {
    case PH_ACCOUNT: {                                       /* hash the password; assemble the configuration */
        static shell_accounts db;
        accounts_init(&db, calibrate_iterations());
        db.random = rng;
        if (!account_create(&db, S.cfg.username, S.cfg.display, S.cfg.password, NULL, true)) return fail(err, ecap, "Cannot create the account.");
        memset(S.cfg.password, 0, sizeof S.cfg.password);   /* the plain password is gone from here on */
        char *c = (char *)S.config;
        int n = snprintf(c, sizeof S.config, "LOOSCFG1\nhostname=%s\ndisplay=%s\nnet.hard_off=%d\nupdates.enabled=%d\n[accounts]\n",
                         S.cfg.hostname, S.cfg.display, S.cfg.net_blocked ? 1 : 0, S.cfg.updates ? 1 : 0);
        if (n < 0 || (size_t)n >= sizeof S.config) return fail(err, ecap, "Configuration too large.");
        if (!accounts_save(&db, c + n, sizeof S.config - (size_t)n)) return fail(err, ecap, "Cannot store the account.");
        memset(&db, 0, sizeof db);
        S.pos = 0; S.phase = PH_WIPE;
        *percent = 6; snprintf(msg, mcap, "Erasing the disk");
        return 0;
    }
    case PH_WIPE: {                                          /* 2 MiB at the start, 1 MiB at the end: partition tables and old signatures */
        memset(S.chunk, 0, sizeof S.chunk);
        uint64_t total = WIPE_HEAD_SECTORS + WIPE_TAIL_SECTORS;
        uint64_t lba = S.pos < WIPE_HEAD_SECTORS ? S.pos : S.disk_sectors - WIPE_TAIL_SECTORS + (S.pos - WIPE_HEAD_SECTORS);
        if (dst->write(dst, lba, CHUNK, S.chunk) != 0) return fail(err, ecap, "Write error on the target disk.");
        S.pos += CHUNK;
        *percent = 6 + (int)(S.pos * 6 / total);
        snprintf(msg, mcap, "Erasing the disk");
        if (S.pos >= total) { S.pos = 0; S.phase = is_uefi() ? PH_GPT : PH_COPY; }
        return 0;
    }
    case PH_COPY: {                                          /* BIOS: clone the boot area from the media image, patched */
        uint32_t n = S.end_lba - S.pos > CHUNK ? CHUNK : (uint32_t)(S.end_lba - S.pos);
        memcpy(S.chunk, S.image + S.pos * 512, (size_t)n * 512);
        bios_patch(S.pos, n, S.chunk);
        if (dst->write(dst, S.pos, n, S.chunk) != 0) return fail(err, ecap, "Write error on the target disk.");
        S.pos += n;
        *percent = 12 + (int)(S.pos * 50 / S.end_lba);
        snprintf(msg, mcap, "Copying system files");
        if (S.pos >= S.end_lba) S.phase = PH_BTRFS;
        return 0;
    }
    case PH_GPT: {
        struct gpt_part parts[2];
        memset(parts, 0, sizeof parts);
        memcpy(parts[0].type_guid, GPT_TYPE_ESP, 16); memcpy(parts[0].unique_guid, S.guid_esp, 16);
        parts[0].first_lba = S.esp_first; parts[0].last_lba = S.esp_first + ESP_SECTORS - 1; strcpy(parts[0].name, "EFI System");
        memcpy(parts[1].type_guid, GPT_TYPE_LINUX_DATA, 16); memcpy(parts[1].unique_guid, S.guid_data, 16);
        parts[1].first_lba = S.data_first; parts[1].last_lba = S.data_last; strcpy(parts[1].name, "LoOS data");
        if (gpt_write(S.disk_sectors, S.guid_disk, parts, 2, 0, wcb, 0)) return fail(err, ecap, "Cannot write the partition table.");
        S.phase = PH_ESP_FORMAT; *percent = 14; snprintf(msg, mcap, "Creating the boot partition");
        return 0;
    }
    case PH_ESP_FORMAT: {
        uint32_t entries = fat32_fat_entries(ESP_SECTORS * 512);
        S.fat_table = kalloc((uint64_t)entries * 4, 4096);
        if (!S.fat_table) return fail(err, ecap, "Out of memory.");
        uint32_t vol; krandom((uint8_t *)&vol, 4);
        if (fat32_format(&S.fat, wcb, 0, S.esp_first * 512, ESP_SECTORS * 512, vol, "LOOSESP", rtc_unix(), S.fat_table)) return fail(err, ecap, "Cannot format the boot partition.");
        uint32_t efi = fat32_mkdir(&S.fat, 2, "EFI");
        S.dir_efi_boot = efi ? fat32_mkdir(&S.fat, efi, "BOOT") : 0;
        S.dir_loos = fat32_mkdir(&S.fat, 2, "LOOS");
        if (!S.dir_efi_boot || !S.dir_loos) return fail(err, ecap, "Cannot create the boot directories.");
        S.file_i = 0; S.phase = PH_ESP_FILES; *percent = 18;
        return 0;
    }
    case PH_ESP_FILES: {                                     /* one file per step */
        struct esp_file *f = &S.files[S.file_i];
        struct memsrc src = { f->data };
        if (fat32_write_file(&S.fat, dir_for((int)f->dir_kind), f->name, f->size, mem_read, &src, &f->first)) return fail(err, ecap, "Write error on the boot partition.");
        S.file_i++;
        *percent = 18 + S.file_i * 40 / S.nfiles;
        snprintf(msg, mcap, "Copying system files");
        if (S.file_i >= S.nfiles) S.phase = PH_ESP_FINISH;
        return 0;
    }
    case PH_ESP_FINISH:
        if (fat32_finish(&S.fat)) return fail(err, ecap, "Cannot finish the boot partition.");
        S.phase = PH_BTRFS; *percent = 62;
        return 0;
    case PH_BTRFS: {                                         /* the data partition becomes an empty btrfs */
        struct loos_btrfs_mkfs_params p;
        memset(&p, 0, sizeof p);
        p.device_bytes = (S.data_last - S.data_first + 1) * 512; p.label = "LoOS"; p.unix_time = rtc_unix();
        new_uuid(p.fsid); new_uuid(p.dev_uuid); new_uuid(p.chunk_tree_uuid); new_uuid(p.subvol_uuid);
        if (loos_btrfs_mkfs(&p, btrfs_wcb, 0)) return fail(err, ecap, "Cannot format the data partition.");
        uint8_t mark[512] = "LOOSDUMP-AREA";                  /* the 1 MiB before the data partition holds crash dumps */
        mark[16] = 1; put32(mark + 20, DUMP_AREA_SECTORS);
        if (wcb(0, (S.data_first - DUMP_AREA_SECTORS) * 512, mark, sizeof mark)) return fail(err, ecap, "Cannot write the crash dump area.");
        S.pos = 0; S.vstage = 0; S.phase = PH_VERIFY; *percent = 72; snprintf(msg, mcap, "Verifying the installation");
        return 0;
    }
    case PH_VERIFY: {
        if (S.vstage == 0) {
            if (!is_uefi()) {                                /* BIOS: read the boot area back and compare */
                uint32_t n = S.end_lba - S.pos > CHUNK ? CHUNK : (uint32_t)(S.end_lba - S.pos);
                memcpy(S.chunk, S.image + S.pos * 512, (size_t)n * 512);
                bios_patch(S.pos, n, S.chunk);
                if (dst->read(dst, S.pos, n, S.check) != 0) return fail(err, ecap, "Read error while verifying.");
                if (memcmp(S.chunk, S.check, (size_t)n * 512) != 0) { snprintf(err, ecap, "Verification failed near sector %u.", (unsigned)S.pos); S.phase = PH_FAILED; return -1; }
                S.pos += n;
                *percent = 72 + (int)(S.pos * 24 / S.end_lba);
                if (S.pos >= S.end_lba) S.vstage = 1;
                return 0;
            }
            if ((int)S.pos < S.nfiles) {                     /* UEFI: every file, byte for byte */
                struct esp_file *f = &S.files[S.pos++];
                uint64_t off = fat32_cluster_offset(&S.fat, f->first), done = 0;
                while (done < f->size) {
                    uint32_t n = f->size - done > sizeof S.check ? (uint32_t)sizeof S.check : (uint32_t)(f->size - done);
                    if (dst->read(dst, (off + done) / 512, (n + 511) / 512, S.check) != 0) return fail(err, ecap, "Read error while verifying.");
                    if (memcmp(S.check, f->data + done, n) != 0) { snprintf(err, ecap, "Verification failed in %s.", f->name); S.phase = PH_FAILED; return -1; }
                    done += n;
                }
                *percent = 72 + (int)(S.pos * 20 / (S.nfiles + 1));
                return 0;
            }
            if (dst->read(dst, 1, 1, S.check) != 0 || memcmp(S.check, "EFI PART", 8) != 0) return fail(err, ecap, "The partition table did not verify.");
            S.vstage = 1;
            return 0;
        }
        /* the btrfs superblock must read back with its magic and checksum */
        if (dst->read(dst, S.data_first + 0x10000 / 512, 8, S.check) != 0) return fail(err, ecap, "Read error while verifying.");
        uint32_t c = ~0u;
        for (int i = 32; i < 4096; i++) { c ^= S.check[i]; for (int k = 0; k < 8; k++) c = (c >> 1) ^ (0x82f63b78u & (0u - (c & 1))); }
        if (memcmp(S.check + 64, "_BHRfS_M", 8) != 0 || le32(S.check) != ~c) return fail(err, ecap, "The data partition did not verify.");
        S.phase = PH_DONE; *percent = 100;
        serial_puts("install: ok (verified)\n");
        return 0;
    }
    case PH_DONE: *percent = 100; return 1;
    default: snprintf(err, ecap, "The installer is not running."); return -1;
    }
}

static void be_reboot(void *u)
{
    (void)u;
    outb(0x64, 0xfe);                                        /* keyboard controller reset line */
    for (;;) __asm__ volatile("hlt");
}

static const shell_inst_backend backend = { be_list, be_begin, be_step, be_reboot, 0 };
const shell_inst_backend *install_backend(void) { return &backend; }

/* unattended install for automated tests (installer.auto=N) */
int install_unattended(int disk, const char *user, const char *pass, const char *host)
{
    inst_config cfg;
    memset(&cfg, 0, sizeof cfg);
    cfg.disk = disk; cfg.net_blocked = true;
    snprintf(cfg.username, sizeof cfg.username, "%s", user);
    snprintf(cfg.display, sizeof cfg.display, "%s", user);
    snprintf(cfg.hostname, sizeof cfg.hostname, "%s", host);
    snprintf(cfg.password, sizeof cfg.password, "%s", pass);
    char err[160] = "", msg[96] = ""; int pct = 0;
    if (!be_begin(0, &cfg, err, sizeof err)) { serial_puts("install: failed: "); serial_puts(err); serial_puts("\n"); return -1; }
    for (;;) {
        int rc = be_step(0, &pct, msg, sizeof msg, err, sizeof err);
        if (rc < 0) { serial_puts("install: failed: "); serial_puts(err); serial_puts("\n"); return -1; }
        if (rc > 0) break;
    }
    return 0;
}
