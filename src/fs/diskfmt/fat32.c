#include "diskfmt.h"
#include <string.h>

#define SECT 512u

static void p16(uint8_t *b, size_t o, uint16_t v) { b[o] = (uint8_t)v; b[o + 1] = (uint8_t)(v >> 8); }
static void p32(uint8_t *b, size_t o, uint32_t v) { for (int i = 0; i < 4; i++) b[o + i] = (uint8_t)(v >> (8 * i)); }

/* sectors per cluster by volume size, as in Microsoft's FAT32 table (4 KiB clusters up to 8 GiB) */
static uint32_t spc_for(uint64_t sectors) { return sectors <= 16777216ull ? 8 : sectors <= 33554432ull ? 16 : 32; }

static int geometry(uint64_t bytes, uint32_t *total, uint32_t *spc, uint32_t *reserved, uint32_t *fat_sectors, uint32_t *clusters)
{
    if (bytes < FAT32_MIN_BYTES || (bytes % SECT) != 0) return -22;
    uint64_t sectors = bytes / SECT;
    if (sectors > 0xffffffffull) return -22;
    *total = (uint32_t)sectors; *spc = spc_for(sectors); *reserved = 32;
    /* FAT size: iterate to a fixed point (data clusters depend on the FAT size) */
    uint32_t fs = 1;
    int converged = 0;
    for (int i = 0; i < 8; i++) {
        if (sectors <= (uint64_t)*reserved + 2ull * fs) return -22;
        uint64_t data = sectors - *reserved - 2ull * fs;
        uint64_t cl = data / *spc;
        uint32_t need = (uint32_t)(((cl + 2) * 4 + SECT - 1) / SECT);
        if (need == fs) { converged = 1; break; }
        fs = need;
    }
    if (!converged || sectors <= (uint64_t)*reserved + 2ull * fs) return -22;
    *fat_sectors = fs;
    *clusters = (uint32_t)((sectors - *reserved - 2ull * fs) / *spc);
    if (*clusters < 65525 || *clusters > 0x0ffffff5u) return -22;
    return 0;
}

uint32_t fat32_fat_entries(uint64_t bytes)
{
    uint32_t t, s, r, f, c;
    if (geometry(bytes, &t, &s, &r, &f, &c)) return 0;
    return c + 2;
}

void fat32_dos_time(uint64_t t, uint16_t *date, uint16_t *time)
{
    const uint64_t min_time = 315532800ull;  /* 1980-01-01 00:00:00 UTC */
    const uint64_t max_time = 4354819199ull; /* 2107-12-31 23:59:59 UTC */
    if (!date || !time) return;
    if (t < min_time) t = min_time;
    if (t > max_time) t = max_time;
    uint64_t days = t / 86400, rem = t % 86400;
    int64_t z = (int64_t)days + 719468;                                /* civil from days (Hinnant) */
    int64_t era = (z >= 0 ? z : z - 146096) / 146097;
    unsigned doe = (unsigned)(z - era * 146097), yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    int64_t y = (int64_t)yoe + era * 400;
    unsigned doy = doe - (365 * yoe + yoe / 4 - yoe / 100), mp = (5 * doy + 2) / 153;
    unsigned d = doy - (153 * mp + 2) / 5 + 1, m = mp < 10 ? mp + 3 : mp - 9;
    if (m <= 2) y++;
    if (y < 1980) { y = 1980; m = 1; d = 1; }
    *date = (uint16_t)(((y - 1980) << 9) | (m << 5) | d);
    *time = (uint16_t)(((rem / 3600) << 11) | (((rem / 60) % 60) << 5) | ((rem % 60) / 2));
}

static uint64_t cluster_off(const struct fat32 *fs, uint32_t cl)
{
    return fs->base + ((uint64_t)fs->reserved + 2ull * fs->fat_sectors + (uint64_t)(cl - 2) * fs->sectors_per_cluster) * SECT;
}

int fat32_format(struct fat32 *fs, diskfmt_write_fn write, void *opaque, uint64_t base, uint64_t bytes, uint32_t volume_id, const char *label,
                 uint64_t unix_time, uint32_t *fat)
{
    if (!fs || !write || !fat || base > UINT64_MAX - bytes) return -22;
    memset(fs, 0, sizeof *fs);
    fs->write = write; fs->opaque = opaque; fs->base = base; fs->fat = fat;
    if (geometry(bytes, &fs->sectors_total, &fs->sectors_per_cluster, &fs->reserved, &fs->fat_sectors, &fs->clusters)) return -22;
    fat32_dos_time(unix_time, &fs->dos_date, &fs->dos_time);
    memset(fat, 0, (size_t)(fs->clusters + 2) * 4);
    fs->ndirs = 1; fs->dirs[0].cluster = 2;                              /* the root directory is tracked like any other */
    fat[0] = 0x0ffffff8u; fat[1] = 0x0fffffffu;
    fat[2] = 0x0fffffffu;                                                /* root directory: one cluster, end of chain */
    fs->next_free = 3;
    {                                                                    /* volume label entry first in the root directory */
        uint8_t *e = fs->dirs[0].data;
        memset(e, 0, 32); memset(e, ' ', 11);
        for (int i = 0; i < 11 && label && label[i]; i++) e[i] = (uint8_t)label[i];
        e[11] = 0x08; p16(e, 22, fs->dos_time); p16(e, 24, fs->dos_date);
        fs->dirs[0].used = 32;
    }

    uint8_t b[SECT];
    memset(b, 0, sizeof b);
    b[0] = 0xeb; b[1] = 0x58; b[2] = 0x90;
    memcpy(b + 3, "LOOS 1.0", 8);
    p16(b, 11, SECT); b[13] = (uint8_t)fs->sectors_per_cluster; p16(b, 14, (uint16_t)fs->reserved);
    b[16] = 2; p16(b, 17, 0); p16(b, 19, 0); b[21] = 0xf8; p16(b, 22, 0);
    p16(b, 24, 63); p16(b, 26, 255); p32(b, 28, 0); p32(b, 32, fs->sectors_total);
    p32(b, 36, fs->fat_sectors); p16(b, 40, 0); p16(b, 42, 0); p32(b, 44, 2); p16(b, 48, 1); p16(b, 50, 6);
    b[64] = 0x80; b[66] = 0x29; p32(b, 67, volume_id);
    memset(b + 71, ' ', 11);
    for (int i = 0; i < 11 && label && label[i]; i++) b[71 + i] = (uint8_t)label[i];
    memcpy(b + 82, "FAT32   ", 8);
    b[510] = 0x55; b[511] = 0xaa;
    if (write(opaque, base, b, SECT) || write(opaque, base + 6 * SECT, b, SECT)) return -5;          /* boot sector + backup */

    return 0;
}

static uint32_t alloc_clusters(struct fat32 *fs, uint32_t n)
{
    if (n == 0 || fs->next_free + n > fs->clusters + 2) return 0;
    uint32_t first = fs->next_free;
    for (uint32_t i = 0; i < n; i++) fs->fat[first + i] = i + 1 < n ? first + i + 1 : 0x0fffffffu;
    fs->next_free += n;
    return first;
}

static void release_clusters(struct fat32 *fs, uint32_t first, uint32_t n)
{
    if (!first || !n) return;
    for (uint32_t i = 0; i < n; i++) fs->fat[first + i] = 0;
    fs->next_free = first;
}

/* directories are kept in RAM until fat32_finish(): each is exactly one cluster */
static struct fat32_dir *dir_of(struct fat32 *fs, uint32_t cluster)
{
    if (!fs) return 0;
    for (int i = 0; i < fs->ndirs; i++) if (fs->dirs[i].cluster == cluster) return &fs->dirs[i];
    return 0;
}

static void put_name(uint8_t *e, const char *name83)
{
    memset(e, ' ', 11);
    if (name83[0] == '.' && (name83[1] == 0 || (name83[1] == '.' && name83[2] == 0))) { e[0] = '.'; if (name83[1]) e[1] = '.'; return; }
    if (name83[0] == '=') { for (int i = 0; i < 11 && name83[1 + i]; i++) e[i] = (uint8_t)name83[1 + i]; return; }          /* raw 11-byte name (volume label) */
    int i = 0, k = 0;
    for (; name83[i] && name83[i] != '.' && k < 8; i++) e[k++] = (uint8_t)(name83[i] >= 'a' && name83[i] <= 'z' ? name83[i] - 32 : name83[i]);
    while (name83[i] && name83[i] != '.') i++;
    if (name83[i] == '.') {
        i++; k = 8;
        for (; name83[i] && k < 11; i++) e[k++] = (uint8_t)(name83[i] >= 'a' && name83[i] <= 'z' ? name83[i] - 32 : name83[i]);
    }
}

static int add_entry(struct fat32 *fs, struct fat32_dir *d, const char *name83, uint8_t attr, uint32_t first, uint32_t size)
{
    if (!fs || !d || !name83) return -22;
    uint32_t cbytes = fs->sectors_per_cluster * SECT;
    if (cbytes < 32 || d->used > cbytes - 32) return -28;                  /* the directory cluster is full */
    uint8_t *e = d->data + d->used;
    memset(e, 0, 32);
    put_name(e, name83);
    e[11] = attr;
    p16(e, 14, fs->dos_time); p16(e, 16, fs->dos_date); p16(e, 18, fs->dos_date); p16(e, 22, fs->dos_time); p16(e, 24, fs->dos_date);
    p16(e, 20, (uint16_t)(first >> 16)); p16(e, 26, (uint16_t)first); p32(e, 28, size);
    d->used += 32;
    return 0;
}

uint32_t fat32_mkdir(struct fat32 *fs, uint32_t parent, const char *name83)
{
    if (!fs || !fs->write || !fs->fat || fs->sectors_per_cluster == 0) return 0;
    struct fat32_dir *p = dir_of(fs, parent);
    if (!p || fs->ndirs >= FAT32_MAX_DIRS) return 0;
    uint32_t cbytes = fs->sectors_per_cluster * SECT;
    if (!name83 || p->used > cbytes - 32) return 0;
    uint32_t cl = alloc_clusters(fs, 1);
    if (!cl) return 0;
    if (add_entry(fs, p, name83, 0x10, cl, 0)) { release_clusters(fs, cl, 1); return 0; }
    struct fat32_dir *d = &fs->dirs[fs->ndirs++];
    memset(d, 0, sizeof *d);
    d->cluster = cl;
    add_entry(fs, d, ".", 0x10, cl, 0);
    add_entry(fs, d, "..", 0x10, parent == 2 ? 0 : parent, 0);
    return cl;
}

uint64_t fat32_cluster_offset(const struct fat32 *fs, uint32_t cluster)
{
    if (!fs || cluster < 2 || cluster >= fs->clusters + 2) return 0;
    return cluster_off(fs, cluster);
}

int fat32_write_file(struct fat32 *fs, uint32_t dir, const char *name83, uint64_t size, fat32_read_fn read, void *rd, uint32_t *first_out)
{
    if (!fs || !fs->write || !fs->fat || fs->sectors_per_cluster == 0) return -22;
    if (first_out) *first_out = 0;
    struct fat32_dir *d = dir_of(fs, dir);
    if (!d || !name83 || size > 0xffffffffull || (size && !read)) return -22;
    uint32_t cbytes = fs->sectors_per_cluster * SECT;
    if (cbytes < 32 || d->used > cbytes - 32) return -28;
    uint32_t n = (uint32_t)((size + cbytes - 1) / cbytes), first = 0;
    if (n) { first = alloc_clusters(fs, n); if (!first) return -28; }
    static uint8_t buf[65536];
    uint64_t done = 0, total = (uint64_t)n * cbytes;
    while (done < total) {
        size_t chunk = total - done > sizeof buf ? sizeof buf : (size_t)(total - done);
        size_t have = done < size ? (size - done > chunk ? chunk : (size_t)(size - done)) : 0;
        if (have && read(rd, done, buf, have)) { release_clusters(fs, first, n); return -5; }
        if (have < chunk) memset(buf + have, 0, chunk - have);
        if (fs->write(fs->opaque, cluster_off(fs, first) + done, buf, chunk)) { release_clusters(fs, first, n); return -5; }
        done += chunk;
    }
    int rc = add_entry(fs, d, name83, 0x20, first, (uint32_t)size);
    if (rc) release_clusters(fs, first, n);
    else if (first_out) *first_out = first;
    return rc;
}

int fat32_finish(struct fat32 *fs)
{
    if (!fs || !fs->write || !fs->fat || fs->ndirs < 1 || fs->ndirs > FAT32_MAX_DIRS || fs->next_free > fs->clusters + 2) return -22;
    uint32_t cbytes = fs->sectors_per_cluster * SECT;
    if (cbytes < 32 || cbytes > FAT32_MAX_CLUSTER_BYTES) return -22;
    for (int i = 0; i < fs->ndirs; i++) {                                  /* directory clusters (zero padded) */
        struct fat32_dir *d = &fs->dirs[i];
        if (d->cluster < 2 || d->cluster >= fs->clusters + 2 || d->used > cbytes) return -22;
        memset(d->data + d->used, 0, cbytes - d->used);
        if (fs->write(fs->opaque, cluster_off(fs, d->cluster), d->data, cbytes)) return -5;
    }
    uint32_t fat_bytes = fs->fat_sectors * SECT, entries_bytes = (fs->clusters + 2) * 4;
    static uint8_t pad[512];
    for (int copy = 0; copy < 2; copy++) {
        uint64_t at = fs->base + ((uint64_t)fs->reserved + (uint64_t)copy * fs->fat_sectors) * SECT;
        uint32_t whole = entries_bytes & ~(SECT - 1);
        if (whole && fs->write(fs->opaque, at, fs->fat, whole)) return -5;
        if (entries_bytes > whole) {                                       /* the partial last sector */
            memset(pad, 0, sizeof pad);
            memcpy(pad, (const uint8_t *)fs->fat + whole, entries_bytes - whole);
            if (fs->write(fs->opaque, at + whole, pad, SECT)) return -5;
            whole += SECT;
        }
        memset(pad, 0, sizeof pad);
        for (uint32_t o = whole; o < fat_bytes; o += SECT) if (fs->write(fs->opaque, at + o, pad, SECT)) return -5;
    }
    uint8_t info[SECT];
    memset(info, 0, sizeof info);
    p32(info, 0, 0x41615252u); p32(info, 484, 0x61417272u);
    p32(info, 488, fs->clusters + 2 - fs->next_free); p32(info, 492, fs->next_free);
    info[510] = 0x55; info[511] = 0xaa; p32(info, 508, 0xaa550000u);
    if (fs->write(fs->opaque, fs->base + 1 * SECT, info, SECT) || fs->write(fs->opaque, fs->base + 7 * SECT, info, SECT)) return -5;
    return 0;
}
