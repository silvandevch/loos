#include "../../src/io/block.h"
#include "../../src/security/crypto.h"

#include <stdio.h>
#include <string.h>

#define CHECK(condition, message) \
    do { \
        if (!(condition)) { \
            fprintf(stderr, "FAIL: %s\n", message); \
            return 1; \
        } \
    } while (0)

struct memory_device {
    uint8_t bytes[4096];
    unsigned flushes;
};

static int memory_read(void *opaque, uint64_t offset, void *buffer, size_t length)
{
    struct memory_device *device = opaque;
    memcpy(buffer, device->bytes + offset, length);
    return 0;
}

static int memory_write(void *opaque, uint64_t offset, const void *buffer,
                        size_t length)
{
    struct memory_device *device = opaque;
    memcpy(device->bytes + offset, buffer, length);
    return 0;
}

static int memory_flush(void *opaque)
{
    ++((struct memory_device *)opaque)->flushes;
    return 0;
}

int main(void)
{
    static const uint8_t expected_sha256[] = {
        0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea,
        0x41, 0x41, 0x40, 0xde, 0x5d, 0xae, 0x22, 0x23,
        0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c,
        0xb4, 0x10, 0xff, 0x61, 0xf2, 0x00, 0x15, 0xad,
    };
    uint8_t digest[LOOS_SHA256_DIGEST_SIZE];
    loos_sha256("abc", 3, digest);
    CHECK(loos_constant_time_equal(digest, expected_sha256, sizeof(digest)),
          "SHA-256 abc");
    uint8_t tampered[LOOS_SHA256_DIGEST_SIZE];
    memcpy(tampered, digest, sizeof(tampered));
    tampered[sizeof(tampered) - 1U] ^= 0x01U;
    CHECK(loos_constant_time_equal(digest, tampered, sizeof(digest) - 1U) &&
              !loos_constant_time_equal(digest, tampered, sizeof(digest)),
          "constant-time length boundary");

    struct memory_device memory = {0};
    struct loos_block_device device = {
        .capacity = sizeof(memory.bytes),
        .sector_size = 512,
        .opaque = &memory,
        .read = memory_read,
        .write = memory_write,
        .flush = memory_flush,
    };
    uint8_t sector[512];
    memset(sector, 0xa5, sizeof(sector));
    CHECK(loos_block_write(&device, 512, sector, sizeof(sector)) == 0,
          "aligned block write");
    memset(sector, 0, sizeof(sector));
    CHECK(loos_block_read(&device, 512, sector, sizeof(sector)) == 0 &&
              sector[0] == 0xa5 && sector[511] == 0xa5,
          "aligned block read");
    CHECK(loos_block_read(&device, 1, sector, sizeof(sector)) ==
              LOOS_BLOCK_EALIGN,
          "reject unaligned read");
    CHECK(loos_block_write(&device, 4096, sector, sizeof(sector)) ==
              LOOS_BLOCK_EBOUNDS,
          "reject out-of-bounds write");
    CHECK(loos_block_flush(&device) == 0 && memory.flushes == 1,
          "block flush");
    return 0;
}
