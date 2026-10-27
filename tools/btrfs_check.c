#define _POSIX_C_SOURCE 200809L

#include "integrity.h"

#include <errno.h>
#include <fcntl.h>
#include <inttypes.h>
#include <limits.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

struct file_device {
    int fd;
};

static int file_read(void *opaque,
                     uint64_t offset,
                     void *buffer,
                     size_t length)
{
    struct file_device *device = opaque;
    if (offset > (uint64_t)INT64_MAX) {
        return -1;
    }
    const ssize_t result = pread(device->fd, buffer, length, (off_t)offset);
    return result == (ssize_t)length ? 0 : -1;
}

static int file_write(void *opaque,
                      uint64_t offset,
                      const void *buffer,
                      size_t length)
{
    struct file_device *device = opaque;
    if (offset > (uint64_t)INT64_MAX) {
        return -1;
    }
    const ssize_t result = pwrite(device->fd, buffer, length, (off_t)offset);
    return result == (ssize_t)length ? 0 : -1;
}

static int file_flush(void *opaque)
{
    return fdatasync(((struct file_device *)opaque)->fd) == 0 ? 0 : -1;
}

static const char *status_name(int status)
{
    switch (status) {
    case LOOS_BTRFS_OK:
        return "ok";
    case LOOS_BTRFS_EINVAL:
        return "invalid_argument";
    case LOOS_BTRFS_EIO:
        return "io_error";
    case LOOS_BTRFS_EBADFS:
        return "bad_filesystem";
    case LOOS_BTRFS_ENOTFOUND:
        return "not_found";
    case LOOS_BTRFS_ENOTDIR:
        return "not_directory";
    case LOOS_BTRFS_EISDIR:
        return "is_directory";
    case LOOS_BTRFS_ENOTSUP:
        return "unsupported";
    case LOOS_BTRFS_ECORRUPT:
        return "corrupt";
    case LOOS_BTRFS_EOVERFLOW:
        return "overflow";
    case LOOS_BTRFS_ELIMIT:
        return "limit";
    case LOOS_BTRFS_EROFS:
        return "read_only";
    default:
        return "unknown";
    }
}

static void print_json(const char *operation,
                       int status,
                       const struct loos_btrfs_integrity_report *report)
{
    printf("{\"operation\":\"%s\",\"status\":%d,\"status_name\":\"%s\"",
           operation, status, status_name(status));
    if (report != NULL) {
        printf(",\"superblocks_checked\":%" PRIu32
               ",\"superblocks_valid\":%" PRIu32
               ",\"chunks_checked\":%" PRIu32
               ",\"metadata_roots_checked\":%" PRIu32
               ",\"first_bad_offset\":%" PRIu64,
               report->superblocks_checked,
               report->superblocks_valid,
               report->chunks_checked,
               report->metadata_roots_checked,
               report->first_bad_offset);
    }
    puts("}");
}

static void print_human(const char *operation,
                        int status,
                        const struct loos_btrfs_integrity_report *report)
{
    printf("%s: %s (%d)\n", operation, status_name(status), status);
    if (report != NULL) {
        printf("  superblocks: %" PRIu32 "/%" PRIu32 " valid\n",
               report->superblocks_valid, report->superblocks_checked);
        printf("  chunks: %" PRIu32 "  metadata_roots: %" PRIu32 "\n",
               report->chunks_checked, report->metadata_roots_checked);
        if (status != LOOS_BTRFS_OK) {
            printf("  first_bad_offset: 0x%" PRIx64 "\n",
                   report->first_bad_offset);
        }
    }
}

static int parse_offset(const char *text, uint64_t *offset)
{
    char *end = NULL;
    errno = 0;
    const unsigned long long value = strtoull(text, &end, 0);
    if (errno != 0 || end == text || *end != '\0') {
        return LOOS_BTRFS_EINVAL;
    }
    *offset = (uint64_t)value;
    return LOOS_BTRFS_OK;
}

static void usage(FILE *stream)
{
    fprintf(stream,
            "usage: btrfs_check [--json] check IMAGE\n"
            "       btrfs_check [--json] restore-primary IMAGE MIRROR_OFFSET\n"
            "\n"
            "MIRROR_OFFSET is a standard Btrfs mirror offset (67108864 or 274877906944).\n");
}

int main(int argc, char **argv)
{
    bool json = false;
    int argument = 1;
    if (argument < argc && strcmp(argv[argument], "--json") == 0) {
        json = true;
        ++argument;
    }
    if (argument >= argc ||
        (strcmp(argv[argument], "check") != 0 &&
         strcmp(argv[argument], "restore-primary") != 0)) {
        usage(stderr);
        return 2;
    }

    const bool restore = strcmp(argv[argument], "restore-primary") == 0;
    const char *operation = restore ? "restore-primary" : "check";
    ++argument;
    if (argument >= argc || (restore && argument + 1 >= argc)) {
        usage(stderr);
        return 2;
    }

    const char *image_path = argv[argument++];
    uint64_t source_offset = 0;
    if (restore && parse_offset(argv[argument], &source_offset) != LOOS_BTRFS_OK) {
        usage(stderr);
        return 2;
    }

    const int flags = restore ? O_RDWR : O_RDONLY;
    const int fd = open(image_path, flags | O_CLOEXEC);
    if (fd < 0) {
        const int status = LOOS_BTRFS_EIO;
        if (json) {
            print_json(operation, status, NULL);
        } else {
            fprintf(stderr, "%s: %s (%d): %s\n", operation,
                    status_name(status), status, strerror(errno));
        }
        return 1;
    }

    struct file_device device = {.fd = fd};
    struct loos_btrfs_io io = {
        .read_at = file_read,
        .write_at = restore ? file_write : NULL,
        .flush = restore ? file_flush : NULL,
        .opaque = &device,
    };
    struct loos_btrfs_integrity_report report;
    uint8_t node_storage[LOOS_BTRFS_NODE_STORAGE_SIZE];
    int status;
    if (restore) {
        status = loos_btrfs_rescue_restore_primary_superblock(
            &io, source_offset, &report);
    } else {
        status = loos_btrfs_integrity_check(&io, node_storage,
                                            sizeof(node_storage), &report);
    }
    close(fd);

    if (json) {
        print_json(operation, status, &report);
    } else {
        print_human(operation, status, &report);
    }
    return status == LOOS_BTRFS_OK ? 0 : 1;
}
