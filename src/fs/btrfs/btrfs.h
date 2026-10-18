#ifndef LOOS_BTRFS_H
#define LOOS_BTRFS_H

/*
 * LoOS read-only btrfs interface.
 *
 * The filesystem talks to a block device through read_at.  No host file,
 * libc allocator, or global state is required by the reader, which keeps the
 * same interface usable by the kernel and by the offline test harness.
 */

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define LOOS_BTRFS_SUPERBLOCK_OFFSET UINT64_C(65536)
#define LOOS_BTRFS_SUPERBLOCK_SIZE UINT32_C(4096)
#define LOOS_BTRFS_MAX_NODE_SIZE UINT32_C(65536)
#define LOOS_BTRFS_MAX_TREE_LEVEL UINT32_C(8)
#define LOOS_BTRFS_NODE_STORAGE_SIZE \
    (LOOS_BTRFS_MAX_NODE_SIZE * (LOOS_BTRFS_MAX_TREE_LEVEL + 1U))
#define LOOS_BTRFS_MAX_CHUNKS UINT32_C(512)
#define LOOS_BTRFS_MAX_NAME UINT32_C(255)

enum loos_btrfs_status {
    LOOS_BTRFS_OK = 0,
    LOOS_BTRFS_STOP = 1,
    LOOS_BTRFS_EINVAL = -1,
    LOOS_BTRFS_EIO = -2,
    LOOS_BTRFS_EBADFS = -3,
    LOOS_BTRFS_ENOTFOUND = -4,
    LOOS_BTRFS_ENOTDIR = -5,
    LOOS_BTRFS_EISDIR = -6,
    LOOS_BTRFS_ENOTSUP = -7,
    LOOS_BTRFS_ECORRUPT = -8,
    LOOS_BTRFS_EOVERFLOW = -9,
    LOOS_BTRFS_ELIMIT = -10,
};

/* Return zero after filling the complete buffer; return a negative status on
 * a short or failed read.  Reads are always physical device offsets. */
typedef int (*loos_btrfs_read_at_fn)(void *opaque,
                                     uint64_t physical_offset,
                                     void *buffer,
                                     size_t length);

struct loos_btrfs_io {
    loos_btrfs_read_at_fn read_at;
    void *opaque;
};

struct loos_btrfs_chunk {
    uint64_t logical;
    uint64_t length;
    uint64_t physical;
    uint64_t type;
    uint64_t devid;
    uint16_t num_stripes;
};

/* The struct is intentionally caller-owned.  node_storage must be at least
 * LOOS_BTRFS_NODE_STORAGE_SIZE bytes and aligned for byte access. */
struct loos_btrfs_mount {
    struct loos_btrfs_io io;
    uint8_t *node_storage;
    size_t node_storage_size;
    bool verify_checksums;
    bool mounted;

    uint16_t csum_type;
    uint8_t fsid[16];
    uint8_t metadata_uuid[16];
    uint64_t generation;
    uint64_t total_bytes;
    uint32_t sectorsize;
    uint32_t nodesize;

    uint64_t root_tree;
    uint64_t chunk_tree;
    uint64_t fs_tree;
    uint64_t csum_tree;
    uint64_t fs_root_dirid;
    uint8_t root_tree_level;
    uint8_t chunk_tree_level;
    uint8_t fs_tree_level;
    uint8_t csum_tree_level;
    bool has_csum_tree;

    size_t chunk_count;
    struct loos_btrfs_chunk chunks[LOOS_BTRFS_MAX_CHUNKS];
};

struct loos_btrfs_dirent {
    uint64_t inode;
    uint8_t type;
    uint16_t name_length;
    char name[LOOS_BTRFS_MAX_NAME + 1U];
};

typedef int (*loos_btrfs_dirent_fn)(void *opaque,
                                    const struct loos_btrfs_dirent *entry);

/* Mounts the top-level filesystem root read-only.  Metadata tree blocks are
 * checked by default when verify_checksums is true. */
int loos_btrfs_mount(struct loos_btrfs_mount *mount,
                     const struct loos_btrfs_io *io,
                     void *node_storage,
                     size_t node_storage_size,
                     bool verify_checksums);

void loos_btrfs_unmount(struct loos_btrfs_mount *mount);

/* Enumerates the directory at path.  Paths use '/' separators and are UTF-8
 * byte strings; name comparison is byte-exact at this layer. */
int loos_btrfs_list_dir(struct loos_btrfs_mount *mount,
                        const char *path,
                        loos_btrfs_dirent_fn callback,
                        void *opaque);

/* Reads up to length bytes from a regular file.  Holes are returned as zeroes.
 * Compressed, encrypted, and unsupported striped extents are rejected rather
 * than silently returning incorrect data. */
int loos_btrfs_read_file(struct loos_btrfs_mount *mount,
                         const char *path,
                         uint64_t offset,
                         void *buffer,
                         size_t length,
                         size_t *bytes_read);

#ifdef __cplusplus
}
#endif

#endif /* LOOS_BTRFS_H */
