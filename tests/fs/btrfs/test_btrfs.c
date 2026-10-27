#include "btrfs.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

struct image {
    uint8_t *data;
    size_t size;
    unsigned writes;
    unsigned flushes;
    bool fail_write;
    bool fail_flush;
};

struct allocator_state {
    uint64_t next;
    uint64_t physical_delta;
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
    if (image->fail_write) {
        return -1;
    }
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
    if (image->fail_flush) {
        return -1;
    }
    ++image->flushes;
    return 0;
}

static int allocate_block(void *opaque,
                          uint64_t length,
                          uint64_t *logical,
                          uint64_t *physical)
{
    struct allocator_state *state = opaque;
    if ((length != 4096 && length != 8192) ||
        state->next > UINT64_C(0x5f000)) {
        return -1;
    }
    *logical = state->next;
    *physical = state->next + state->physical_delta;
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

struct cookie_control {
    int status;
    unsigned calls;
};

static int control_cookie(void *opaque,
                          uint64_t cookie,
                          const struct loos_btrfs_dirent *entry)
{
    (void)cookie;
    (void)entry;
    struct cookie_control *control = opaque;
    ++control->calls;
    return control->status;
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
    CHECK(mount.has_csum_tree, "discover data checksum tree");
    struct loos_btrfs_mount subvolume;
    status = loos_btrfs_mount_subvolume(
        &subvolume, &io, node_storage, LOOS_BTRFS_NODE_STORAGE_SIZE, true, 5);
    CHECK(status == LOOS_BTRFS_OK && subvolume.subvolume_id == 5 &&
              subvolume.fs_tree == mount.fs_tree,
          "open default subvolume view");
    loos_btrfs_unmount(&subvolume);
    if (expect_synthetic_fixture) {
        status = loos_btrfs_mount_subvolume(
            &subvolume, &io, node_storage, LOOS_BTRFS_NODE_STORAGE_SIZE, true,
            256);
        CHECK(status == LOOS_BTRFS_OK && subvolume.subvolume_id == 256 &&
                  subvolume.fs_tree != mount.fs_tree,
              "open independent subvolume owner root");
        char subvolume_hello[64] = {0};
        size_t subvolume_bytes_read = 0;
        status = loos_btrfs_read_file(&subvolume, "/hello.txt", 0,
                                      subvolume_hello,
                                      sizeof(subvolume_hello),
                                      &subvolume_bytes_read);
        CHECK(status == LOOS_BTRFS_OK &&
                  memcmp(subvolume_hello, "hello from LoOS\n",
                         subvolume_bytes_read) == 0,
              "read independent subvolume");
        uint8_t readonly_storage[LOOS_BTRFS_MAX_NODE_SIZE +
                                 LOOS_BTRFS_SUPERBLOCK_SIZE];
        struct allocator_state readonly_allocator_state = {
            .next = UINT64_C(0x4b000)};
        const struct loos_btrfs_allocator readonly_allocator = {
            .allocate = allocate_block,
            .opaque = &readonly_allocator_state,
        };
        struct loos_btrfs_write_transaction readonly_transaction;
        status = loos_btrfs_write_transaction_begin(
            &readonly_transaction, &subvolume, &readonly_allocator,
            readonly_storage, sizeof(readonly_storage));
        CHECK(status == LOOS_BTRFS_EROFS,
              "reject writes through read-only subvolume");
        loos_btrfs_unmount(&subvolume);
    }

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
    uint64_t paused_cookie = 0;
    struct cookie_control pause = {.status = LOOS_BTRFS_STOP};
    status = loos_btrfs_readdir(&mount, 256, &paused_cookie, control_cookie,
                                &pause);
    CHECK(status == LOOS_BTRFS_OK && pause.calls == 1 && paused_cookie == 0,
          "readdir keeps entry cookie on STOP");
    memset(&cookie_listing, 0, sizeof(cookie_listing));
    status = loos_btrfs_readdir(&mount, 256, &paused_cookie, record_cookie,
                                &cookie_listing);
    CHECK(status == LOOS_BTRFS_OK && cookie_listing.count >= 3 &&
              cookie_listing.last_cookie > 0,
          "readdir resumes the stopped entry");
    uint64_t error_cookie = 0;
    struct cookie_control error = {.status = LOOS_BTRFS_EIO};
    status = loos_btrfs_readdir(&mount, 256, &error_cookie, control_cookie,
                                &error);
    CHECK(status == LOOS_BTRFS_EIO && error.calls == 1 && error_cookie == 0,
          "readdir keeps entry cookie on error");
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
    uint8_t partial[100];
    status = loos_btrfs_read_file(&mount, "/regular.bin", 123, partial,
                                  sizeof(partial), &bytes_read);
    CHECK(status == LOOS_BTRFS_OK && bytes_read == sizeof(partial) &&
              partial[0] == (uint8_t)((123U * 13U + 7U) & 0xffU),
          "verify and return partial data range");
    const bool saved_has_csum_tree = mount.has_csum_tree;
    mount.has_csum_tree = false;
    status = loos_btrfs_read_file(&mount, "/regular.bin", 0, partial,
                                  sizeof(partial), &bytes_read);
    CHECK(status == LOOS_BTRFS_ECORRUPT,
          "reject checksummed data without checksum tree");
    mount.has_csum_tree = saved_has_csum_tree;
    if (expect_synthetic_fixture) {
        uint8_t nodatasum[0x2000];
        status = loos_btrfs_read_file(&mount, "/nodatasum.bin", 0, nodatasum,
                                      sizeof(nodatasum), &bytes_read);
        CHECK(status == LOOS_BTRFS_OK && bytes_read == sizeof(nodatasum),
              "read NODATASUM extent");
        image.data[0x40000 + 123] ^= 0x01;
        status = loos_btrfs_read_file(&mount, "/regular.bin", 0, regular,
                                      sizeof(regular), &bytes_read);
        CHECK(status == LOOS_BTRFS_ECORRUPT,
              "detect data checksum failure");
        status = loos_btrfs_read_file(&mount, "/nodatasum.bin", 0, nodatasum,
                                      sizeof(nodatasum), &bytes_read);
        CHECK(status == LOOS_BTRFS_OK && nodatasum[123] !=
                  (uint8_t)((123U * 13U + 7U) & 0xffU),
              "skip checksum for NODATASUM inode");
        image.data[0x40000 + 123] ^= 0x01;
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
        const uint64_t original_root = mount.root_tree;
        uint8_t replacement[4096];
        memcpy(replacement, image.data + 0x13000, sizeof(replacement));
        uint8_t replacement2[4096];

        struct allocator_state mismatch_state = {
            .next = UINT64_C(0x4f000),
            .physical_delta = UINT64_C(0x1000),
        };
        const struct loos_btrfs_allocator mismatch_allocator = {
            .allocate = allocate_block,
            .opaque = &mismatch_state,
        };
        struct loos_btrfs_write_transaction mismatch_transaction;
        status = loos_btrfs_write_transaction_begin(
            &mismatch_transaction, &mount, &mismatch_allocator,
            transaction_storage, sizeof(transaction_storage));
        CHECK(status == LOOS_BTRFS_OK, "begin mismatched COW transaction");
        uint64_t rejected_root = 0;
        status = loos_btrfs_write_transaction_cow_tree(
            &mismatch_transaction, original_root, replacement, &rejected_root);
        CHECK(status == LOOS_BTRFS_EINVAL && image.writes == 0 &&
                  image.flushes == 0,
              "reject allocator logical-physical mismatch");
        loos_btrfs_write_transaction_abort(&mismatch_transaction);

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
        uint64_t new_root = 0;
        status = loos_btrfs_write_transaction_cow_tree(
            &transaction, original_root, replacement, &new_root);
        CHECK(status == LOOS_BTRFS_OK && new_root == UINT64_C(0x4f000),
              "write COW tree block");
        uint64_t new_generation = 0;
        memcpy(&new_generation, image.data + UINT64_C(0x4f000) + 80,
               sizeof(new_generation));
        CHECK(new_generation == 2, "COW block gets next generation");
        memcpy(replacement2, image.data + UINT64_C(0x4f000),
               sizeof(replacement2));
        status = loos_btrfs_write_transaction_publish_root(&transaction,
                                                            new_root, 0);
        CHECK(status == LOOS_BTRFS_OK, "stage new root pointer");
        CHECK(image.writes == 1 && image.flushes == 1,
              "flush COW block before publication");
        const uint64_t actual_total_bytes = mount.total_bytes;
        mount.total_bytes = UINT64_C(67108864) + 4095;
        status = loos_btrfs_write_transaction_commit(&transaction);
        CHECK(status == LOOS_BTRFS_OK, "commit COW transaction");
        CHECK(image.writes == 2 && image.flushes == 2,
              "skip truncated superblock mirror and flush publication");
        mount.total_bytes = actual_total_bytes;
        CHECK(mount.root_tree == new_root && mount.generation == 2,
              "publish root in mounted state");

        struct allocator_state data_state = {.next = UINT64_C(0x4e000)};
        const struct loos_btrfs_allocator data_allocator = {
            .allocate = allocate_block,
            .opaque = &data_state,
        };
        struct loos_btrfs_write_transaction data_transaction;
        status = loos_btrfs_write_transaction_begin(
            &data_transaction, &mount, &data_allocator, transaction_storage,
            sizeof(transaction_storage));
        CHECK(status == LOOS_BTRFS_OK, "begin data COW transaction");
        uint8_t data_payload[4096];
        memset(data_payload, 0xa5, sizeof(data_payload));
        uint64_t new_data = 0;
        status = loos_btrfs_write_transaction_cow_data(
            &data_transaction, data_payload, sizeof(data_payload), &new_data);
        CHECK(status == LOOS_BTRFS_OK && new_data == UINT64_C(0x4e000) &&
                  memcmp(image.data + UINT64_C(0x4e000), data_payload,
                         sizeof(data_payload)) == 0,
              "write data extent with COW");
        for (size_t i = 0; i < sizeof(regular); ++i) {
            CHECK(image.data[UINT64_C(0x40000) + i] ==
                      (uint8_t)((i * 13U + 7U) & 0xffU),
                  "data COW preserves old extent");
        }
        loos_btrfs_write_transaction_abort(&data_transaction);

        struct allocator_state overlap_state = {.next = UINT64_C(0x4e000)};
        const struct loos_btrfs_allocator overlap_allocator = {
            .allocate = allocate_block,
            .opaque = &overlap_state,
        };
        struct loos_btrfs_write_transaction overlap_transaction;
        status = loos_btrfs_write_transaction_begin(
            &overlap_transaction, &mount, &overlap_allocator,
            transaction_storage, sizeof(transaction_storage));
        CHECK(status == LOOS_BTRFS_OK, "begin overlap transaction");
        uint8_t overlap_payload[8192];
        memset(overlap_payload, 0x5a, sizeof(overlap_payload));
        uint64_t overlap_data = 0;
        status = loos_btrfs_write_transaction_cow_data(
            &overlap_transaction, overlap_payload, sizeof(overlap_payload),
            &overlap_data);
        CHECK(status == LOOS_BTRFS_OK, "write first overlapping range");
        status = loos_btrfs_write_transaction_cow_data(
            &overlap_transaction, data_payload, sizeof(data_payload),
            &new_data);
        CHECK(status == LOOS_BTRFS_EINVAL,
              "reject overlapping transaction range");
        loos_btrfs_write_transaction_abort(&overlap_transaction);

        struct allocator_state staged_state = {.next = UINT64_C(0x4e000)};
        const struct loos_btrfs_allocator staged_allocator = {
            .allocate = allocate_block,
            .opaque = &staged_state,
        };
        struct loos_btrfs_write_transaction staged_transaction;
        status = loos_btrfs_write_transaction_begin(
            &staged_transaction, &mount, &staged_allocator,
            transaction_storage, sizeof(transaction_storage));
        CHECK(status == LOOS_BTRFS_OK, "begin staged-root transaction");
        uint8_t staged_replacement[4096];
        memcpy(staged_replacement, replacement2, sizeof(staged_replacement));
        const uint64_t staged_old_logical = UINT64_C(0x4e000);
        memcpy(staged_replacement + 48, &staged_old_logical,
               sizeof(staged_old_logical));
        uint64_t staged_block = 0;
        status = loos_btrfs_write_transaction_cow_tree(
            &staged_transaction, new_root, replacement2, &staged_block);
        CHECK(status == LOOS_BTRFS_OK, "write staged-root COW block");
        status = loos_btrfs_write_transaction_publish_root(
            &staged_transaction, staged_block, 0);
        CHECK(status == LOOS_BTRFS_OK, "stage root before later COW");
        const uint64_t staged_root_before_later_cow = staged_block;
        uint64_t later_block = 0;
        status = loos_btrfs_write_transaction_cow_tree(
            &staged_transaction, staged_block, staged_replacement,
            &later_block);
        uint64_t staged_root_after_later_cow = 0;
        memcpy(&staged_root_after_later_cow,
               staged_transaction.staged_superblock + 80,
               sizeof(staged_root_after_later_cow));
        CHECK(status == LOOS_BTRFS_OK &&
                  staged_root_after_later_cow ==
                      staged_root_before_later_cow,
              "preserve staged root across later COW");
        loos_btrfs_write_transaction_abort(&staged_transaction);

        struct allocator_state pinned_state = {.next = UINT64_C(0x52000)};
        const struct loos_btrfs_allocator pinned_allocator = {
            .allocate = allocate_block,
            .opaque = &pinned_state,
        };
        struct loos_btrfs_write_transaction pinned_transaction;
        status = loos_btrfs_write_transaction_begin(
            &pinned_transaction, &mount, &pinned_allocator,
            transaction_storage, sizeof(transaction_storage));
        CHECK(status == LOOS_BTRFS_OK, "begin pinned-generation transaction");
        image.data[UINT64_C(0x10000) + 72] ^= 1;
        status = loos_btrfs_write_transaction_publish_root(
            &pinned_transaction, new_root, 0);
        image.data[UINT64_C(0x10000) + 72] ^= 1;
        CHECK(status == LOOS_BTRFS_EINVAL,
              "reject transaction after generation changes");
        loos_btrfs_write_transaction_abort(&pinned_transaction);

        struct allocator_state flush_state = {.next = UINT64_C(0x4e000)};
        const struct loos_btrfs_allocator flush_allocator = {
            .allocate = allocate_block,
            .opaque = &flush_state,
        };
        struct loos_btrfs_write_transaction flush_transaction;
        status = loos_btrfs_write_transaction_begin(
            &flush_transaction, &mount, &flush_allocator, transaction_storage,
            sizeof(transaction_storage));
        CHECK(status == LOOS_BTRFS_OK, "begin flush-failure transaction");
        image.fail_flush = true;
        uint64_t flush_root = 0;
        status = loos_btrfs_write_transaction_cow_tree(
            &flush_transaction, new_root, replacement2, &flush_root);
        image.fail_flush = false;
        CHECK(status == LOOS_BTRFS_EIO && mount.root_tree == new_root,
              "abort on COW flush failure");
        loos_btrfs_write_transaction_abort(&flush_transaction);

        struct allocator_state write_state = {.next = UINT64_C(0x4d000)};
        const struct loos_btrfs_allocator write_allocator = {
            .allocate = allocate_block,
            .opaque = &write_state,
        };
        struct loos_btrfs_write_transaction write_transaction;
        status = loos_btrfs_write_transaction_begin(
            &write_transaction, &mount, &write_allocator, transaction_storage,
            sizeof(transaction_storage));
        CHECK(status == LOOS_BTRFS_OK, "begin write-failure transaction");
        uint64_t write_root = 0;
        status = loos_btrfs_write_transaction_cow_tree(
            &write_transaction, new_root, replacement2, &write_root);
        CHECK(status == LOOS_BTRFS_OK, "stage write-failure COW block");
        status = loos_btrfs_write_transaction_publish_root(&write_transaction,
                                                            write_root, 0);
        CHECK(status == LOOS_BTRFS_OK, "stage write-failure root");
        image.fail_write = true;
        status = loos_btrfs_write_transaction_commit(&write_transaction);
        image.fail_write = false;
        CHECK(status == LOOS_BTRFS_EIO && mount.root_tree == new_root,
              "abort publication on superblock write failure");
        loos_btrfs_write_transaction_abort(&write_transaction);

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
