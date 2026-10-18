#include "btrfs.h"

#include <string.h>

/* On-disk constants are kept here rather than exposing packed disk structs to
 * callers.  Every multi-byte value is decoded explicitly as little-endian. */
#define BTRFS_MAGIC UINT64_C(0x4d5f53665248425f)
#define BTRFS_CSUM_TYPE_CRC32 UINT16_C(0)
#define BTRFS_CSUM_SIZE 32U
#define BTRFS_HEADER_SIZE 101U
#define BTRFS_DISK_KEY_SIZE 17U
#define BTRFS_ITEM_SIZE 25U
#define BTRFS_KEY_PTR_SIZE 33U
#define BTRFS_SYSTEM_CHUNK_ARRAY_SIZE 2048U

#define BTRFS_ROOT_TREE_OBJECTID UINT64_C(1)
#define BTRFS_CHUNK_TREE_OBJECTID UINT64_C(3)
#define BTRFS_FS_TREE_OBJECTID UINT64_C(5)
#define BTRFS_CSUM_TREE_OBJECTID UINT64_C(7)
#define BTRFS_EXTENT_CSUM_OBJECTID UINT64_C(0xfffffffffffffff6)

#define BTRFS_INODE_ITEM_KEY UINT8_C(1)
#define BTRFS_DIR_ITEM_KEY UINT8_C(84)
#define BTRFS_EXTENT_DATA_KEY UINT8_C(108)
#define BTRFS_EXTENT_CSUM_KEY UINT8_C(128)
#define BTRFS_ROOT_ITEM_KEY UINT8_C(132)
#define BTRFS_CHUNK_ITEM_KEY UINT8_C(228)

#define BTRFS_FILE_EXTENT_INLINE UINT8_C(0)
#define BTRFS_FILE_EXTENT_REG UINT8_C(1)
#define BTRFS_FILE_EXTENT_PREALLOC UINT8_C(2)

#define BTRFS_BLOCK_GROUP_RAID0 (UINT64_C(1) << 3)
#define BTRFS_BLOCK_GROUP_RAID1 (UINT64_C(1) << 4)
#define BTRFS_BLOCK_GROUP_DUP (UINT64_C(1) << 5)
#define BTRFS_BLOCK_GROUP_RAID10 (UINT64_C(1) << 6)
#define BTRFS_BLOCK_GROUP_RAID5 (UINT64_C(1) << 7)
#define BTRFS_BLOCK_GROUP_RAID6 (UINT64_C(1) << 8)
#define BTRFS_BLOCK_GROUP_RAID1C3 (UINT64_C(1) << 9)
#define BTRFS_BLOCK_GROUP_RAID1C4 (UINT64_C(1) << 10)

#define BTRFS_SUPER_FSID 32U
#define BTRFS_SUPER_BYTENR 48U
#define BTRFS_SUPER_MAGIC 64U
#define BTRFS_SUPER_GENERATION 72U
#define BTRFS_SUPER_ROOT 80U
#define BTRFS_SUPER_CHUNK_ROOT 88U
#define BTRFS_SUPER_TOTAL_BYTES 112U
#define BTRFS_SUPER_ROOT_DIR_OBJECTID 128U
#define BTRFS_SUPER_SECTORSIZE 144U
#define BTRFS_SUPER_NODESIZE 148U
#define BTRFS_SUPER_SYS_CHUNK_ARRAY_SIZE 160U
#define BTRFS_SUPER_CHUNK_ROOT_GENERATION 164U
#define BTRFS_SUPER_CSUM_TYPE 196U
#define BTRFS_SUPER_ROOT_LEVEL 198U
#define BTRFS_SUPER_CHUNK_ROOT_LEVEL 199U
#define BTRFS_SUPER_DEV_ITEM 201U
#define BTRFS_SUPER_LABEL (BTRFS_SUPER_DEV_ITEM + 98U)
#define BTRFS_SUPER_METADATA_UUID (BTRFS_SUPER_LABEL + 256U + 16U)
#define BTRFS_SUPER_SYS_CHUNK_ARRAY \
    (BTRFS_SUPER_METADATA_UUID + 16U + 8U + 216U)

#define BTRFS_HEADER_CSUM 0U
#define BTRFS_HEADER_FSID 32U
#define BTRFS_HEADER_BYTENR 48U
#define BTRFS_HEADER_GENERATION 80U
#define BTRFS_HEADER_OWNER 88U
#define BTRFS_HEADER_NRITEMS 96U
#define BTRFS_HEADER_LEVEL 100U

#define BTRFS_ROOT_ITEM_ROOT_DIRID 168U
#define BTRFS_ROOT_ITEM_BYTENR 176U
#define BTRFS_ROOT_ITEM_LEVEL 238U

#define BTRFS_INODE_SIZE 16U
#define BTRFS_INODE_NLINK 40U
#define BTRFS_INODE_UID 44U
#define BTRFS_INODE_GID 48U
#define BTRFS_INODE_FLAGS 64U
#define BTRFS_INODE_MODE 52U
#define BTRFS_INODE_ITEM_MIN_SIZE 160U

#define BTRFS_DIR_ITEM_LOCATION 0U
#define BTRFS_DIR_ITEM_TRANSID 17U
#define BTRFS_DIR_ITEM_DATA_LEN 25U
#define BTRFS_DIR_ITEM_NAME_LEN 27U
#define BTRFS_DIR_ITEM_TYPE 29U
#define BTRFS_DIR_ITEM_HEADER_SIZE 30U

#define BTRFS_FILE_EXTENT_COMPRESSION 16U
#define BTRFS_FILE_EXTENT_ENCRYPTION 17U
#define BTRFS_FILE_EXTENT_TYPE 20U
#define BTRFS_FILE_EXTENT_INLINE_DATA 21U
#define BTRFS_FILE_EXTENT_DISK_BYTENR 21U
#define BTRFS_FILE_EXTENT_DISK_NUM_BYTES 29U
#define BTRFS_FILE_EXTENT_OFFSET 37U
#define BTRFS_FILE_EXTENT_NUM_BYTES 45U
#define BTRFS_FILE_EXTENT_MIN_SIZE 53U

struct btrfs_key {
    uint64_t objectid;
    uint8_t type;
    uint64_t offset;
};

struct btrfs_item_view {
    struct btrfs_key key;
    const uint8_t *data;
    uint32_t size;
};

typedef int (*btrfs_item_fn)(struct loos_btrfs_mount *mount,
                             const struct btrfs_item_view *item,
                             void *opaque);

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
    return (uint64_t)read_le32(p) | ((uint64_t)read_le32(p + 4) << 32);
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
    write_le32(p + 4, (uint32_t)(value >> 32));
}

static bool checked_add_u64(uint64_t left, uint64_t right, uint64_t *result)
{
    if (right > UINT64_MAX - left) {
        return false;
    }
    *result = left + right;
    return true;
}

static bool checked_mul_size(size_t left, size_t right, size_t *result)
{
    if (right != 0 && left > SIZE_MAX / right) {
        return false;
    }
    *result = left * right;
    return true;
}

static bool fits_size_t(uint64_t value)
{
    const size_t converted = (size_t)value;
    return (uint64_t)converted == value;
}

/* Reflected CRC32C core.  btrfs uses the raw form for directory-name hashes
 * and the complemented form for stored block checksums. */
static uint32_t crc32c_raw(uint32_t crc, const uint8_t *data, size_t length)
{
    for (size_t i = 0; i < length; ++i) {
        crc ^= data[i];
        for (unsigned bit = 0; bit < 8; ++bit) {
            const uint32_t mask = (uint32_t)-(int)(crc & 1U);
            crc = (crc >> 1) ^ (UINT32_C(0x82f63b78) & mask);
        }
    }
    return crc;
}

static uint32_t crc32c(uint32_t seed, const uint8_t *data, size_t length)
{
    return ~crc32c_raw(~seed, data, length);
}

static int read_physical(struct loos_btrfs_mount *mount,
                         uint64_t physical,
                         void *buffer,
                         size_t length)
{
    if (length == 0) {
        return LOOS_BTRFS_OK;
    }
    if (mount->io.read_at == NULL) {
        return LOOS_BTRFS_EINVAL;
    }
    return mount->io.read_at(mount->io.opaque, physical, buffer, length) == 0
               ? LOOS_BTRFS_OK
               : LOOS_BTRFS_EIO;
}

static int write_physical(struct loos_btrfs_mount *mount,
                          uint64_t physical,
                          const void *buffer,
                          size_t length)
{
    if (length == 0) {
        return LOOS_BTRFS_OK;
    }
    if (mount->io.write_at == NULL) {
        return LOOS_BTRFS_EINVAL;
    }
    return mount->io.write_at(mount->io.opaque, physical, buffer, length) == 0
               ? LOOS_BTRFS_OK
               : LOOS_BTRFS_EIO;
}

static int flush_device(struct loos_btrfs_mount *mount)
{
    if (mount->io.flush == NULL) {
        return LOOS_BTRFS_EINVAL;
    }
    return mount->io.flush(mount->io.opaque) == 0 ? LOOS_BTRFS_OK
                                                  : LOOS_BTRFS_EIO;
}

static int write_superblock_mirrors(struct loos_btrfs_mount *mount,
                                    uint8_t *superblock)
{
    static const uint64_t mirror_offsets[] = {
        UINT64_C(65536),
        UINT64_C(67108864),
        UINT64_C(274877906944),
    };
    const uint64_t original_bytenr = read_le64(superblock + BTRFS_SUPER_BYTENR);
    for (size_t i = 0; i < sizeof(mirror_offsets) / sizeof(mirror_offsets[0]);
         ++i) {
        const uint64_t mirror = mirror_offsets[i];
        if (mirror == LOOS_BTRFS_SUPERBLOCK_OFFSET ||
            mirror >= mount->total_bytes) {
            continue;
        }
        write_le64(superblock + BTRFS_SUPER_BYTENR, mirror);
        memset(superblock, 0, BTRFS_CSUM_SIZE);
        write_le32(superblock,
                   crc32c(0, superblock + BTRFS_CSUM_SIZE,
                          LOOS_BTRFS_SUPERBLOCK_SIZE - BTRFS_CSUM_SIZE));
        int status = write_physical(mount, mirror, superblock,
                                    LOOS_BTRFS_SUPERBLOCK_SIZE);
        if (status != LOOS_BTRFS_OK) {
            write_le64(superblock + BTRFS_SUPER_BYTENR, original_bytenr);
            return status;
        }
    }
    write_le64(superblock + BTRFS_SUPER_BYTENR, original_bytenr);
    memset(superblock, 0, BTRFS_CSUM_SIZE);
    write_le32(superblock,
               crc32c(0, superblock + BTRFS_CSUM_SIZE,
                      LOOS_BTRFS_SUPERBLOCK_SIZE - BTRFS_CSUM_SIZE));
    return LOOS_BTRFS_OK;
}

static bool is_power_of_two(uint32_t value)
{
    return value != 0 && (value & (value - 1U)) == 0;
}

static int compare_keys(const struct btrfs_key *left,
                        const struct btrfs_key *right)
{
    if (left->objectid < right->objectid) {
        return -1;
    }
    if (left->objectid > right->objectid) {
        return 1;
    }
    if (left->type < right->type) {
        return -1;
    }
    if (left->type > right->type) {
        return 1;
    }
    if (left->offset < right->offset) {
        return -1;
    }
    if (left->offset > right->offset) {
        return 1;
    }
    return 0;
}

static struct btrfs_key disk_key(const uint8_t *data)
{
    struct btrfs_key key;
    key.objectid = read_le64(data);
    key.type = data[8];
    key.offset = read_le64(data + 9);
    return key;
}

static int verify_checksum(struct loos_btrfs_mount *mount,
                           const uint8_t *block,
                           size_t length)
{
    if (!mount->verify_checksums) {
        return LOOS_BTRFS_OK;
    }
    if (mount->csum_type != BTRFS_CSUM_TYPE_CRC32) {
        return LOOS_BTRFS_ENOTSUP;
    }
    if (length < BTRFS_CSUM_SIZE) {
        return LOOS_BTRFS_ECORRUPT;
    }

    const uint32_t expected = read_le32(block + BTRFS_HEADER_CSUM);
    const uint32_t actual = crc32c(0, block + BTRFS_CSUM_SIZE,
                                   length - BTRFS_CSUM_SIZE);
    return expected == actual ? LOOS_BTRFS_OK : LOOS_BTRFS_ECORRUPT;
}

static int add_chunk(struct loos_btrfs_mount *mount,
                     uint64_t logical,
                     uint64_t length,
                     uint64_t physical,
                     uint64_t type,
                     uint64_t devid,
                     uint16_t num_stripes)
{
    const uint64_t unsupported_profiles =
        BTRFS_BLOCK_GROUP_RAID0 | BTRFS_BLOCK_GROUP_RAID10 |
        BTRFS_BLOCK_GROUP_RAID5 | BTRFS_BLOCK_GROUP_RAID6 |
        BTRFS_BLOCK_GROUP_RAID1C3 | BTRFS_BLOCK_GROUP_RAID1C4;
    uint64_t logical_end;
    uint64_t physical_end;

    if (length == 0 || num_stripes == 0 ||
        !checked_add_u64(logical, length, &logical_end) ||
        !checked_add_u64(physical, length, &physical_end)) {
        return LOOS_BTRFS_ECORRUPT;
    }
    if ((type & unsupported_profiles) != 0) {
        return LOOS_BTRFS_ENOTSUP;
    }

    for (size_t i = 0; i < mount->chunk_count; ++i) {
        struct loos_btrfs_chunk *existing = &mount->chunks[i];
        if (existing->logical != logical) {
            uint64_t existing_end;
            if (!checked_add_u64(existing->logical, existing->length,
                                 &existing_end) ||
                (logical < existing_end && existing->logical < logical_end)) {
                return LOOS_BTRFS_ECORRUPT;
            }
            continue;
        }
        if (existing->length != length || existing->physical != physical) {
            return LOOS_BTRFS_ECORRUPT;
        }
        return LOOS_BTRFS_OK;
    }

    if (mount->chunk_count == LOOS_BTRFS_MAX_CHUNKS) {
        return LOOS_BTRFS_ELIMIT;
    }
    struct loos_btrfs_chunk *chunk = &mount->chunks[mount->chunk_count++];
    chunk->logical = logical;
    chunk->length = length;
    chunk->physical = physical;
    chunk->type = type;
    chunk->devid = devid;
    chunk->num_stripes = num_stripes;
    return LOOS_BTRFS_OK;
}

static int parse_chunk_item(struct loos_btrfs_mount *mount,
                            uint64_t logical,
                            const uint8_t *data,
                            size_t size)
{
    const size_t stripe_size = 32U;
    const size_t chunk_header_size = 48U;
    if (size < chunk_header_size) {
        return LOOS_BTRFS_ECORRUPT;
    }

    const uint64_t length = read_le64(data);
    const uint64_t type = read_le64(data + 24U);
    const uint16_t num_stripes = read_le16(data + 44U);
    size_t stripes_bytes;
    size_t needed;
    if (!checked_mul_size((size_t)num_stripes, stripe_size, &stripes_bytes) ||
        chunk_header_size > SIZE_MAX - stripes_bytes) {
        return LOOS_BTRFS_ECORRUPT;
    }
    needed = chunk_header_size + stripes_bytes;
    if (needed > size) {
        return LOOS_BTRFS_ECORRUPT;
    }

    /* SINGLE, DUP, and mirrored chunks all have a usable first stripe.  RAID0
     * and parity profiles are rejected above because their address mapping is
     * not a simple logical-to-physical offset. */
    const uint64_t physical = read_le64(data + chunk_header_size + 8U);
    const uint64_t devid = read_le64(data + chunk_header_size);
    return add_chunk(mount, logical, length, physical, type, devid,
                     num_stripes);
}

static int map_logical(const struct loos_btrfs_mount *mount,
                       uint64_t logical,
                       size_t length,
                       uint64_t *physical,
                       size_t *available)
{
    for (size_t i = 0; i < mount->chunk_count; ++i) {
        const struct loos_btrfs_chunk *chunk = &mount->chunks[i];
        if (logical < chunk->logical) {
            continue;
        }
        const uint64_t delta = logical - chunk->logical;
        if (delta >= chunk->length ||
            (length != 0 && (uint64_t)length > chunk->length - delta)) {
            continue;
        }
        if (!checked_add_u64(chunk->physical, delta, physical)) {
            return LOOS_BTRFS_EOVERFLOW;
        }
        const uint64_t remaining = chunk->length - delta;
        if (remaining > SIZE_MAX) {
            *available = SIZE_MAX;
        } else {
            *available = (size_t)remaining;
        }
        return LOOS_BTRFS_OK;
    }
    return LOOS_BTRFS_EBADFS;
}

static int read_logical(struct loos_btrfs_mount *mount,
                        uint64_t logical,
                        void *buffer,
                        size_t length)
{
    uint8_t *cursor = (uint8_t *)buffer;
    size_t remaining = length;

    while (remaining != 0) {
        uint64_t physical;
        size_t available;
        int status = map_logical(mount, logical, 0, &physical, &available);
        if (status != LOOS_BTRFS_OK) {
            return status;
        }
        const size_t step = remaining < available ? remaining : available;
        status = read_physical(mount, physical, cursor, step);
        if (status != LOOS_BTRFS_OK) {
            return status;
        }
        if (!checked_add_u64(logical, (uint64_t)step, &logical)) {
            return LOOS_BTRFS_EOVERFLOW;
        }
        cursor += step;
        remaining -= step;
    }
    return LOOS_BTRFS_OK;
}

static int read_tree_block(struct loos_btrfs_mount *mount,
                           uint64_t logical,
                           unsigned depth,
                           uint8_t expected_level,
                           uint64_t expected_owner,
                           uint8_t **block_out,
                           uint8_t *level_out,
                           uint32_t *item_count_out)
{
    if (depth > LOOS_BTRFS_MAX_TREE_LEVEL || mount->nodesize == 0 ||
        mount->node_storage == NULL) {
        return LOOS_BTRFS_EINVAL;
    }

    size_t depth_offset;
    if (!checked_mul_size((size_t)depth, mount->nodesize, &depth_offset) ||
        depth_offset > mount->node_storage_size ||
        mount->nodesize > mount->node_storage_size - depth_offset) {
        return LOOS_BTRFS_EINVAL;
    }
    uint8_t *block = mount->node_storage + depth_offset;
    int status = read_logical(mount, logical, block, mount->nodesize);
    if (status != LOOS_BTRFS_OK) {
        return status;
    }

    status = verify_checksum(mount, block, mount->nodesize);
    if (status != LOOS_BTRFS_OK) {
        return status;
    }
    if (read_le64(block + BTRFS_HEADER_BYTENR) != logical ||
        memcmp(block + BTRFS_HEADER_FSID, mount->metadata_uuid, 16U) != 0) {
        return LOOS_BTRFS_ECORRUPT;
    }

    const uint8_t level = block[BTRFS_HEADER_LEVEL];
    const uint32_t item_count = read_le32(block + BTRFS_HEADER_NRITEMS);
    if (level > LOOS_BTRFS_MAX_TREE_LEVEL || level != expected_level ||
        (expected_owner != 0 &&
         read_le64(block + BTRFS_HEADER_OWNER) != expected_owner)) {
        return LOOS_BTRFS_ECORRUPT;
    }

    const size_t entry_size = level == 0 ? BTRFS_ITEM_SIZE : BTRFS_KEY_PTR_SIZE;
    size_t entries_bytes;
    if (!checked_mul_size((size_t)item_count, entry_size, &entries_bytes) ||
        BTRFS_HEADER_SIZE > mount->nodesize -
                                (entries_bytes > mount->nodesize
                                     ? mount->nodesize
                                     : entries_bytes)) {
        return LOOS_BTRFS_ECORRUPT;
    }
    if (entries_bytes > mount->nodesize - BTRFS_HEADER_SIZE) {
        return LOOS_BTRFS_ECORRUPT;
    }

    *block_out = block;
    *level_out = level;
    *item_count_out = item_count;
    return LOOS_BTRFS_OK;
}

static int scan_tree_range(struct loos_btrfs_mount *mount,
                           uint64_t logical,
                           uint8_t level,
                           uint64_t owner,
                           const struct btrfs_key *lower,
                           const struct btrfs_key *upper,
                           unsigned depth,
                           btrfs_item_fn callback,
                           void *opaque)
{
    uint8_t *block;
    uint8_t actual_level;
    uint32_t item_count;
    int status = read_tree_block(mount, logical, depth, level, owner, &block,
                                 &actual_level, &item_count);
    if (status != LOOS_BTRFS_OK) {
        return status;
    }

    if (actual_level == 0) {
        struct btrfs_key previous_key = {0, 0, 0};
        for (uint32_t i = 0; i < item_count; ++i) {
            const size_t item_offset = BTRFS_HEADER_SIZE +
                                       (size_t)i * BTRFS_ITEM_SIZE;
            const uint8_t *item_data = block + item_offset;
            const struct btrfs_key key = disk_key(item_data);
            if (i != 0 && compare_keys(&previous_key, &key) >= 0) {
                return LOOS_BTRFS_ECORRUPT;
            }
            previous_key = key;
            const uint32_t data_offset = read_le32(item_data + 17U);
            const uint32_t data_size = read_le32(item_data + 21U);
            const uint32_t data_area_size =
                mount->nodesize - BTRFS_HEADER_SIZE;
            const uint32_t data_area_start =
                (uint32_t)item_count * BTRFS_ITEM_SIZE;
            if (data_offset > data_area_size ||
                (data_size != 0 &&
                 (data_offset < data_area_start ||
                  data_size > data_area_size - data_offset))) {
                return LOOS_BTRFS_ECORRUPT;
            }
            if (compare_keys(&key, lower) < 0) {
                continue;
            }
            if (compare_keys(&key, upper) > 0) {
                break;
            }
            struct btrfs_item_view view = {
                .key = key,
                .data = block + BTRFS_HEADER_SIZE + data_offset,
                .size = data_size,
            };
            status = callback(mount, &view, opaque);
            if (status != LOOS_BTRFS_OK) {
                return status;
            }
        }
        return LOOS_BTRFS_OK;
    }

    struct btrfs_key previous_child_key = {0, 0, 0};
    for (uint32_t i = 0; i < item_count; ++i) {
        const size_t pointer_offset = BTRFS_HEADER_SIZE +
                                      (size_t)i * BTRFS_KEY_PTR_SIZE;
        const uint8_t *pointer = block + pointer_offset;
        const struct btrfs_key child_key = disk_key(pointer);
        if (i != 0 && compare_keys(&previous_child_key, &child_key) >= 0) {
            return LOOS_BTRFS_ECORRUPT;
        }
        previous_child_key = child_key;
        const struct btrfs_key *next_key = NULL;
        struct btrfs_key next;
        if (i + 1U < item_count) {
            next = disk_key(pointer + BTRFS_KEY_PTR_SIZE);
            next_key = &next;
        }

        if (compare_keys(&child_key, upper) > 0) {
            break;
        }
        if (next_key != NULL && compare_keys(next_key, lower) < 0) {
            continue;
        }

        const uint64_t child_logical = read_le64(pointer + BTRFS_DISK_KEY_SIZE);
        status = scan_tree_range(mount, child_logical, (uint8_t)(level - 1U),
                                 owner, lower, upper, depth + 1U, callback,
                                 opaque);
        if (status != LOOS_BTRFS_OK) {
            return status;
        }
    }
    return LOOS_BTRFS_OK;
}

static int scan_all_tree(struct loos_btrfs_mount *mount,
                         uint64_t logical,
                         uint8_t level,
                         uint64_t owner,
                         btrfs_item_fn callback,
                         void *opaque)
{
    const struct btrfs_key lower = {0, 0, 0};
    const struct btrfs_key upper = {UINT64_MAX, UINT8_MAX, UINT64_MAX};
    return scan_tree_range(mount, logical, level, owner, &lower, &upper, 0,
                           callback, opaque);
}

static int parse_system_chunks(struct loos_btrfs_mount *mount,
                               const uint8_t *superblock,
                               uint32_t array_size)
{
    if (array_size == 0 || array_size > BTRFS_SYSTEM_CHUNK_ARRAY_SIZE) {
        return LOOS_BTRFS_ECORRUPT;
    }

    size_t cursor = 0;
    const uint8_t *array = superblock + BTRFS_SUPER_SYS_CHUNK_ARRAY;
    while (cursor < array_size) {
        if (array_size - cursor < BTRFS_DISK_KEY_SIZE) {
            return LOOS_BTRFS_ECORRUPT;
        }
        const struct btrfs_key key = disk_key(array + cursor);
        cursor += BTRFS_DISK_KEY_SIZE;
        if (key.type != BTRFS_CHUNK_ITEM_KEY) {
            return LOOS_BTRFS_ECORRUPT;
        }
        if (array_size - cursor < 48U) {
            return LOOS_BTRFS_ECORRUPT;
        }
        const uint16_t num_stripes = read_le16(array + cursor + 44U);
        size_t item_size;
        if (!checked_mul_size((size_t)num_stripes, 32U, &item_size) ||
            item_size > SIZE_MAX - 48U) {
            return LOOS_BTRFS_ECORRUPT;
        }
        item_size += 48U;
        if (item_size > array_size - cursor) {
            return LOOS_BTRFS_ECORRUPT;
        }
        int status = parse_chunk_item(mount, key.offset, array + cursor,
                                      item_size);
        if (status != LOOS_BTRFS_OK) {
            return status;
        }
        cursor += item_size;
    }
    return cursor == array_size ? LOOS_BTRFS_OK : LOOS_BTRFS_ECORRUPT;
}

struct chunk_scan_context {
    bool saw_chunk;
};

static int collect_chunk_item(struct loos_btrfs_mount *mount,
                              const struct btrfs_item_view *item,
                              void *opaque)
{
    if (item->key.type != BTRFS_CHUNK_ITEM_KEY) {
        return LOOS_BTRFS_OK;
    }
    struct chunk_scan_context *context = opaque;
    context->saw_chunk = true;
    return parse_chunk_item(mount, item->key.offset, item->data, item->size);
}

struct root_item_result {
    bool found;
    uint64_t bytenr;
    uint64_t root_dirid;
    uint8_t level;
};

static int collect_root_item(struct loos_btrfs_mount *mount,
                             const struct btrfs_item_view *item,
                             void *opaque)
{
    (void)mount;
    struct root_item_result *result = opaque;
    if (item->key.type != BTRFS_ROOT_ITEM_KEY) {
        return LOOS_BTRFS_OK;
    }
    if (item->size <= BTRFS_ROOT_ITEM_LEVEL ||
        item->size < BTRFS_ROOT_ITEM_BYTENR + 8U) {
        return LOOS_BTRFS_ECORRUPT;
    }
    result->found = true;
    result->bytenr = read_le64(item->data + BTRFS_ROOT_ITEM_BYTENR);
    result->root_dirid = read_le64(item->data + BTRFS_ROOT_ITEM_ROOT_DIRID);
    result->level = item->data[BTRFS_ROOT_ITEM_LEVEL];
    return LOOS_BTRFS_STOP;
}

static int find_root_item(struct loos_btrfs_mount *mount,
                          uint64_t root_objectid,
                          struct root_item_result *result)
{
    memset(result, 0, sizeof(*result));
    const struct btrfs_key key = {
        .objectid = root_objectid,
        .type = BTRFS_ROOT_ITEM_KEY,
        .offset = 0,
    };
    int status = scan_tree_range(mount, mount->root_tree,
                                 mount->root_tree_level,
                                 BTRFS_ROOT_TREE_OBJECTID, &key, &key, 0,
                                 collect_root_item, result);
    if (status == LOOS_BTRFS_STOP) {
        status = LOOS_BTRFS_OK;
    }
    if (status == LOOS_BTRFS_OK && !result->found) {
        return LOOS_BTRFS_ENOTFOUND;
    }
    if (status == LOOS_BTRFS_OK && result->bytenr == 0) {
        return LOOS_BTRFS_ECORRUPT;
    }
    if (status == LOOS_BTRFS_OK && result->level > LOOS_BTRFS_MAX_TREE_LEVEL) {
        return LOOS_BTRFS_ECORRUPT;
    }
    return status;
}

int loos_btrfs_mount(struct loos_btrfs_mount *mount,
                     const struct loos_btrfs_io *io,
                     void *node_storage,
                     size_t node_storage_size,
                     bool verify_checksums)
{
    uint8_t superblock[LOOS_BTRFS_SUPERBLOCK_SIZE];
    if (mount == NULL || io == NULL || io->read_at == NULL ||
        node_storage == NULL ||
        node_storage_size < LOOS_BTRFS_NODE_STORAGE_SIZE) {
        return LOOS_BTRFS_EINVAL;
    }
    memset(mount, 0, sizeof(*mount));
    mount->io = *io;
    mount->node_storage = node_storage;
    mount->node_storage_size = node_storage_size;
    mount->verify_checksums = verify_checksums;

    int status = read_physical(mount, LOOS_BTRFS_SUPERBLOCK_OFFSET,
                               superblock, sizeof(superblock));
    if (status != LOOS_BTRFS_OK) {
        return status;
    }
    memcpy(mount->fsid, superblock + BTRFS_SUPER_FSID, sizeof(mount->fsid));
    memcpy(mount->metadata_uuid, superblock + BTRFS_SUPER_METADATA_UUID,
           sizeof(mount->metadata_uuid));
    bool metadata_uuid_is_zero = true;
    for (size_t i = 0; i < sizeof(mount->metadata_uuid); ++i) {
        metadata_uuid_is_zero &= mount->metadata_uuid[i] == 0;
    }
    if (metadata_uuid_is_zero) {
        memcpy(mount->metadata_uuid, mount->fsid, sizeof(mount->metadata_uuid));
    }

    mount->csum_type = read_le16(superblock + BTRFS_SUPER_CSUM_TYPE);
    if (verify_checksums && mount->csum_type != BTRFS_CSUM_TYPE_CRC32) {
        return LOOS_BTRFS_ENOTSUP;
    }
    status = verify_checksum(mount, superblock, sizeof(superblock));
    if (status != LOOS_BTRFS_OK) {
        return status;
    }
    if (read_le64(superblock + BTRFS_SUPER_MAGIC) != BTRFS_MAGIC ||
        read_le64(superblock + BTRFS_SUPER_BYTENR) !=
            LOOS_BTRFS_SUPERBLOCK_OFFSET) {
        return LOOS_BTRFS_EBADFS;
    }

    mount->generation = read_le64(superblock + BTRFS_SUPER_GENERATION);
    mount->total_bytes = read_le64(superblock + BTRFS_SUPER_TOTAL_BYTES);
    mount->sectorsize = read_le32(superblock + BTRFS_SUPER_SECTORSIZE);
    mount->nodesize = read_le32(superblock + BTRFS_SUPER_NODESIZE);
    mount->root_tree = read_le64(superblock + BTRFS_SUPER_ROOT);
    mount->chunk_tree = read_le64(superblock + BTRFS_SUPER_CHUNK_ROOT);
    mount->root_tree_level = superblock[BTRFS_SUPER_ROOT_LEVEL];
    mount->chunk_tree_level = superblock[BTRFS_SUPER_CHUNK_ROOT_LEVEL];
    const uint32_t sys_array_size =
        read_le32(superblock + BTRFS_SUPER_SYS_CHUNK_ARRAY_SIZE);
    if (!is_power_of_two(mount->sectorsize) ||
        !is_power_of_two(mount->nodesize) || mount->sectorsize < 4096U ||
        mount->nodesize < mount->sectorsize ||
        mount->nodesize > LOOS_BTRFS_MAX_NODE_SIZE ||
        mount->root_tree == 0 || mount->chunk_tree == 0 ||
        mount->root_tree_level > LOOS_BTRFS_MAX_TREE_LEVEL ||
        mount->chunk_tree_level > LOOS_BTRFS_MAX_TREE_LEVEL) {
        return LOOS_BTRFS_EBADFS;
    }

    size_t required_storage;
    if (!checked_mul_size((size_t)mount->nodesize,
                          LOOS_BTRFS_MAX_TREE_LEVEL + 1U,
                          &required_storage) ||
        required_storage > node_storage_size) {
        return LOOS_BTRFS_EINVAL;
    }
    status = parse_system_chunks(mount, superblock, sys_array_size);
    if (status != LOOS_BTRFS_OK) {
        return status;
    }

    struct chunk_scan_context chunk_context = {0};
    status = scan_all_tree(mount, mount->chunk_tree,
                           mount->chunk_tree_level,
                           BTRFS_CHUNK_TREE_OBJECTID, collect_chunk_item,
                           &chunk_context);
    if (status != LOOS_BTRFS_OK) {
        return status;
    }
    if (!chunk_context.saw_chunk) {
        return LOOS_BTRFS_EBADFS;
    }

    struct root_item_result fs_root;
    status = find_root_item(mount, BTRFS_FS_TREE_OBJECTID, &fs_root);
    if (status != LOOS_BTRFS_OK) {
        return status;
    }
    mount->fs_tree = fs_root.bytenr;
    mount->fs_tree_level = fs_root.level;
    mount->fs_root_dirid = fs_root.root_dirid;
    if (mount->fs_root_dirid == 0) {
        return LOOS_BTRFS_ECORRUPT;
    }

    struct root_item_result csum_root;
    status = find_root_item(mount, BTRFS_CSUM_TREE_OBJECTID, &csum_root);
    if (status == LOOS_BTRFS_OK) {
        mount->csum_tree = csum_root.bytenr;
        mount->csum_tree_level = csum_root.level;
        mount->has_csum_tree = true;
    } else if (status != LOOS_BTRFS_ENOTFOUND) {
        return status;
    }

    mount->mounted = true;
    return LOOS_BTRFS_OK;
}

void loos_btrfs_unmount(struct loos_btrfs_mount *mount)
{
    if (mount == NULL) {
        return;
    }
    mount->mounted = false;
    mount->chunk_count = 0;
}

struct inode_result {
    bool found;
    uint64_t size;
    uint64_t flags;
    uint32_t mode;
    uint32_t uid;
    uint32_t gid;
    uint32_t nlink;
};

static int collect_inode_item(struct loos_btrfs_mount *mount,
                              const struct btrfs_item_view *item,
                              void *opaque)
{
    (void)mount;
    struct inode_result *result = opaque;
    if (item->key.type != BTRFS_INODE_ITEM_KEY) {
        return LOOS_BTRFS_OK;
    }
    if (item->size < BTRFS_INODE_ITEM_MIN_SIZE) {
        return LOOS_BTRFS_ECORRUPT;
    }
    result->found = true;
    result->size = read_le64(item->data + BTRFS_INODE_SIZE);
    result->mode = read_le32(item->data + BTRFS_INODE_MODE);
    result->flags = read_le64(item->data + BTRFS_INODE_FLAGS);
    result->uid = read_le32(item->data + BTRFS_INODE_UID);
    result->gid = read_le32(item->data + BTRFS_INODE_GID);
    result->nlink = read_le32(item->data + BTRFS_INODE_NLINK);
    return LOOS_BTRFS_STOP;
}

static int find_inode(struct loos_btrfs_mount *mount,
                      uint64_t inode,
                      struct inode_result *result)
{
    memset(result, 0, sizeof(*result));
    const struct btrfs_key key = {
        .objectid = inode,
        .type = BTRFS_INODE_ITEM_KEY,
        .offset = 0,
    };
    int status = scan_tree_range(mount, mount->fs_tree, mount->fs_tree_level,
                                 BTRFS_FS_TREE_OBJECTID, &key, &key, 0,
                                 collect_inode_item, result);
    if (status == LOOS_BTRFS_STOP) {
        status = LOOS_BTRFS_OK;
    }
    if (status == LOOS_BTRFS_OK && !result->found) {
        return LOOS_BTRFS_ENOTFOUND;
    }
    return status;
}

static bool mode_is_directory(uint32_t mode)
{
    return (mode & UINT32_C(0170000)) == UINT32_C(0040000);
}

static bool mode_is_regular(uint32_t mode)
{
    return (mode & UINT32_C(0170000)) == UINT32_C(0100000);
}

struct dir_lookup_context {
    const char *name;
    size_t name_length;
    struct loos_btrfs_dirent result;
    bool found;
    loos_btrfs_dirent_fn callback;
    void *callback_opaque;
    loos_btrfs_readdir_fn readdir;
    uint64_t readdir_cookie;
    uint64_t readdir_start;
};

static int parse_dir_items(const struct btrfs_item_view *item,
                           struct dir_lookup_context *context,
                           loos_btrfs_dirent_fn callback,
                           void *callback_opaque)
{
    size_t cursor = 0;
    while (cursor < item->size) {
        if (item->size - cursor < BTRFS_DIR_ITEM_HEADER_SIZE) {
            return LOOS_BTRFS_ECORRUPT;
        }
        const uint8_t *entry = item->data + cursor;
        const uint16_t data_length = read_le16(entry + BTRFS_DIR_ITEM_DATA_LEN);
        const uint16_t name_length = read_le16(entry + BTRFS_DIR_ITEM_NAME_LEN);
        size_t entry_size = BTRFS_DIR_ITEM_HEADER_SIZE;
        if (entry_size > SIZE_MAX - (size_t)data_length ||
            (entry_size += data_length) > SIZE_MAX - (size_t)name_length ||
            (entry_size += name_length) > item->size - cursor) {
            return LOOS_BTRFS_ECORRUPT;
        }
        if (name_length > LOOS_BTRFS_MAX_NAME) {
            return LOOS_BTRFS_ECORRUPT;
        }

        const char *name = (const char *)(entry + BTRFS_DIR_ITEM_HEADER_SIZE +
                                          data_length);
        struct loos_btrfs_dirent dirent;
        memset(&dirent, 0, sizeof(dirent));
        dirent.inode = read_le64(entry + BTRFS_DIR_ITEM_LOCATION);
        if ((entry[BTRFS_DIR_ITEM_TYPE] & UINT8_C(0x80)) != 0) {
            return LOOS_BTRFS_ENOTSUP;
        }
        dirent.type = entry[BTRFS_DIR_ITEM_TYPE];
        dirent.name_length = name_length;
        memcpy(dirent.name, name, name_length);
        dirent.name[name_length] = '\0';

        if (context != NULL && context->name != NULL &&
            context->name_length == name_length &&
            memcmp(context->name, name, name_length) == 0) {
            context->result = dirent;
            context->found = true;
            return LOOS_BTRFS_STOP;
        }
        if (callback != NULL) {
            const int callback_status = callback(callback_opaque, &dirent);
            if (callback_status != LOOS_BTRFS_OK) {
                return callback_status;
            }
        }
        if (context != NULL && context->readdir != NULL) {
            const uint64_t cookie = context->readdir_cookie++;
            if (cookie >= context->readdir_start) {
                const int callback_status = context->readdir(
                    context->callback_opaque, cookie, &dirent);
                if (callback_status != LOOS_BTRFS_OK) {
                    return callback_status;
                }
            }
        }
        cursor += entry_size;
    }
    return LOOS_BTRFS_OK;
}

static int collect_dir_item(struct loos_btrfs_mount *mount,
                            const struct btrfs_item_view *item,
                            void *opaque)
{
    (void)mount;
    struct dir_lookup_context *context = opaque;
    return parse_dir_items(item, context, NULL, NULL);
}

static int collect_dirent(struct loos_btrfs_mount *mount,
                          const struct btrfs_item_view *item,
                          void *opaque)
{
    (void)mount;
    struct dir_lookup_context *context = opaque;
    return parse_dir_items(item, NULL, context->callback, context->callback_opaque);
}

static int collect_readdir(struct loos_btrfs_mount *mount,
                           const struct btrfs_item_view *item,
                           void *opaque)
{
    (void)mount;
    struct dir_lookup_context *context = opaque;
    return parse_dir_items(item, context, NULL, NULL);
}

static int find_dirent_in_range(struct loos_btrfs_mount *mount,
                                uint64_t directory,
                                const struct btrfs_key *lower,
                                const struct btrfs_key *upper,
                                struct dir_lookup_context *context)
{
    int status = scan_tree_range(mount, mount->fs_tree, mount->fs_tree_level,
                                 BTRFS_FS_TREE_OBJECTID, lower, upper, 0,
                                 collect_dir_item, context);
    if (status == LOOS_BTRFS_STOP) {
        return LOOS_BTRFS_OK;
    }
    (void)directory;
    return status;
}

static int find_dirent(struct loos_btrfs_mount *mount,
                       uint64_t directory,
                       const char *name,
                       size_t name_length,
                       struct loos_btrfs_dirent *result)
{
    if (name == NULL || result == NULL || name_length == 0 ||
        name_length > LOOS_BTRFS_MAX_NAME) {
        return LOOS_BTRFS_EINVAL;
    }
    struct dir_lookup_context context = {
        .name = name,
        .name_length = name_length,
    };
    const uint64_t hash = crc32c_raw(~UINT32_C(1), (const uint8_t *)name,
                                     name_length);
    const struct btrfs_key hashed = {
        .objectid = directory,
        .type = BTRFS_DIR_ITEM_KEY,
        .offset = hash,
    };
    int status = find_dirent_in_range(mount, directory, &hashed, &hashed,
                                      &context);
    if (status != LOOS_BTRFS_OK) {
        return status;
    }

    /* Hash collisions are stored in one item, but a reader should also be
     * tolerant of older metadata that used a different name-hash seed. */
    if (!context.found) {
        const struct btrfs_key lower = {
            .objectid = directory,
            .type = BTRFS_DIR_ITEM_KEY,
            .offset = 0,
        };
        const struct btrfs_key upper = {
            .objectid = directory,
            .type = BTRFS_DIR_ITEM_KEY,
            .offset = UINT64_MAX,
        };
        status = find_dirent_in_range(mount, directory, &lower, &upper,
                                       &context);
        if (status != LOOS_BTRFS_OK) {
            return status;
        }
    }
    if (!context.found) {
        return LOOS_BTRFS_ENOTFOUND;
    }
    *result = context.result;
    return LOOS_BTRFS_OK;
}

static int next_path_component(const char **cursor,
                               char *component,
                               size_t *component_length)
{
    const char *path = *cursor;
    while (*path == '/') {
        ++path;
    }
    if (*path == '\0') {
        *cursor = path;
        *component_length = 0;
        return LOOS_BTRFS_OK;
    }

    const char *start = path;
    while (*path != '\0' && *path != '/') {
        ++path;
    }
    const size_t length = (size_t)(path - start);
    if (length == 0 || length > LOOS_BTRFS_MAX_NAME) {
        return LOOS_BTRFS_EINVAL;
    }
    memcpy(component, start, length);
    component[length] = '\0';
    *cursor = path;
    *component_length = length;
    return LOOS_BTRFS_OK;
}

static int resolve_path(struct loos_btrfs_mount *mount,
                        const char *path,
                        uint64_t *inode_out,
                        struct inode_result *inode_result_out)
{
    if (mount == NULL || !mount->mounted || path == NULL || path[0] != '/' ||
        inode_out == NULL || inode_result_out == NULL) {
        return LOOS_BTRFS_EINVAL;
    }

    uint64_t inode = mount->fs_root_dirid;
    struct inode_result inode_result;
    int status = find_inode(mount, inode, &inode_result);
    if (status != LOOS_BTRFS_OK) {
        return status;
    }
    const char *cursor = path;
    char component[LOOS_BTRFS_MAX_NAME + 1U];
    size_t component_length;
    while (true) {
        status = next_path_component(&cursor, component, &component_length);
        if (status != LOOS_BTRFS_OK) {
            return status;
        }
        if (component_length == 0) {
            break;
        }
        if (!mode_is_directory(inode_result.mode)) {
            return LOOS_BTRFS_ENOTDIR;
        }
        struct loos_btrfs_dirent entry;
        status = find_dirent(mount, inode, component, component_length, &entry);
        if (status != LOOS_BTRFS_OK) {
            return status;
        }
        inode = entry.inode;
        status = find_inode(mount, inode, &inode_result);
        if (status != LOOS_BTRFS_OK) {
            return status;
        }
    }
    *inode_out = inode;
    *inode_result_out = inode_result;
    return LOOS_BTRFS_OK;
}

int loos_btrfs_list_dir(struct loos_btrfs_mount *mount,
                        const char *path,
                        loos_btrfs_dirent_fn callback,
                        void *opaque)
{
    if (callback == NULL) {
        return LOOS_BTRFS_EINVAL;
    }
    uint64_t inode;
    struct inode_result inode_result;
    int status = resolve_path(mount, path, &inode, &inode_result);
    if (status != LOOS_BTRFS_OK) {
        return status;
    }
    if (!mode_is_directory(inode_result.mode)) {
        return LOOS_BTRFS_ENOTDIR;
    }

    const struct btrfs_key lower = {
        .objectid = inode,
        .type = BTRFS_DIR_ITEM_KEY,
        .offset = 0,
    };
    const struct btrfs_key upper = {
        .objectid = inode,
        .type = BTRFS_DIR_ITEM_KEY,
        .offset = UINT64_MAX,
    };
    struct dir_lookup_context context = {
        .callback = callback,
        .callback_opaque = opaque,
    };
    status = scan_tree_range(mount, mount->fs_tree, mount->fs_tree_level,
                             BTRFS_FS_TREE_OBJECTID, &lower, &upper, 0,
                             collect_dirent, &context);
    return status == LOOS_BTRFS_STOP ? LOOS_BTRFS_OK : status;
}

struct file_read_context {
    struct loos_btrfs_mount *mount;
    uint64_t request_start;
    uint64_t request_end;
    uint8_t *destination;
};

static int collect_file_extent(struct loos_btrfs_mount *mount,
                               const struct btrfs_item_view *item,
                               void *opaque);

static void fill_stat(struct loos_btrfs_stat *stat,
                      uint64_t inode,
                      const struct inode_result *result)
{
    stat->inode = inode;
    stat->size = result->size;
    stat->flags = result->flags;
    stat->mode = result->mode;
    stat->uid = result->uid;
    stat->gid = result->gid;
    stat->nlink = result->nlink;
}

int loos_btrfs_lookup(struct loos_btrfs_mount *mount,
                      uint64_t directory,
                      const char *name,
                      struct loos_btrfs_dirent *entry,
                      struct loos_btrfs_stat *stat)
{
    if (mount == NULL || !mount->mounted || name == NULL || entry == NULL ||
        stat == NULL || name[0] == '\0' || strlen(name) > LOOS_BTRFS_MAX_NAME) {
        return LOOS_BTRFS_EINVAL;
    }
    struct inode_result directory_result;
    int status = find_inode(mount, directory, &directory_result);
    if (status != LOOS_BTRFS_OK) {
        return status;
    }
    if (!mode_is_directory(directory_result.mode)) {
        return LOOS_BTRFS_ENOTDIR;
    }
    status = find_dirent(mount, directory, name, strlen(name), entry);
    if (status != LOOS_BTRFS_OK) {
        return status;
    }
    struct inode_result child_result;
    status = find_inode(mount, entry->inode, &child_result);
    if (status != LOOS_BTRFS_OK) {
        return status;
    }
    fill_stat(stat, entry->inode, &child_result);
    return LOOS_BTRFS_OK;
}

int loos_btrfs_stat_path(struct loos_btrfs_mount *mount,
                         const char *path,
                         struct loos_btrfs_stat *stat)
{
    if (stat == NULL) {
        return LOOS_BTRFS_EINVAL;
    }
    uint64_t inode;
    struct inode_result result;
    int status = resolve_path(mount, path, &inode, &result);
    if (status != LOOS_BTRFS_OK) {
        return status;
    }
    fill_stat(stat, inode, &result);
    return LOOS_BTRFS_OK;
}

int loos_btrfs_readdir(struct loos_btrfs_mount *mount,
                       uint64_t directory,
                       uint64_t *cookie,
                       loos_btrfs_readdir_fn callback,
                       void *opaque)
{
    if (mount == NULL || !mount->mounted || cookie == NULL ||
        callback == NULL) {
        return LOOS_BTRFS_EINVAL;
    }
    struct inode_result directory_result;
    int status = find_inode(mount, directory, &directory_result);
    if (status != LOOS_BTRFS_OK) {
        return status;
    }
    if (!mode_is_directory(directory_result.mode)) {
        return LOOS_BTRFS_ENOTDIR;
    }
    const struct btrfs_key lower = {
        .objectid = directory,
        .type = BTRFS_DIR_ITEM_KEY,
        .offset = 0,
    };
    const struct btrfs_key upper = {
        .objectid = directory,
        .type = BTRFS_DIR_ITEM_KEY,
        .offset = UINT64_MAX,
    };
    struct dir_lookup_context context = {
        .readdir = callback,
        .callback_opaque = opaque,
        .readdir_start = *cookie,
    };
    status = scan_tree_range(mount, mount->fs_tree, mount->fs_tree_level,
                             BTRFS_FS_TREE_OBJECTID, &lower, &upper, 0,
                             collect_readdir, &context);
    *cookie = context.readdir_cookie;
    if (status == LOOS_BTRFS_STOP) {
        return LOOS_BTRFS_OK;
    }
    return status;
}

int loos_btrfs_open_inode(struct loos_btrfs_mount *mount,
                          uint64_t inode,
                          struct loos_btrfs_file *file)
{
    if (file == NULL || mount == NULL || !mount->mounted) {
        return LOOS_BTRFS_EINVAL;
    }
    struct inode_result result;
    int status = find_inode(mount, inode, &result);
    if (status != LOOS_BTRFS_OK) {
        return status;
    }
    fill_stat(&file->stat, inode, &result);
    file->mount = mount;
    return LOOS_BTRFS_OK;
}

int loos_btrfs_read_inode(struct loos_btrfs_file *file,
                          uint64_t offset,
                          void *buffer,
                          size_t length,
                          size_t *bytes_read)
{
    if (file == NULL || file->mount == NULL || !file->mount->mounted ||
        bytes_read == NULL || (length != 0 && buffer == NULL)) {
        return LOOS_BTRFS_EINVAL;
    }
    if (!mode_is_regular(file->stat.mode)) {
        return mode_is_directory(file->stat.mode) ? LOOS_BTRFS_EISDIR
                                                   : LOOS_BTRFS_ENOTSUP;
    }
    if (offset >= file->stat.size || length == 0) {
        *bytes_read = 0;
        return LOOS_BTRFS_OK;
    }
    uint64_t end;
    if (!checked_add_u64(offset, (uint64_t)length, &end) ||
        end > file->stat.size) {
        end = file->stat.size;
    }
    if (end <= offset || end - offset > SIZE_MAX) {
        return LOOS_BTRFS_EOVERFLOW;
    }
    const size_t requested = (size_t)(end - offset);
    memset(buffer, 0, requested);
    const struct btrfs_key lower = {
        .objectid = file->stat.inode,
        .type = BTRFS_EXTENT_DATA_KEY,
        .offset = 0,
    };
    const struct btrfs_key upper = {
        .objectid = file->stat.inode,
        .type = BTRFS_EXTENT_DATA_KEY,
        .offset = UINT64_MAX,
    };
    struct file_read_context context = {
        .mount = file->mount,
        .request_start = offset,
        .request_end = end,
        .destination = buffer,
    };
    int status = scan_tree_range(file->mount, file->mount->fs_tree,
                                 file->mount->fs_tree_level,
                                 BTRFS_FS_TREE_OBJECTID, &lower, &upper, 0,
                                 collect_file_extent, &context);
    if (status != LOOS_BTRFS_OK) {
        return status;
    }
    *bytes_read = requested;
    return LOOS_BTRFS_OK;
}

int loos_btrfs_get_xattr(struct loos_btrfs_mount *mount,
                         uint64_t inode,
                         const char *name,
                         void *value,
                         size_t capacity,
                         size_t *value_length)
{
    (void)mount;
    (void)inode;
    (void)name;
    (void)value;
    (void)capacity;
    if (value_length != NULL) {
        *value_length = 0;
    }
    return LOOS_BTRFS_ENOTSUP;
}

int loos_btrfs_set_xattr(struct loos_btrfs_mount *mount,
                         uint64_t inode,
                         const char *name,
                         const void *value,
                         size_t value_length)
{
    (void)mount;
    (void)inode;
    (void)name;
    (void)value;
    (void)value_length;
    return LOOS_BTRFS_EROFS;
}

static int copy_file_extent(struct file_read_context *context,
                            const struct btrfs_item_view *item)
{
    if (item->size < BTRFS_FILE_EXTENT_INLINE_DATA) {
        return LOOS_BTRFS_ECORRUPT;
    }
    const uint8_t extent_type = item->data[BTRFS_FILE_EXTENT_TYPE];

    uint64_t extent_length;
    uint64_t disk_bytenr = 0;
    uint64_t disk_num_bytes = 0;
    uint64_t extent_offset = 0;
    if (extent_type == BTRFS_FILE_EXTENT_INLINE) {
        /* Inline items end at the start of disk_bytenr; the remaining bytes
         * are data, so ram_bytes is the authoritative file length. */
        extent_length = read_le64(item->data + 8U);
        const size_t inline_length = item->size - BTRFS_FILE_EXTENT_INLINE_DATA;
        if (extent_length > inline_length) {
            return LOOS_BTRFS_ECORRUPT;
        }
    } else if (extent_type == BTRFS_FILE_EXTENT_REG) {
        if (item->size < BTRFS_FILE_EXTENT_MIN_SIZE) {
            return LOOS_BTRFS_ECORRUPT;
        }
        disk_bytenr = read_le64(item->data + BTRFS_FILE_EXTENT_DISK_BYTENR);
        disk_num_bytes =
            read_le64(item->data + BTRFS_FILE_EXTENT_DISK_NUM_BYTES);
        extent_offset = read_le64(item->data + BTRFS_FILE_EXTENT_OFFSET);
        extent_length = read_le64(item->data + BTRFS_FILE_EXTENT_NUM_BYTES);
        if (disk_num_bytes == 0 && extent_length != 0) {
            return LOOS_BTRFS_ECORRUPT;
        }
        if (extent_offset > disk_num_bytes ||
            extent_length > disk_num_bytes - extent_offset) {
            return LOOS_BTRFS_ECORRUPT;
        }
    } else if (extent_type == BTRFS_FILE_EXTENT_PREALLOC) {
        if (item->size < BTRFS_FILE_EXTENT_MIN_SIZE) {
            return LOOS_BTRFS_ECORRUPT;
        }
        extent_length = read_le64(item->data + BTRFS_FILE_EXTENT_NUM_BYTES);
    } else {
        return LOOS_BTRFS_ENOTSUP;
    }

    uint64_t extent_end;
    if (!checked_add_u64(item->key.offset, extent_length, &extent_end)) {
        return LOOS_BTRFS_ECORRUPT;
    }
    const uint64_t start = item->key.offset > context->request_start
                               ? item->key.offset
                               : context->request_start;
    const uint64_t end = extent_end < context->request_end
                             ? extent_end
                             : context->request_end;
    if (start >= end) {
        return LOOS_BTRFS_OK;
    }
    if (item->data[BTRFS_FILE_EXTENT_COMPRESSION] != 0 ||
        item->data[BTRFS_FILE_EXTENT_ENCRYPTION] != 0) {
        return LOOS_BTRFS_ENOTSUP;
    }

    const uint64_t extent_relative = start - item->key.offset;
    const uint64_t output_relative = start - context->request_start;
    const uint64_t copy_length_u64 = end - start;
    const uint64_t request_length =
        context->request_end - context->request_start;
    if (!fits_size_t(extent_relative) || !fits_size_t(output_relative) ||
        !fits_size_t(copy_length_u64) || output_relative > request_length ||
        copy_length_u64 > request_length - output_relative) {
        return LOOS_BTRFS_EOVERFLOW;
    }
    const size_t output_relative_size = (size_t)output_relative;
    const size_t copy_length = (size_t)copy_length_u64;

    if (extent_type == BTRFS_FILE_EXTENT_INLINE) {
        const size_t inline_length = item->size - BTRFS_FILE_EXTENT_INLINE_DATA;
        if (extent_relative > inline_length ||
            copy_length > inline_length - (size_t)extent_relative) {
            return LOOS_BTRFS_ECORRUPT;
        }
        memcpy(context->destination + output_relative_size,
               item->data + BTRFS_FILE_EXTENT_INLINE_DATA +
                   (size_t)extent_relative,
               copy_length);
        return LOOS_BTRFS_OK;
    }
    if (extent_type == BTRFS_FILE_EXTENT_PREALLOC) {
        return LOOS_BTRFS_OK;
    }
    if (disk_bytenr == 0) {
        /* A regular extent with no backing address is a hole. */
        return LOOS_BTRFS_OK;
    }

    uint64_t disk_offset;
    if (!checked_add_u64(disk_bytenr, extent_offset, &disk_offset) ||
        !checked_add_u64(disk_offset, extent_relative, &disk_offset)) {
        return LOOS_BTRFS_EOVERFLOW;
    }
    return read_logical(context->mount, disk_offset,
                        context->destination + output_relative_size,
                        copy_length);
}

static int collect_file_extent(struct loos_btrfs_mount *mount,
                               const struct btrfs_item_view *item,
                               void *opaque)
{
    (void)mount;
    if (item->key.type != BTRFS_EXTENT_DATA_KEY) {
        return LOOS_BTRFS_OK;
    }
    struct file_read_context *context = opaque;
    return copy_file_extent(context, item);
}

int loos_btrfs_read_file(struct loos_btrfs_mount *mount,
                         const char *path,
                         uint64_t offset,
                         void *buffer,
                         size_t length,
                         size_t *bytes_read)
{
    if (bytes_read == NULL || (length != 0 && buffer == NULL)) {
        return LOOS_BTRFS_EINVAL;
    }
    *bytes_read = 0;

    uint64_t inode;
    struct inode_result inode_result;
    int status = resolve_path(mount, path, &inode, &inode_result);
    if (status != LOOS_BTRFS_OK) {
        return status;
    }
    if (!mode_is_regular(inode_result.mode)) {
        return mode_is_directory(inode_result.mode) ? LOOS_BTRFS_EISDIR
                                                     : LOOS_BTRFS_ENOTSUP;
    }
    if (offset >= inode_result.size || length == 0) {
        return LOOS_BTRFS_OK;
    }

    uint64_t request_end;
    if (!checked_add_u64(offset, (uint64_t)length, &request_end)) {
        request_end = UINT64_MAX;
    }
    if (request_end > inode_result.size) {
        request_end = inode_result.size;
    }
    if (request_end <= offset || request_end - offset > SIZE_MAX) {
        return LOOS_BTRFS_EOVERFLOW;
    }
    const size_t request_length = (size_t)(request_end - offset);
    memset(buffer, 0, request_length);

    const struct btrfs_key lower = {
        .objectid = inode,
        .type = BTRFS_EXTENT_DATA_KEY,
        .offset = 0,
    };
    const struct btrfs_key upper = {
        .objectid = inode,
        .type = BTRFS_EXTENT_DATA_KEY,
        .offset = UINT64_MAX,
    };
    struct file_read_context context = {
        .mount = mount,
        .request_start = offset,
        .request_end = request_end,
        .destination = buffer,
    };
    status = scan_tree_range(mount, mount->fs_tree, mount->fs_tree_level,
                             BTRFS_FS_TREE_OBJECTID, &lower, &upper, 0,
                             collect_file_extent, &context);
    if (status != LOOS_BTRFS_OK) {
        return status;
    }
    *bytes_read = request_length;
    return LOOS_BTRFS_OK;
}

static bool transaction_has_block(
    const struct loos_btrfs_write_transaction *transaction,
    uint64_t logical)
{
    for (size_t i = 0; i < transaction->new_block_count; ++i) {
        if (transaction->new_blocks[i] == logical) {
            return true;
        }
    }
    return false;
}

int loos_btrfs_write_transaction_begin(
    struct loos_btrfs_write_transaction *transaction,
    struct loos_btrfs_mount *mount,
    const struct loos_btrfs_allocator *allocator,
    void *storage,
    size_t storage_size)
{
    if (transaction == NULL || mount == NULL || !mount->mounted ||
        allocator == NULL || allocator->allocate == NULL || storage == NULL ||
        mount->io.write_at == NULL || mount->io.flush == NULL ||
        mount->csum_type != BTRFS_CSUM_TYPE_CRC32) {
        return LOOS_BTRFS_EINVAL;
    }
    size_t needed = (size_t)mount->nodesize + LOOS_BTRFS_SUPERBLOCK_SIZE;
    if (needed < (size_t)mount->nodesize || storage_size < needed) {
        return LOOS_BTRFS_EINVAL;
    }
    memset(transaction, 0, sizeof(*transaction));
    transaction->mount = mount;
    transaction->allocator = *allocator;
    transaction->storage = storage;
    transaction->storage_size = storage_size;
    transaction->active = true;
    return LOOS_BTRFS_OK;
}

int loos_btrfs_write_transaction_cow_tree(
    struct loos_btrfs_write_transaction *transaction,
    uint64_t old_logical,
    const void *replacement,
    uint64_t *new_logical)
{
    if (transaction == NULL || !transaction->active || replacement == NULL ||
        new_logical == NULL) {
        return LOOS_BTRFS_EINVAL;
    }
    struct loos_btrfs_mount *mount = transaction->mount;
    uint64_t old_physical;
    size_t old_available;
    int status = map_logical(mount, old_logical, mount->nodesize,
                             &old_physical, &old_available);
    if (status != LOOS_BTRFS_OK) {
        return status;
    }
    (void)old_physical;
    (void)old_available;
    if (transaction->new_block_count == LOOS_BTRFS_MAX_TX_BLOCKS) {
        return LOOS_BTRFS_ELIMIT;
    }
    const uint8_t *source = replacement;
    if (read_le64(source + BTRFS_HEADER_BYTENR) != old_logical ||
        memcmp(source + BTRFS_HEADER_FSID, mount->metadata_uuid, 16U) != 0 ||
        source[BTRFS_HEADER_LEVEL] > LOOS_BTRFS_MAX_TREE_LEVEL) {
        return LOOS_BTRFS_ECORRUPT;
    }
    const size_t entry_size = source[BTRFS_HEADER_LEVEL] == 0
                                  ? BTRFS_ITEM_SIZE
                                  : BTRFS_KEY_PTR_SIZE;
    const uint32_t item_count = read_le32(source + BTRFS_HEADER_NRITEMS);
    size_t entry_bytes;
    if (!checked_mul_size(item_count, entry_size, &entry_bytes) ||
        entry_bytes > mount->nodesize - BTRFS_HEADER_SIZE) {
        return LOOS_BTRFS_ECORRUPT;
    }

    uint64_t logical;
    uint64_t physical;
    if (transaction->allocator.allocate(transaction->allocator.opaque,
                                        mount->nodesize, &logical,
                                        &physical) != 0 ||
        logical == old_logical || logical % mount->nodesize != 0 ||
        physical % mount->sectorsize != 0 ||
        transaction_has_block(transaction, logical)) {
        return LOOS_BTRFS_EINVAL;
    }

    memcpy(transaction->storage, replacement, mount->nodesize);
    write_le64(transaction->storage + BTRFS_HEADER_BYTENR, logical);
    memset(transaction->storage, 0, BTRFS_CSUM_SIZE);
    write_le32(transaction->storage,
               crc32c(0, transaction->storage + BTRFS_CSUM_SIZE,
                      mount->nodesize - BTRFS_CSUM_SIZE));
    status = write_physical(mount, physical, transaction->storage,
                            mount->nodesize);
    if (status != LOOS_BTRFS_OK) {
        return status;
    }
    status = flush_device(mount);
    if (status != LOOS_BTRFS_OK) {
        return status;
    }
    transaction->new_blocks[transaction->new_block_count++] = logical;
    *new_logical = logical;
    return LOOS_BTRFS_OK;
}

int loos_btrfs_write_transaction_publish_root(
    struct loos_btrfs_write_transaction *transaction,
    uint64_t new_root,
    uint8_t root_level)
{
    if (transaction == NULL || !transaction->active ||
        root_level > LOOS_BTRFS_MAX_TREE_LEVEL ||
        !transaction_has_block(transaction, new_root)) {
        return LOOS_BTRFS_EINVAL;
    }
    struct loos_btrfs_mount *mount = transaction->mount;
    uint8_t *superblock = transaction->storage + mount->nodesize;
    int status = read_physical(mount, LOOS_BTRFS_SUPERBLOCK_OFFSET,
                               superblock, LOOS_BTRFS_SUPERBLOCK_SIZE);
    if (status != LOOS_BTRFS_OK) {
        return status;
    }
    if (read_le64(superblock + BTRFS_SUPER_MAGIC) != BTRFS_MAGIC ||
        read_le64(superblock + BTRFS_SUPER_BYTENR) !=
            LOOS_BTRFS_SUPERBLOCK_OFFSET) {
        return LOOS_BTRFS_EBADFS;
    }
    const uint64_t generation = read_le64(superblock + BTRFS_SUPER_GENERATION);
    if (generation == UINT64_MAX) {
        return LOOS_BTRFS_EOVERFLOW;
    }
    write_le64(superblock + BTRFS_SUPER_GENERATION, generation + 1U);
    write_le64(superblock + BTRFS_SUPER_ROOT, new_root);
    superblock[BTRFS_SUPER_ROOT_LEVEL] = root_level;
    memset(superblock, 0, BTRFS_CSUM_SIZE);
    write_le32(superblock,
               crc32c(0, superblock + BTRFS_CSUM_SIZE,
                      LOOS_BTRFS_SUPERBLOCK_SIZE - BTRFS_CSUM_SIZE));
    transaction->published_root = new_root;
    transaction->published_generation = generation + 1U;
    transaction->published_level = root_level;
    transaction->root_published = true;
    return LOOS_BTRFS_OK;
}

int loos_btrfs_write_transaction_commit(
    struct loos_btrfs_write_transaction *transaction)
{
    if (transaction == NULL || !transaction->active ||
        !transaction->root_published) {
        return LOOS_BTRFS_EINVAL;
    }
    struct loos_btrfs_mount *mount = transaction->mount;
    uint8_t *superblock = transaction->storage + mount->nodesize;
    int status = write_superblock_mirrors(mount, superblock);
    if (status != LOOS_BTRFS_OK) {
        return status;
    }
    status = write_physical(mount, LOOS_BTRFS_SUPERBLOCK_OFFSET,
                            superblock, LOOS_BTRFS_SUPERBLOCK_SIZE);
    if (status != LOOS_BTRFS_OK) {
        return status;
    }
    status = flush_device(mount);
    if (status != LOOS_BTRFS_OK) {
        return status;
    }
    mount->root_tree = transaction->published_root;
    mount->root_tree_level = transaction->published_level;
    mount->generation = transaction->published_generation;
    transaction->active = false;
    return LOOS_BTRFS_OK;
}

void loos_btrfs_write_transaction_abort(
    struct loos_btrfs_write_transaction *transaction)
{
    if (transaction == NULL) {
        return;
    }
    transaction->active = false;
    transaction->root_published = false;
    transaction->new_block_count = 0;
}
