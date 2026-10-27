#ifndef LOOS_BTRFS_INTEGRITY_H
#define LOOS_BTRFS_INTEGRITY_H

#include "btrfs.h"

#ifdef __cplusplus
extern "C" {
#endif

/* The helper deliberately checks only the structures understood by the
 * read-only LoOS reader: CRC32C superblocks, supported single/mirrored chunk
 * maps, and the metadata roots reached by loos_btrfs_mount().  It does not
 * claim to validate data checksums, parity RAID, compression, or free-space
 * accounting. */
struct loos_btrfs_integrity_report {
    int status;
    uint32_t superblocks_checked;
    uint32_t superblocks_valid;
    uint32_t chunks_checked;
    uint32_t metadata_roots_checked;
    uint64_t first_bad_offset;
};

/* Validate the primary superblock, available standard mirrors, supported chunk
 * mappings, and the chunk/root/filesystem trees.  node_storage follows the
 * same contract as loos_btrfs_mount(). */
int loos_btrfs_integrity_check(const struct loos_btrfs_io *io,
                               void *node_storage,
                               size_t node_storage_size,
                               struct loos_btrfs_integrity_report *report);

/* Restore only the primary superblock from a valid standard mirror.  Tree
 * blocks, extents, and data are intentionally untouched.  The caller must
 * provide write_at and flush callbacks; the source must be 64 MiB or 256 GiB.
 */
int loos_btrfs_rescue_restore_primary_superblock(
    const struct loos_btrfs_io *io,
    uint64_t source_offset,
    struct loos_btrfs_integrity_report *report);

#ifdef __cplusplus
}
#endif

#endif /* LOOS_BTRFS_INTEGRITY_H */
