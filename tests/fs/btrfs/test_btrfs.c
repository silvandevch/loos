#include "btrfs.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

struct image {
    uint8_t *data;
    size_t size;
};

struct listing {
    unsigned count;
    bool hello;
    bool regular;
    bool subdir;
    bool nested;
};

static int image_read(void *opaque,
                      uint64_t physical_offset,
                      void *buffer,
                      size_t length)
{
    struct image *image = opaque;
    if (physical_offset > image->size ||
        length > image->size - (size_t)physical_offset) {
        return -1;
    }
    memcpy(buffer, image->data + (size_t)physical_offset, length);
    return 0;
}

static int record_dirent(void *opaque, const struct loos_btrfs_dirent *entry)
{
    struct listing *listing = opaque;
    ++listing->count;
    if (strcmp(entry->name, "hello.txt") == 0) {
        listing->hello = true;
    }
    if (strcmp(entry->name, "regular.bin") == 0) {
        listing->regular = true;
    }
    if (strcmp(entry->name, "subdir") == 0) {
        listing->subdir = true;
    }
    if (strcmp(entry->name, "nested.txt") == 0) {
        listing->nested = true;
    }
    return LOOS_BTRFS_OK;
}

static int fail(const char *message, int line)
{
    fprintf(stderr, "FAIL line %d: %s\n", line, message);
    return 1;
}

#define CHECK(condition, message) \
    do { \
        if (!(condition)) { \
            return fail((message), __LINE__); \
        } \
    } while (0)

int main(int argc, char **argv)
{
    CHECK(argc == 2 || (argc == 3 && strcmp(argv[2], "--fixture") == 0),
          "fixture path argument required");
    const bool expect_synthetic_fixture = argc == 3;
    FILE *file = fopen(argv[1], "rb");
    CHECK(file != NULL, "open fixture");
    CHECK(fseek(file, 0, SEEK_END) == 0, "seek fixture end");
    long file_size = ftell(file);
    CHECK(file_size > 0, "fixture has content");
    CHECK(fseek(file, 0, SEEK_SET) == 0, "seek fixture start");

    struct image image = {
        .size = (size_t)file_size,
        .data = malloc((size_t)file_size),
    };
    CHECK(image.data != NULL, "allocate fixture");
    CHECK(fread(image.data, 1, image.size, file) == image.size,
          "read fixture");
    fclose(file);

    void *node_storage = malloc(LOOS_BTRFS_NODE_STORAGE_SIZE);
    CHECK(node_storage != NULL, "allocate tree storage");
    struct loos_btrfs_mount mount;
    const struct loos_btrfs_io io = {
        .read_at = image_read,
        .opaque = &image,
    };
    int status = loos_btrfs_mount(&mount, &io, node_storage,
                                  LOOS_BTRFS_NODE_STORAGE_SIZE, true);
    CHECK(status == LOOS_BTRFS_OK, "mount checksummed fixture");
    CHECK(mount.chunk_count >= 2, "discover logical chunks");
    CHECK(mount.fs_tree != 0, "find top-level fs root");
    CHECK(mount.fs_root_dirid == 256, "find root directory inode");

    struct listing listing = {0};
    status = loos_btrfs_list_dir(&mount, "/", record_dirent, &listing);
    CHECK(status == LOOS_BTRFS_OK, "list root directory");
    CHECK(listing.count >= 3 && listing.hello && listing.regular &&
              listing.subdir,
          "list expected names");

    memset(&listing, 0, sizeof(listing));
    status = loos_btrfs_list_dir(&mount, "/subdir", record_dirent, &listing);
    CHECK(status == LOOS_BTRFS_OK && listing.nested,
          "list nested directory");

    char hello[64] = {0};
    size_t bytes_read = 0;
    status = loos_btrfs_read_file(&mount, "/hello.txt", 0, hello,
                                  sizeof(hello), &bytes_read);
    CHECK(status == LOOS_BTRFS_OK, "read inline file");
    CHECK(bytes_read == strlen("hello from LoOS\n"), "inline size");
    CHECK(memcmp(hello, "hello from LoOS\n", bytes_read) == 0,
          "inline content");

    memset(hello, 0, sizeof(hello));
    status = loos_btrfs_read_file(&mount, "/subdir/nested.txt", 0, hello,
                                  sizeof(hello), &bytes_read);
    CHECK(status == LOOS_BTRFS_OK, "read nested inline file");
    CHECK(bytes_read == strlen("nested from LoOS\n"), "nested inline size");
    CHECK(memcmp(hello, "nested from LoOS\n", bytes_read) == 0,
          "nested inline content");

    uint8_t regular[0x2000];
    status = loos_btrfs_read_file(&mount, "/regular.bin", 0, regular,
                                  sizeof(regular), &bytes_read);
    CHECK(status == LOOS_BTRFS_OK, "read regular file");
    CHECK(bytes_read == sizeof(regular), "regular size");
    for (size_t i = 0; i < sizeof(regular); ++i) {
        CHECK(regular[i] == (uint8_t)((i * 13U + 7U) & 0xffU),
              "regular content");
    }

    status = loos_btrfs_read_file(&mount, "/", 0, hello, sizeof(hello),
                                  &bytes_read);
    CHECK(status == LOOS_BTRFS_EISDIR, "reject directory as file");
    status = loos_btrfs_read_file(&mount, "/missing", 0, hello, sizeof(hello),
                                  &bytes_read);
    CHECK(status == LOOS_BTRFS_ENOTFOUND, "missing path status");

    /* A tree checksum failure must be visible to the caller, not converted to
     * an empty directory or a short read. */
    if (expect_synthetic_fixture) {
        image.data[0x13000 + 128] ^= 0x01;
        loos_btrfs_unmount(&mount);
        status = loos_btrfs_mount(&mount, &io, node_storage,
                                  LOOS_BTRFS_NODE_STORAGE_SIZE, true);
        CHECK(status == LOOS_BTRFS_ECORRUPT,
              "detect metadata checksum failure");
    }

    free(node_storage);
    free(image.data);
    return 0;
}
