/* btrfs formatter: writes the smallest valid single-device filesystem (what `mkfs.btrfs -d single -m single` produces for a
 * fresh disk, minus the optional uuid tree and free-space tree) so the installed system's data partition is a real btrfs that
 * Linux's `btrfs check` accepts. Layout, identical physical and logical addresses:
 *   0x10000 superblock | 0x100000 SYSTEM chunk (4 MiB: chunk tree) | 0x500000 METADATA chunk (32 MiB: extent, dev, fs, root,
 *   csum, data-reloc trees) | 64 MiB superblock mirror | 72 MiB DATA chunk (64 MiB, clipped to the device). */
#include "btrfs.h"
#include <string.h>

#define NODE 16384u
#define SECTOR 4096u
#define HDR 101u

#define SUPER_OFF 0x10000ull
#define MIRROR_OFF 0x4000000ull
#define SYS_AT 0x100000ull
#define SYS_LEN 0x400000ull
#define META_AT 0x500000ull
#define META_LEN 0x2000000ull
#define DATA_AT 0x4800000ull
#define DATA_LEN 0x4000000ull

enum { OBJ_ROOT = 1, OBJ_EXTENT = 2, OBJ_CHUNK = 3, OBJ_DEV = 4, OBJ_FS = 5, OBJ_ROOT_DIR = 6, OBJ_CSUM = 7, OBJ_DATA_RELOC = 0 };
#define DATA_RELOC_ID 0xfffffffffffffff7ull
enum { K_INODE_ITEM = 1, K_INODE_REF = 12, K_DIR_ITEM = 84, K_ROOT_ITEM = 132, K_METADATA_ITEM = 169, K_TREE_BLOCK_REF = 176,
       K_BLOCK_GROUP = 192, K_DEV_EXTENT = 204, K_DEV_ITEM = 216, K_CHUNK_ITEM = 228 };
#define BG_DATA 1ull
#define BG_SYSTEM 2ull
#define BG_METADATA 4ull

static void p16(uint8_t *b, size_t o, uint16_t v) { b[o] = (uint8_t)v; b[o + 1] = (uint8_t)(v >> 8); }
static void p32(uint8_t *b, size_t o, uint32_t v) { for (int i = 0; i < 4; i++) b[o + i] = (uint8_t)(v >> (8 * i)); }
static void p64(uint8_t *b, size_t o, uint64_t v) { for (int i = 0; i < 8; i++) b[o + i] = (uint8_t)(v >> (8 * i)); }

static uint32_t crc32c_raw(uint32_t crc, const uint8_t *d, size_t n)
{
    while (n--) {
        crc ^= *d++;
        for (int i = 0; i < 8; i++) crc = (crc >> 1) ^ (0x82f63b78u & (0u - (crc & 1)));
    }
    return crc;
}

struct ctx {
    const struct loos_btrfs_mkfs_params *p;
    loos_btrfs_write_fn write;
    void *opaque;
};

/* ------------------------------------------------------------------------------------------------ leaf builder */
struct leaf {
    uint8_t b[NODE];
    uint32_t n, data_end;
    uint64_t bytenr, owner;
};

static void leaf_init(struct leaf *l, const struct ctx *c, uint64_t bytenr, uint64_t owner)
{
    memset(l, 0, sizeof *l);
    l->data_end = NODE; l->bytenr = bytenr; l->owner = owner;
    memcpy(l->b + 32, c->p->fsid, 16);
    p64(l->b, 48, bytenr);
    p64(l->b, 56, 1ull | (1ull << 56));                       /* WRITTEN | MIXED_BACKREF revision */
    memcpy(l->b + 64, c->p->chunk_tree_uuid, 16);
    p64(l->b, 80, 1);                                         /* generation */
    p64(l->b, 88, owner);
}

/* items must be added in ascending (objectid, type, offset) order */
static int leaf_add(struct leaf *l, uint64_t obj, uint8_t type, uint64_t off, const void *data, uint32_t size)
{
    uint32_t item_at = HDR + l->n * 25;
    if (l->data_end < size || l->data_end - size < item_at + 25) return -1;
    l->data_end -= size;
    if (size) memcpy(l->b + l->data_end, data, size);
    p64(l->b, item_at, obj); l->b[item_at + 8] = type; p64(l->b, item_at + 9, off);
    p32(l->b, item_at + 17, l->data_end - HDR); p32(l->b, item_at + 21, size);
    l->n++;
    return 0;
}

static int leaf_write(struct leaf *l, const struct ctx *c)
{
    p32(l->b, 96, l->n);
    p32(l->b, 0, ~crc32c_raw(0xffffffffu, l->b + 32, NODE - 32));
    return c->write(c->opaque, l->bytenr, l->b, NODE);
}

/* ------------------------------------------------------------------------------------------------ item payloads */
static void inode_item(uint8_t *d, uint64_t gen, uint32_t mode, uint64_t nbytes, uint64_t t)
{
    memset(d, 0, 160);
    p64(d, 0, gen); p64(d, 8, 0); p64(d, 16, 0); p64(d, 24, nbytes);
    p32(d, 40, 1); p32(d, 52, mode);
    for (int off = 112; off <= 148; off += 12) { p64(d, (size_t)off, t); }          /* atime, ctime, mtime, otime */
}

static void root_item(uint8_t *d, uint64_t bytenr, uint64_t rootdir, uint64_t t, const uint8_t *uuid)
{
    memset(d, 0, 439);
    inode_item(d, 1, 040755, NODE, t);
    p64(d, 160, 1);                                           /* generation */
    p64(d, 168, rootdir);
    p64(d, 176, bytenr);
    p64(d, 192, NODE);                                        /* bytes_used */
    p32(d, 216, 1);                                           /* refs */
    d[238] = 0;                                               /* level */
    p64(d, 239, 1);                                           /* generation_v2 */
    if (uuid) { memcpy(d + 247, uuid, 16); p64(d, 295, 0); p64(d, 303, 0); p64(d, 327, t); p64(d, 339, t); }
}

static uint32_t chunk_item(uint8_t *d, const struct ctx *c, uint64_t length, uint64_t type, uint64_t physical)
{
    memset(d, 0, 80);
    p64(d, 0, length); p64(d, 8, 2); p64(d, 16, 65536); p64(d, 24, type);
    p32(d, 32, 65536); p32(d, 36, 65536); p32(d, 40, 4096);
    p16(d, 44, 1); p16(d, 46, 1);
    p64(d, 48, 1); p64(d, 56, physical); memcpy(d + 64, c->p->dev_uuid, 16);
    return 80;
}

static void dev_item(uint8_t *d, const struct ctx *c, uint64_t total, uint64_t used)
{
    memset(d, 0, 98);
    p64(d, 0, 1); p64(d, 8, total); p64(d, 16, used);
    p32(d, 24, 4096); p32(d, 28, 4096); p32(d, 32, 4096);
    memcpy(d + 66, c->p->dev_uuid, 16); memcpy(d + 82, c->p->fsid, 16);
}

static void extent_item(uint8_t *d, uint64_t owner)
{
    memset(d, 0, 33);
    p64(d, 0, 1); p64(d, 8, 1); p64(d, 16, 2);                /* refs, generation, flags = TREE_BLOCK */
    d[24] = K_TREE_BLOCK_REF; p64(d, 25, owner);
}

static void block_group(uint8_t *d, uint64_t used, uint64_t flags) { memset(d, 0, 24); p64(d, 0, used); p64(d, 8, 256); p64(d, 16, flags); }

static void dev_extent(uint8_t *d, const struct ctx *c, uint64_t chunk_off, uint64_t length)
{
    memset(d, 0, 48);
    p64(d, 0, OBJ_CHUNK); p64(d, 8, 256); p64(d, 16, chunk_off); p64(d, 24, length); memcpy(d + 32, c->p->chunk_tree_uuid, 16);
}

static uint32_t dir_item_default(uint8_t *d)
{
    memset(d, 0, 37);
    p64(d, 0, OBJ_FS); d[8] = K_ROOT_ITEM; p64(d, 9, ~0ull);   /* location key: FS_TREE ROOT_ITEM -1 */
    p64(d, 17, 1);                                            /* transid */
    p16(d, 25, 0); p16(d, 27, 7); d[29] = 2;                  /* data_len, name_len, FT_DIR */
    memcpy(d + 30, "default", 7);
    return 37;
}

static uint32_t inode_ref(uint8_t *d, const char *name)
{
    uint32_t n = (uint32_t)strlen(name);
    p64(d, 0, 0); p16(d, 8, (uint16_t)n); memcpy(d + 10, name, n);
    return 10 + n;
}

/* ------------------------------------------------------------------------------------------------ the filesystem */
int loos_btrfs_mkfs(const struct loos_btrfs_mkfs_params *p, loos_btrfs_write_fn write, void *opaque)
{
    if (!p || !write || p->device_bytes < LOOS_BTRFS_MIN_DEVICE_BYTES) return -22;
    struct ctx c = { p, write, opaque };
    uint64_t total = p->device_bytes & ~(uint64_t)(SECTOR - 1);
    uint64_t data_len = total - DATA_AT - (1u << 20);          /* leave the tail alone */
    if (data_len > DATA_LEN) data_len = DATA_LEN;
    data_len &= ~((1ull << 20) - 1);
    uint64_t t = p->unix_time;

    /* tree block addresses */
    const uint64_t a_chunk = SYS_AT, a_extent = META_AT, a_dev = META_AT + NODE, a_fs = META_AT + 2 * NODE, a_root = META_AT + 3 * NODE,
                   a_csum = META_AT + 4 * NODE, a_reloc = META_AT + 5 * NODE;
    struct leaf L;
    uint8_t v[512];
    int rc = 0;

    /* chunk tree */
    uint64_t used_dev = SYS_LEN + META_LEN + data_len;
    leaf_init(&L, &c, a_chunk, OBJ_CHUNK);
    dev_item(v, &c, total, used_dev);                                                   rc |= leaf_add(&L, 1, K_DEV_ITEM, 1, v, 98);
    chunk_item(v, &c, SYS_LEN, BG_SYSTEM, SYS_AT);                                      rc |= leaf_add(&L, 256, K_CHUNK_ITEM, SYS_AT, v, 80);
    chunk_item(v, &c, META_LEN, BG_METADATA, META_AT);                                  rc |= leaf_add(&L, 256, K_CHUNK_ITEM, META_AT, v, 80);
    chunk_item(v, &c, data_len, BG_DATA, DATA_AT);                                      rc |= leaf_add(&L, 256, K_CHUNK_ITEM, DATA_AT, v, 80);
    if (rc || leaf_write(&L, &c)) return -5;
    uint8_t sys_array[17 + 80];                                                         /* key + the SYSTEM chunk, for the superblock */
    p64(sys_array, 0, 256); sys_array[8] = K_CHUNK_ITEM; p64(sys_array, 9, SYS_AT);
    chunk_item(sys_array + 17, &c, SYS_LEN, BG_SYSTEM, SYS_AT);

    /* extent tree */
    leaf_init(&L, &c, a_extent, OBJ_EXTENT);
    block_group(v, 0, 0); /* placeholder to keep the order below readable */
    extent_item(v, OBJ_CHUNK);                                                           rc |= leaf_add(&L, a_chunk, K_METADATA_ITEM, 0, v, 33);
    block_group(v, NODE, BG_SYSTEM);                                                     rc |= leaf_add(&L, SYS_AT, K_BLOCK_GROUP, SYS_LEN, v, 24);
    extent_item(v, OBJ_EXTENT);                                                          rc |= leaf_add(&L, a_extent, K_METADATA_ITEM, 0, v, 33);
    block_group(v, 6 * NODE, BG_METADATA);                                               rc |= leaf_add(&L, META_AT, K_BLOCK_GROUP, META_LEN, v, 24);
    extent_item(v, OBJ_DEV);                                                             rc |= leaf_add(&L, a_dev, K_METADATA_ITEM, 0, v, 33);
    extent_item(v, OBJ_FS);                                                              rc |= leaf_add(&L, a_fs, K_METADATA_ITEM, 0, v, 33);
    extent_item(v, OBJ_ROOT);                                                            rc |= leaf_add(&L, a_root, K_METADATA_ITEM, 0, v, 33);
    extent_item(v, OBJ_CSUM);                                                            rc |= leaf_add(&L, a_csum, K_METADATA_ITEM, 0, v, 33);
    extent_item(v, DATA_RELOC_ID);                                                       rc |= leaf_add(&L, a_reloc, K_METADATA_ITEM, 0, v, 33);
    block_group(v, 0, BG_DATA);                                                          rc |= leaf_add(&L, DATA_AT, K_BLOCK_GROUP, data_len, v, 24);
    if (rc || leaf_write(&L, &c)) return -5;

    /* device tree */
    leaf_init(&L, &c, a_dev, OBJ_DEV);
    dev_extent(v, &c, SYS_AT, SYS_LEN);                                                  rc |= leaf_add(&L, 1, K_DEV_EXTENT, SYS_AT, v, 48);
    dev_extent(v, &c, META_AT, META_LEN);                                                rc |= leaf_add(&L, 1, K_DEV_EXTENT, META_AT, v, 48);
    dev_extent(v, &c, DATA_AT, data_len);                                                rc |= leaf_add(&L, 1, K_DEV_EXTENT, DATA_AT, v, 48);
    if (rc || leaf_write(&L, &c)) return -5;

    /* fs tree and data-reloc tree: an empty root directory */
    for (int which = 0; which < 2; which++) {
        leaf_init(&L, &c, which ? a_reloc : a_fs, which ? DATA_RELOC_ID : OBJ_FS);
        inode_item(v, 1, 040755, NODE, t);                                               rc |= leaf_add(&L, 256, K_INODE_ITEM, 0, v, 160);
        uint32_t n = inode_ref(v, "..");                                                 rc |= leaf_add(&L, 256, K_INODE_REF, 256, v, n);
        if (rc || leaf_write(&L, &c)) return -5;
    }

    /* csum tree: empty */
    leaf_init(&L, &c, a_csum, OBJ_CSUM);
    if (leaf_write(&L, &c)) return -5;

    /* root tree */
    leaf_init(&L, &c, a_root, OBJ_ROOT);
    root_item(v, a_extent, 0, t, NULL);                                                  rc |= leaf_add(&L, OBJ_EXTENT, K_ROOT_ITEM, 0, v, 439);
    root_item(v, a_dev, 0, t, NULL);                                                     rc |= leaf_add(&L, OBJ_DEV, K_ROOT_ITEM, 0, v, 439);
    uint32_t n = inode_ref(v, "default");                                                rc |= leaf_add(&L, OBJ_FS, K_INODE_REF, OBJ_ROOT_DIR, v, n);
    root_item(v, a_fs, 256, t, p->subvol_uuid);                                          rc |= leaf_add(&L, OBJ_FS, K_ROOT_ITEM, 0, v, 439);
    inode_item(v, 1, 040755, NODE, t);                                                   rc |= leaf_add(&L, OBJ_ROOT_DIR, K_INODE_ITEM, 0, v, 160);
    n = inode_ref(v, "..");                                                              rc |= leaf_add(&L, OBJ_ROOT_DIR, K_INODE_REF, OBJ_ROOT_DIR, v, n);
    n = dir_item_default(v);                                                             rc |= leaf_add(&L, OBJ_ROOT_DIR, K_DIR_ITEM, crc32c_raw(0xfffffffeu, (const uint8_t *)"default", 7), v, n);
    root_item(v, a_csum, 0, t, NULL);                                                    rc |= leaf_add(&L, OBJ_CSUM, K_ROOT_ITEM, 0, v, 439);
    root_item(v, a_reloc, 256, t, NULL);                                                 rc |= leaf_add(&L, DATA_RELOC_ID, K_ROOT_ITEM, 0, v, 439);
    if (rc || leaf_write(&L, &c)) return -5;

    /* superblock (primary and mirror) */
    uint8_t sb[4096];
    memset(sb, 0, sizeof sb);
    memcpy(sb + 32, p->fsid, 16);
    p64(sb, 56, 1);                                           /* WRITTEN */
    p64(sb, 64, 0x4d5f53665248425full);                       /* "_BHRfS_M" */
    p64(sb, 72, 1);                                           /* generation */
    p64(sb, 80, a_root); p64(sb, 88, a_chunk);
    p64(sb, 112, total); p64(sb, 120, 7 * NODE);
    p64(sb, 128, OBJ_ROOT_DIR); p64(sb, 136, 1);
    p32(sb, 144, SECTOR); p32(sb, 148, NODE); p32(sb, 152, NODE); p32(sb, 156, SECTOR);
    p32(sb, 160, (uint32_t)sizeof sys_array);
    p64(sb, 164, 1);                                          /* chunk_root_generation */
    p64(sb, 188, 0x1 | 0x20 | 0x40 | 0x100);                  /* MIXED_BACKREF | BIG_METADATA | EXTENDED_IREF | SKINNY_METADATA */
    p16(sb, 196, 0);                                          /* crc32c */
    dev_item(sb + 201, &c, total, used_dev);
    if (p->label) { size_t ln = strlen(p->label); if (ln > 255) ln = 255; memcpy(sb + 299, p->label, ln); }
    p64(sb, 555, ~0ull);                                      /* cache_generation: no free-space cache */
    memcpy(sb + 571, p->fsid, 16);                            /* metadata_uuid mirrors fsid */
    memcpy(sb + 811, sys_array, sizeof sys_array);
    p64(sb, 48, SUPER_OFF);
    p32(sb, 0, ~crc32c_raw(0xffffffffu, sb + 32, sizeof sb - 32));
    if (write(opaque, SUPER_OFF, sb, sizeof sb)) return -5;
    if (total >= MIRROR_OFF + sizeof sb) {
        p64(sb, 48, MIRROR_OFF);
        p32(sb, 0, ~crc32c_raw(0xffffffffu, sb + 32, sizeof sb - 32));
        if (write(opaque, MIRROR_OFF, sb, sizeof sb)) return -5;
    }
    return 0;
}
