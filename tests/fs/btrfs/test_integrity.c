#define _GNU_SOURCE 1

#include "integrity.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int failures;

#define CHECK(condition, message)                                                \
    do {                                                                         \
        if (!(condition)) {                                                      \
            printf("FAIL: %s\n", message);                                     \
            failures++;                                                         \
        }                                                                        \
    } while (0)

struct device {
    uint8_t *bytes;
    size_t size;
};

static int device_read(void *opaque, uint64_t offset, void *buffer, size_t length)
{
    struct device *device = opaque;
    if (offset > device->size || length > device->size - (size_t)offset) {
        return -1;
    }
    memcpy(buffer, device->bytes + offset, length);
    return 0;
}

static int device_write(void *opaque,
                        uint64_t offset,
                        const void *buffer,
                        size_t length)
{
    struct device *device = opaque;
    if (offset > device->size || length > device->size - (size_t)offset) {
        return -1;
    }
    memcpy(device->bytes + offset, buffer, length);
    return 0;
}

static int device_flush(void *opaque)
{
    (void)opaque;
    return 0;
}

static int device_write_mkfs(void *opaque,
                             uint64_t offset,
                             const void *buffer,
                             size_t length)
{
    return device_write(opaque, offset, buffer, length);
}

int main(void)
{
    const size_t device_size = 200U << 20;
    struct device device = {
        .bytes = calloc(1, device_size),
        .size = device_size,
    };
    CHECK(device.bytes != NULL, "allocate test device");
    if (device.bytes == NULL) {
        return 1;
    }

    struct loos_btrfs_mkfs_params params;
    memset(&params, 0, sizeof(params));
    params.device_bytes = device_size;
    params.label = "LoOS integrity";
    params.unix_time = 1700000000;
    for (size_t i = 0; i < 16U; ++i) {
        params.fsid[i] = (uint8_t)(i + 1U);
        params.dev_uuid[i] = (uint8_t)(0x40U + i);
        params.chunk_tree_uuid[i] = (uint8_t)(0x80U + i);
        params.subvol_uuid[i] = (uint8_t)(0xc0U + i);
    }
    CHECK(loos_btrfs_mkfs(&params, device_write_mkfs, &device) == 0,
          "format test device");

    struct loos_btrfs_io io = {
        .read_at = device_read,
        .write_at = device_write,
        .flush = device_flush,
        .opaque = &device,
    };
    uint8_t node_storage[LOOS_BTRFS_NODE_STORAGE_SIZE];
    struct loos_btrfs_integrity_report report;
    int status = loos_btrfs_integrity_check(&io, node_storage,
                                            sizeof(node_storage), &report);
    CHECK(status == LOOS_BTRFS_OK, "check a clean image");
    CHECK(report.superblocks_checked == 2U && report.superblocks_valid == 2U,
          "check primary and 64 MiB mirror");
    CHECK(report.chunks_checked == 3U && report.metadata_roots_checked == 3U,
          "check supported chunk and metadata roots");

    device.bytes[0x10000U + 299U] ^= 1U;
    status = loos_btrfs_integrity_check(&io, node_storage,
                                        sizeof(node_storage), &report);
    CHECK(status == LOOS_BTRFS_ECORRUPT &&
              report.first_bad_offset == LOOS_BTRFS_SUPERBLOCK_OFFSET,
          "detect a damaged primary superblock");

    status = loos_btrfs_rescue_restore_primary_superblock(
        &io, UINT64_C(0x4000000), &report);
    CHECK(status == LOOS_BTRFS_OK, "restore primary from 64 MiB mirror");
    status = loos_btrfs_integrity_check(&io, node_storage,
                                        sizeof(node_storage), &report);
    CHECK(status == LOOS_BTRFS_OK, "check restored image");

    device.bytes[0x508000U + 200U] ^= 1U;
    status = loos_btrfs_integrity_check(&io, node_storage,
                                        sizeof(node_storage), &report);
    CHECK(status == LOOS_BTRFS_ECORRUPT,
          "detect a damaged metadata tree block");

    struct loos_btrfs_io read_only_io = {
        .read_at = device_read,
        .write_at = NULL,
        .flush = NULL,
        .opaque = &device,
    };
    status = loos_btrfs_rescue_restore_primary_superblock(
        &read_only_io, UINT64_C(0x4000000), &report);
    CHECK(status == LOOS_BTRFS_EROFS,
          "refuse rescue restore without write and flush callbacks");

    free(device.bytes);
    if (failures == 0) {
        printf("btrfs_integrity: all passed\n");
    }
    return failures != 0;
}
