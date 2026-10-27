#include "integrity.h"

#include <string.h>

#define BTRFS_MAGIC UINT64_C(0x4d5f53665248425f)
#define BTRFS_CSUM_TYPE_CRC32 UINT16_C(0)
#define BTRFS_CSUM_SIZE 32U
#define BTRFS_SUPER_FSID 32U
#define BTRFS_SUPER_BYTENR 48U
#define BTRFS_SUPER_MAGIC 64U
#define BTRFS_SUPER_TOTAL_BYTES 112U
#define BTRFS_SUPER_SECTORSIZE 144U
#define BTRFS_SUPER_NODESIZE 148U
#define BTRFS_SUPER_CSUM_TYPE 196U
#define BTRFS_SUPER_ROOT 80U
#define BTRFS_SUPER_CHUNK_ROOT 88U

#define BTRFS_PRIMARY_SUPERBLOCK UINT64_C(65536)
#define BTRFS_MIRROR_1 UINT64_C(67108864)
#define BTRFS_MIRROR_2 UINT64_C(274877906944)

static uint16_t read_le16(const uint8_t *p)
{
    return (uint16_t)p[0] | ((uint16_t)p[1] << 8);
}

static uint32_t read_le32(const uint8_t *p)
{
    return (uint32_t)p[0] |
           ((uint32_t)p[1] << 8) |
           ((uint32_t)p[2] << 16) |
           ((uint32_t)p[3] << 24);
}

static uint64_t read_le64(const uint8_t *p)
{
    return (uint64_t)read_le32(p) | ((uint64_t)read_le32(p + 4U) << 32);
}

static void write_le32(uint8_t *p, uint32_t value)
{
    p[0] = (uint8_t)value;
    p[1] = (uint8_t)(value >> 8);
    p[2] = (uint8_t)(value >> 16);
    p[3] = (uint8_t)(value >> 24);
}

static void write_le64(uint8_t *p, uint64_t value)
{
    write_le32(p, (uint32_t)value);
    write_le32(p + 4U, (uint32_t)(value >> 32));
}

static uint32_t crc32c_raw(uint32_t crc, const uint8_t *data, size_t length)
{
    for (size_t i = 0; i < length; ++i) {
        crc ^= data[i];
        for (unsigned bit = 0; bit < 8U; ++bit) {
            const uint32_t mask = (uint32_t)-(int)(crc & 1U);
            crc = (crc >> 1) ^ (UINT32_C(0x82f63b78) & mask);
        }
    }
    return crc;
}

static uint32_t crc32c(const uint8_t *data, size_t length)
{
    return ~crc32c_raw(UINT32_C(0xffffffff), data, length);
}

static bool checked_add_u64(uint64_t left, uint64_t right, uint64_t *result)
{
    if (right > UINT64_MAX - left) {
        return false;
    }
    *result = left + right;
    return true;
}

static void report_init(struct loos_btrfs_integrity_report *report)
{
    memset(report, 0, sizeof(*report));
    report->status = LOOS_BTRFS_OK;
}

static int report_failure(struct loos_btrfs_integrity_report *report,
                          int status,
                          uint64_t offset)
{
    if (report->status == LOOS_BTRFS_OK) {
        report->status = status;
        report->first_bad_offset = offset;
    }
    return status;
}

static int read_superblock(const struct loos_btrfs_io *io,
                           uint64_t offset,
                           uint8_t *buffer)
{
    uint64_t end;
    if (io == NULL || io->read_at == NULL ||
        !checked_add_u64(offset, LOOS_BTRFS_SUPERBLOCK_SIZE, &end)) {
        return LOOS_BTRFS_EINVAL;
    }
    (void)end;
    return io->read_at(io->opaque, offset, buffer,
                       LOOS_BTRFS_SUPERBLOCK_SIZE) == 0
               ? LOOS_BTRFS_OK
               : LOOS_BTRFS_EIO;
}

static bool is_standard_mirror(uint64_t offset)
{
    return offset == BTRFS_MIRROR_1 || offset == BTRFS_MIRROR_2;
}

static int validate_superblock(const uint8_t *buffer, uint64_t expected_offset)
{
    const uint64_t total_bytes = read_le64(buffer + BTRFS_SUPER_TOTAL_BYTES);
    const uint32_t sectorsize = read_le32(buffer + BTRFS_SUPER_SECTORSIZE);
    const uint32_t nodesize = read_le32(buffer + BTRFS_SUPER_NODESIZE);
    const uint64_t minimum_end = expected_offset + LOOS_BTRFS_SUPERBLOCK_SIZE;

    if (read_le16(buffer + BTRFS_SUPER_CSUM_TYPE) != BTRFS_CSUM_TYPE_CRC32) {
        return LOOS_BTRFS_ENOTSUP;
    }
    if (read_le32(buffer) != crc32c(buffer + BTRFS_CSUM_SIZE,
                                     LOOS_BTRFS_SUPERBLOCK_SIZE -
                                         BTRFS_CSUM_SIZE)) {
        return LOOS_BTRFS_ECORRUPT;
    }
    if (read_le64(buffer + BTRFS_SUPER_MAGIC) != BTRFS_MAGIC ||
        read_le64(buffer + BTRFS_SUPER_BYTENR) != expected_offset ||
        total_bytes < minimum_end || sectorsize < 4096U ||
        (sectorsize & (sectorsize - 1U)) != 0U || nodesize < sectorsize ||
        (nodesize & (nodesize - 1U)) != 0U ||
        nodesize > LOOS_BTRFS_MAX_NODE_SIZE ||
        read_le64(buffer + BTRFS_SUPER_ROOT) == 0 ||
        read_le64(buffer + BTRFS_SUPER_CHUNK_ROOT) == 0) {
        return LOOS_BTRFS_EBADFS;
    }
    return LOOS_BTRFS_OK;
}

static bool same_filesystem(const uint8_t *primary, const uint8_t *mirror)
{
    return memcmp(primary + BTRFS_SUPER_FSID,
                  mirror + BTRFS_SUPER_FSID, 16U) == 0 &&
           read_le64(primary + BTRFS_SUPER_TOTAL_BYTES) ==
               read_le64(mirror + BTRFS_SUPER_TOTAL_BYTES) &&
           read_le32(primary + BTRFS_SUPER_NODESIZE) ==
               read_le32(mirror + BTRFS_SUPER_NODESIZE) &&
           read_le64(primary + BTRFS_SUPER_ROOT) ==
               read_le64(mirror + BTRFS_SUPER_ROOT) &&
           read_le64(primary + BTRFS_SUPER_CHUNK_ROOT) ==
               read_le64(mirror + BTRFS_SUPER_CHUNK_ROOT);
}

static int validate_chunks(const struct loos_btrfs_mount *mount,
                           struct loos_btrfs_integrity_report *report)
{
    for (size_t i = 0; i < mount->chunk_count; ++i) {
        const struct loos_btrfs_chunk *chunk = &mount->chunks[i];
        uint64_t logical_end;
        uint64_t physical_end;
        if (chunk->length == 0 || chunk->num_stripes == 0 ||
            !checked_add_u64(chunk->logical, chunk->length, &logical_end) ||
            !checked_add_u64(chunk->physical, chunk->length, &physical_end) ||
            physical_end > mount->total_bytes) {
            return report_failure(report, LOOS_BTRFS_ECORRUPT,
                                  chunk->physical);
        }
        for (size_t j = 0; j < i; ++j) {
            const struct loos_btrfs_chunk *other = &mount->chunks[j];
            uint64_t other_end;
            if (!checked_add_u64(other->logical, other->length,
                                 &other_end) ||
                (chunk->logical < other_end && other->logical < logical_end)) {
                return report_failure(report, LOOS_BTRFS_ECORRUPT,
                                      chunk->logical);
            }
        }
    }
    report->chunks_checked = (uint32_t)mount->chunk_count;
    return LOOS_BTRFS_OK;
}

int loos_btrfs_integrity_check(const struct loos_btrfs_io *io,
                               void *node_storage,
                               size_t node_storage_size,
                               struct loos_btrfs_integrity_report *report)
{
    uint8_t primary[LOOS_BTRFS_SUPERBLOCK_SIZE];
    struct loos_btrfs_mount mount;
    struct loos_btrfs_stat root_stat;
    const uint64_t mirror_offsets[] = {BTRFS_MIRROR_1, BTRFS_MIRROR_2};

    if (report == NULL || io == NULL || io->read_at == NULL ||
        node_storage == NULL) {
        return LOOS_BTRFS_EINVAL;
    }
    report_init(report);

    int status = read_superblock(io, BTRFS_PRIMARY_SUPERBLOCK, primary);
    report->superblocks_checked = 1;
    if (status != LOOS_BTRFS_OK) {
        return report_failure(report, status, BTRFS_PRIMARY_SUPERBLOCK);
    }
    status = validate_superblock(primary, BTRFS_PRIMARY_SUPERBLOCK);
    if (status != LOOS_BTRFS_OK) {
        return report_failure(report, status, BTRFS_PRIMARY_SUPERBLOCK);
    }
    report->superblocks_valid = 1;

    const uint64_t total_bytes = read_le64(primary + BTRFS_SUPER_TOTAL_BYTES);
    for (size_t i = 0; i < sizeof(mirror_offsets) / sizeof(mirror_offsets[0]);
         ++i) {
        const uint64_t offset = mirror_offsets[i];
        uint64_t end;
        uint8_t mirror[LOOS_BTRFS_SUPERBLOCK_SIZE];
        if (!checked_add_u64(offset, LOOS_BTRFS_SUPERBLOCK_SIZE, &end) ||
            end > total_bytes) {
            continue;
        }
        report->superblocks_checked++;
        status = read_superblock(io, offset, mirror);
        if (status != LOOS_BTRFS_OK) {
            return report_failure(report, status, offset);
        }
        status = validate_superblock(mirror, offset);
        if (status != LOOS_BTRFS_OK || !same_filesystem(primary, mirror)) {
            if (status == LOOS_BTRFS_OK) {
                status = LOOS_BTRFS_ECORRUPT;
            }
            return report_failure(report, status, offset);
        }
        report->superblocks_valid++;
    }

    status = loos_btrfs_mount(&mount, io, node_storage, node_storage_size,
                              true);
    if (status != LOOS_BTRFS_OK) {
        return report_failure(report, status, 0);
    }
    status = validate_chunks(&mount, report);
    if (status == LOOS_BTRFS_OK) {
        status = loos_btrfs_stat_path(&mount, "/", &root_stat);
    }
    if (status == LOOS_BTRFS_OK) {
        /* mount() has checked the chunk and root trees; stat_path() reaches
         * the filesystem tree and confirms its root inode is readable. */
        report->metadata_roots_checked = 3;
    }
    loos_btrfs_unmount(&mount);
    if (status != LOOS_BTRFS_OK) {
        return report_failure(report, status, 0);
    }
    return LOOS_BTRFS_OK;
}

int loos_btrfs_rescue_restore_primary_superblock(
    const struct loos_btrfs_io *io,
    uint64_t source_offset,
    struct loos_btrfs_integrity_report *report)
{
    uint8_t superblock[LOOS_BTRFS_SUPERBLOCK_SIZE];

    if (report == NULL) {
        return LOOS_BTRFS_EINVAL;
    }
    report_init(report);
    if (io == NULL || io->read_at == NULL || io->write_at == NULL ||
        io->flush == NULL || !is_standard_mirror(source_offset)) {
        const int status = io != NULL &&
                                   (io->write_at == NULL || io->flush == NULL)
                               ? LOOS_BTRFS_EROFS
                               : LOOS_BTRFS_EINVAL;
        return report_failure(report, status, source_offset);
    }
    report->superblocks_checked = 1;

    int status = read_superblock(io, source_offset, superblock);
    if (status != LOOS_BTRFS_OK) {
        return report_failure(report, status, source_offset);
    }
    status = validate_superblock(superblock, source_offset);
    if (status != LOOS_BTRFS_OK) {
        return report_failure(report, status, source_offset);
    }
    report->superblocks_valid = 1;

    write_le64(superblock + BTRFS_SUPER_BYTENR, BTRFS_PRIMARY_SUPERBLOCK);
    memset(superblock, 0, BTRFS_CSUM_SIZE);
    write_le32(superblock,
               crc32c(superblock + BTRFS_CSUM_SIZE,
                      LOOS_BTRFS_SUPERBLOCK_SIZE - BTRFS_CSUM_SIZE));
    if (io->write_at(io->opaque, BTRFS_PRIMARY_SUPERBLOCK, superblock,
                     LOOS_BTRFS_SUPERBLOCK_SIZE) != 0) {
        return report_failure(report, LOOS_BTRFS_EIO,
                              BTRFS_PRIMARY_SUPERBLOCK);
    }
    if (io->flush(io->opaque) != 0) {
        return report_failure(report, LOOS_BTRFS_EIO,
                              BTRFS_PRIMARY_SUPERBLOCK);
    }
    return LOOS_BTRFS_OK;
}
