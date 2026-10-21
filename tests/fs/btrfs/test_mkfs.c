/* Formats an in-memory device with loos_btrfs_mkfs and mounts it with the checksum-verifying reader. */
#include "btrfs.h"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>

static int fails;
#define CHECK(c, m) do { if (!(c)) { printf("FAIL: %s\n", m); fails++; } } while (0)

static uint8_t *dev;
static size_t dev_size;
static int dev_write(void *o, uint64_t off, const void *d, size_t n) { (void)o; if (off + n > dev_size) return -1; memcpy(dev + off, d, n); return 0; }
static int dev_read(void *o, uint64_t off, void *d, size_t n) { (void)o; if (off + n > dev_size) return -1; memcpy(d, dev + off, n); return 0; }
static int count_entry(void *o, const struct loos_btrfs_dirent *e) { (void)e; ++*(int *)o; return 0; }

int main(void)
{
    dev_size = 200u << 20;
    dev = mmap(0, dev_size, PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
    struct loos_btrfs_mkfs_params p;
    memset(&p, 0, sizeof p);
    p.device_bytes = dev_size; p.label = "LoOS test"; p.unix_time = 1700000000;
    for (int i = 0; i < 16; i++) { p.fsid[i] = (uint8_t)(i + 1); p.dev_uuid[i] = (uint8_t)(0x40 + i); p.chunk_tree_uuid[i] = (uint8_t)(0x80 + i); p.subvol_uuid[i] = (uint8_t)(0xc0 + i); }
    CHECK(loos_btrfs_mkfs(&p, dev_write, 0) == 0, "format");
    p.device_bytes = 64u << 20;
    CHECK(loos_btrfs_mkfs(&p, dev_write, 0) < 0, "reject a device below 128 MiB");
    CHECK(loos_btrfs_mkfs(0, dev_write, 0) < 0 && loos_btrfs_mkfs(&p, 0, 0) < 0, "reject missing arguments");

    static uint8_t nodes[LOOS_BTRFS_NODE_STORAGE_SIZE];
    struct loos_btrfs_io io = { .read_at = dev_read, .write_at = 0, .flush = 0, .opaque = 0 };
    struct loos_btrfs_mount m;
    CHECK(loos_btrfs_mount(&m, &io, nodes, sizeof nodes, true) == LOOS_BTRFS_OK, "mount with checksum verification");
    CHECK(m.chunk_count == 3 && m.fs_tree != 0 && m.fs_root_dirid == 256, "three chunks, fs root, root directory inode");
    int n = 0;
    CHECK(loos_btrfs_list_dir(&m, "/", count_entry, &n) == LOOS_BTRFS_OK && n == 0, "root directory is empty");
    struct loos_btrfs_stat st;
    CHECK(loos_btrfs_stat_path(&m, "/", &st) == LOOS_BTRFS_OK && (st.mode & 0170000) == 0040000, "root is a directory");
    loos_btrfs_unmount(&m);

    CHECK(memcmp(dev + 0x10000 + 64, "_BHRfS_M", 8) == 0 && memcmp(dev + 0x4000000 + 64, "_BHRfS_M", 8) == 0, "primary and mirror superblocks");
    dev[0x500000 + 200] ^= 1;                                                      /* corrupt the extent tree */
    dev[0x508000 + 200] ^= 1;                                                      /* ...and the fs tree */
    CHECK(loos_btrfs_mount(&m, &io, nodes, sizeof nodes, true) != LOOS_BTRFS_OK || loos_btrfs_list_dir(&m, "/", count_entry, &n) != LOOS_BTRFS_OK, "a damaged tree block is detected");
    if (!fails) printf("btrfs_mkfs: all passed\n");
    return fails != 0;
}
