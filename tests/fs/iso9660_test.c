#include "iso9660.h"

#include <stdio.h>
#include <string.h>

#define IMAGE_BLOCKS 64U
#define IMAGE_SIZE (IMAGE_BLOCKS * LOOS_ISO9660_BLOCK_SIZE)

#define CHECK(condition, message) \
    do { \
        if (!(condition)) { \
            fprintf(stderr, "FAIL: %s\n", message); \
            return 1; \
        } \
    } while (0)

struct image {
    uint8_t bytes[IMAGE_SIZE];
    int fail_reads;
};

static void put_both16(uint8_t *p, uint16_t value)
{
    p[0] = (uint8_t)value;
    p[1] = (uint8_t)(value >> 8);
    p[2] = (uint8_t)(value >> 8);
    p[3] = (uint8_t)value;
}

static void put_both32(uint8_t *p, uint32_t value)
{
    p[0] = (uint8_t)value;
    p[1] = (uint8_t)(value >> 8);
    p[2] = (uint8_t)(value >> 16);
    p[3] = (uint8_t)(value >> 24);
    p[4] = (uint8_t)(value >> 24);
    p[5] = (uint8_t)(value >> 16);
    p[6] = (uint8_t)(value >> 8);
    p[7] = (uint8_t)value;
}

static size_t add_record(uint8_t *block, size_t offset, uint32_t extent, uint32_t size,
                         uint8_t flags, const uint8_t *name, uint8_t name_length)
{
    size_t length = 33U + name_length;
    if ((length & 1U) != 0) length++;
    memset(block + offset, 0, length);
    block[offset] = (uint8_t)length;
    put_both32(block + offset + 2, extent);
    put_both32(block + offset + 10, size);
    block[offset + 25] = flags;
    block[offset + 32] = name_length;
    memcpy(block + offset + 33, name, name_length);
    return offset + length;
}

static void build_fixture(struct image *image)
{
    uint8_t *pvd = image->bytes + 16U * LOOS_ISO9660_BLOCK_SIZE;
    uint8_t *root = image->bytes + 20U * LOOS_ISO9660_BLOCK_SIZE;
    uint8_t *docs = image->bytes + 21U * LOOS_ISO9660_BLOCK_SIZE;
    static const uint8_t dot = 0;
    static const uint8_t dotdot = 1;
    static const uint8_t docs_name[] = "DOCS";
    static const uint8_t readme_name[] = "README.TXT;1";
    static const uint8_t note_name[] = "NOTE.TXT;1";
    size_t offset;

    memset(image, 0, sizeof *image);
    pvd[0] = 1;
    memcpy(pvd + 1, "CD001", 5);
    pvd[6] = 1;
    memcpy(pvd + 40, "LOOS ISO FIXTURE", 16);
    put_both32(pvd + 80, IMAGE_BLOCKS);
    put_both16(pvd + 128, LOOS_ISO9660_BLOCK_SIZE);
    add_record(pvd + 156, 0, 20, LOOS_ISO9660_BLOCK_SIZE, 2, &dot, 1);
    image->bytes[17U * LOOS_ISO9660_BLOCK_SIZE] = 255;
    memcpy(image->bytes + 17U * LOOS_ISO9660_BLOCK_SIZE + 1, "CD001", 5);
    image->bytes[17U * LOOS_ISO9660_BLOCK_SIZE + 6] = 1;

    offset = add_record(root, 0, 20, LOOS_ISO9660_BLOCK_SIZE, 2, &dot, 1);
    offset = add_record(root, offset, 20, LOOS_ISO9660_BLOCK_SIZE, 2, &dotdot, 1);
    offset = add_record(root, offset, 21, LOOS_ISO9660_BLOCK_SIZE, 2, docs_name, sizeof docs_name - 1U);
    (void)add_record(root, offset, 22, 11, 0, readme_name, sizeof readme_name - 1U);
    offset = add_record(docs, 0, 21, LOOS_ISO9660_BLOCK_SIZE, 2, &dot, 1);
    offset = add_record(docs, offset, 20, LOOS_ISO9660_BLOCK_SIZE, 2, &dotdot, 1);
    (void)add_record(docs, offset, 23, 5, 0, note_name, sizeof note_name - 1U);
    memcpy(image->bytes + 22U * LOOS_ISO9660_BLOCK_SIZE, "hello iso\n\0\0", 11);
    memcpy(image->bytes + 23U * LOOS_ISO9660_BLOCK_SIZE, "notes", 5);
}

static int image_read(void *opaque, uint64_t offset, void *buffer, size_t length)
{
    struct image *image = opaque;
    if (image->fail_reads || offset > IMAGE_SIZE || length > IMAGE_SIZE - (size_t)offset) return -1;
    memcpy(buffer, image->bytes + offset, length);
    return 0;
}

static int test_valid_image(void)
{
    struct image image;
    struct loos_iso9660 fs;
    struct loos_iso9660_entry entry;
    struct loos_iso9660_io io = {image_read, &image};
    char buffer[12] = {0};

    build_fixture(&image);
    CHECK(loos_iso9660_mount(&fs, &io) == 0, "mount ISO9660 fixture");
    CHECK(strcmp(fs.volume_id, "LOOS ISO FIXTURE") == 0, "read volume identifier");
    CHECK(loos_iso9660_lookup(&fs, "/", &entry) == 0 && (entry.flags & 2U) != 0, "lookup root");
    CHECK(loos_iso9660_lookup(&fs, "/readme.txt", &entry) == 0 && entry.size == 11, "lookup versionless file");
    CHECK(loos_iso9660_read(&fs, &entry, 0, buffer, 11) == 0, "read file data");
    CHECK(memcmp(buffer, "hello iso\n\0\0", 11) == 0, "file data matches fixture");
    CHECK(loos_iso9660_lookup(&fs, "docs/note.txt;1", &entry) == 0 && entry.size == 5, "lookup nested file");
    CHECK(loos_iso9660_read(&fs, &entry, 0, buffer, 6) < 0, "reject read past file size");
    CHECK(loos_iso9660_lookup(&fs, "/missing", &entry) == LOOS_ISO9660_ENOTFOUND, "missing path");
    CHECK(loos_iso9660_lookup(&fs, "/docs/note.txt/child", &entry) == LOOS_ISO9660_ENOTDIR, "reject traversal through file");
    CHECK(loos_iso9660_lookup(&fs, "/docs", &entry) == 0 && loos_iso9660_read(&fs, &entry, 0, buffer, 1) == LOOS_ISO9660_EISDIR,
          "reject directory read");
    image.fail_reads = 1;
    CHECK(loos_iso9660_lookup(&fs, "/readme.txt", &entry) == LOOS_ISO9660_EIO, "propagate I/O failure");
    return 0;
}

static int test_rejects_bad_metadata(void)
{
    struct image image;
    struct loos_iso9660 fs;
    struct loos_iso9660_io io = {image_read, &image};

    build_fixture(&image);
    image.bytes[16U * LOOS_ISO9660_BLOCK_SIZE + 129] = 0;
    CHECK(loos_iso9660_mount(&fs, &io) == LOOS_ISO9660_ECORRUPT, "reject mismatched block size fields");
    build_fixture(&image);
    image.bytes[16U * LOOS_ISO9660_BLOCK_SIZE + 1] = 'X';
    CHECK(loos_iso9660_mount(&fs, &io) == LOOS_ISO9660_EBADFS, "reject non ISO9660 descriptor");
    build_fixture(&image);
    image.bytes[20U * LOOS_ISO9660_BLOCK_SIZE] = 1;
    CHECK(loos_iso9660_mount(&fs, &io) == 0 &&
              loos_iso9660_lookup(&fs, "/missing", &(struct loos_iso9660_entry){0}) == LOOS_ISO9660_ECORRUPT,
          "reject short directory record");
    return 0;
}

int main(void)
{
    CHECK(test_valid_image() == 0, "valid ISO9660 tests");
    CHECK(test_rejects_bad_metadata() == 0, "invalid ISO9660 tests");
    puts("iso9660: all passed");
    return 0;
}
