#include "btrfs.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

struct image {
    uint8_t *data;
    size_t size;
    unsigned writes;
    unsigned flushes;
};

struct allocator_state {
    uint64_t next;
};

struct listing {
    unsigned count;
    bool hello;
    bool regular;
    bool subdir;
    bool nested;
};

struct cookie_listing {
    unsigned count;
    uint64_t last_cookie;
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

static int image_write(void *opaque,
                       uint64_t physical_offset,
                       const void *buffer,
                       size_t length)
{
    struct image *image = opaque;
    if (physical_offset > image->size ||
        length > image->size - (size_t)physical_offset) {
        return -1;
    }
    memcpy(image->data + (size_t)physical_offset, buffer, length);
    ++image->writes;
    return 0;
}

static int image_flush(void *opaque)
{
    struct image *image = opaque;
    ++image->flushes;
    return 0;
}

static int allocate_block(void *opaque,
                          uint64_t length,
                          uint64_t *logical,
                          uint64_t *physical)
{
    struct allocator_state *state = opaque;
    if (length != 4096 || state->next > UINT64_C(0x5f000)) {
        return -1;
    }
    *logical = state->next;
    *physical = state->next;
    state->next += 4096;
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

static int record_cookie(void *opaque,
                         uint64_t cookie,
                         const struct loos_btrfs_dirent *entry)
{
    (void)entry;
    struct cookie_listing *listing = opaque;
    ++listing->count;
    listing->last_cookie = cookie;
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
    FILE *image_file = fopen(argv[1], "rb");
    CHECK(image_file != NULL, "open fixture");
    CHECK(fseek(image_file, 0, SEEK_END) == 0, "seek fixture end");
    long file_size = ftell(image_file);
    CHECK(file_size > 0, "fixture has content");
    CHECK(fseek(image_file, 0, SEEK_SET) == 0, "seek fixture start");

    struct image image = {
        .size = (size_t)file_size,
        .data = malloc((size_t)file_size),
    };
    CHECK(image.data != NULL, "allocate fixture");
    CHECK(fread(image.data, 1, image.size, image_file) == image.size,
          "read fixture");
    fclose(image_file);

    void *node_storage = malloc(LOOS_BTRFS_NODE_STORAGE_SIZE);
    CHECK(node_storage != NULL, "allocate tree storage");
    struct loos_btrfs_mount mount;
    const struct loos_btrfs_io io = {
        .read_at = image_read,
        .write_at = image_write,
        .flush = image_flush,
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

    struct loos_btrfs_stat root_stat;
    status = loos_btrfs_stat_path(&mount, "/", &root_stat);
    CHECK(status == LOOS_BTRFS_OK && root_stat.inode == 256 &&
              (root_stat.mode & 0170000) == 0040000,
          "stat root path");
    struct loos_btrfs_dirent lookup_entry;
    struct loos_btrfs_stat lookup_stat;
    status = loos_btrfs_lookup(&mount, mount.fs_root_dirid, "hello.txt", &lookup_entry,
                               &lookup_stat);
    CHECK(status == LOOS_BTRFS_OK && lookup_entry.inode != 0 &&
              lookup_stat.size == strlen("hello from LoOS\n"),
          "lookup child by inode and name");
    uint64_t cookie = 0;
    struct cookie_listing cookie_listing = {0};
    status = loos_btrfs_readdir(&mount, 256, &cookie, record_cookie,
                                &cookie_listing);
    CHECK(status == LOOS_BTRFS_OK && cookie_listing.count >= 3 && cookie > 0,
          "readdir resumable cookie");
    struct loos_btrfs_file file;
    status = loos_btrfs_open_inode(&mount, lookup_entry.inode, &file);
    CHECK(status == LOOS_BTRFS_OK, "open regular inode");

    char hello[64] = {0};
    size_t bytes_read = 0;
    status = loos_btrfs_read_file(&mount, "/hello.txt", 0, hello,
                                  sizeof(hello), &bytes_read);
    CHECK(status == LOOS_BTRFS_OK, "read inline file");
    CHECK(bytes_read == strlen("hello from LoOS\n"), "inline size");
    CHECK(memcmp(hello, "hello from LoOS\n", bytes_read) == 0,
          "inline content");
    memset(hello, 0, sizeof(hello));
    status = loos_btrfs_read_inode(&file, 0, hello, sizeof(hello),
                                   &bytes_read);
    CHECK(status == LOOS_BTRFS_OK &&
              memcmp(hello, "hello from LoOS\n", bytes_read) == 0,
          "read open inode");

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

    if (expect_synthetic_fixture) {
        uint8_t transaction_storage[LOOS_BTRFS_MAX_NODE_SIZE +
                                    LOOS_BTRFS_SUPERBLOCK_SIZE];
        struct allocator_state allocator_state = {.next = UINT64_C(0x4f000)};
        const struct loos_btrfs_allocator allocator = {
            .allocate = allocate_block,
            .opaque = &allocator_state,
        };
        struct loos_btrfs_write_transaction transaction;
        status = loos_btrfs_write_transaction_begin(
            &transaction, &mount, &allocator, transaction_storage,
            sizeof(transaction_storage));
        CHECK(status == LOOS_BTRFS_OK, "begin COW transaction");
        uint8_t replacement[4096];
        memcpy(replacement, image.data + 0x13000, sizeof(replacement));
        uint64_t new_root = 0;
        status = loos_btrfs_write_transaction_cow_tree(
            &transaction, mount.root_tree, replacement, &new_root);
        CHECK(status == LOOS_BTRFS_OK && new_root == UINT64_C(0x4f000),
              "write COW tree block");
        status = loos_btrfs_write_transaction_publish_root(&transaction,
                                                            new_root, 0);
        CHECK(status == LOOS_BTRFS_OK, "stage new root pointer");
        CHECK(image.writes == 1 && image.flushes == 1,
              "flush COW block before publication");
        status = loos_btrfs_write_transaction_commit(&transaction);
        CHECK(status == LOOS_BTRFS_OK, "commit COW transaction");
        CHECK(image.writes == 2 && image.flushes == 2,
              "flush superblock publication");
        CHECK(mount.root_tree == new_root && mount.generation == 2,
              "publish root in mounted state");

        struct loos_btrfs_mount remounted;
        status = loos_btrfs_mount(&remounted, &io, node_storage,
                                  LOOS_BTRFS_NODE_STORAGE_SIZE, true);
        CHECK(status == LOOS_BTRFS_OK && remounted.root_tree == new_root,
              "remount committed root");
        loos_btrfs_unmount(&remounted);
    }

    /* A tree checksum failure must be visible to the caller, not converted to
     * an empty directory or a short read. */
    if (expect_synthetic_fixture) {
        image.data[(size_t)mount.root_tree + 128U] ^= 0x01;
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
