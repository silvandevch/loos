#ifndef LOOS_ISO9660_H
#define LOOS_ISO9660_H

/* A bounded, read-only ISO9660 Level 1 reader.  The reader performs no
 * allocation and obtains all bytes through the caller's physical read hook,
 * so it can be used by the host tools and a freestanding kernel alike. */

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define LOOS_ISO9660_BLOCK_SIZE UINT32_C(2048)
#define LOOS_ISO9660_MAX_NAME UINT32_C(255)
#define LOOS_ISO9660_MAX_PATH UINT32_C(1024)
#define LOOS_ISO9660_MAX_PATH_COMPONENTS UINT32_C(128)
#define LOOS_ISO9660_MAX_DIRECTORY_BYTES (UINT64_C(16) * UINT64_C(1024) * UINT64_C(1024))

enum loos_iso9660_status {
    LOOS_ISO9660_OK = 0,
    LOOS_ISO9660_EINVAL = -1,
    LOOS_ISO9660_EIO = -2,
    LOOS_ISO9660_EBADFS = -3,
    LOOS_ISO9660_ENOTFOUND = -4,
    LOOS_ISO9660_ENOTDIR = -5,
    LOOS_ISO9660_EISDIR = -6,
    LOOS_ISO9660_ENOTSUP = -7,
    LOOS_ISO9660_ECORRUPT = -8,
    LOOS_ISO9660_EOVERFLOW = -9,
    LOOS_ISO9660_ELIMIT = -10,
};

typedef int (*loos_iso9660_read_at_fn)(void *opaque,
                                       uint64_t physical_offset,
                                       void *buffer,
                                       size_t length);

struct loos_iso9660_io {
    loos_iso9660_read_at_fn read_at;
    void *opaque;
};

struct loos_iso9660_entry {
    uint64_t extent;
    uint64_t size;
    uint8_t flags;
    uint8_t name_length;
    char name[LOOS_ISO9660_MAX_NAME + 1U];
#ifdef LOOS_RUST_ISO9660
    char path[LOOS_ISO9660_MAX_PATH];     /* absolute path the Rust reader resolves again on read */
#endif
};

struct loos_iso9660 {
    struct loos_iso9660_io io;
    uint64_t volume_bytes;
    uint32_t volume_blocks;
    char volume_id[33];
    struct loos_iso9660_entry root;
    bool mounted;
#ifdef LOOS_RUST_ISO9660
    void *rust;                           /* Rust reader state; release with loos_iso9660_unmount */
#endif
};

/* Reads and validates the primary volume descriptor and its root record. */
int loos_iso9660_mount(struct loos_iso9660 *fs, const struct loos_iso9660_io *io);

#ifdef LOOS_RUST_ISO9660
/* Frees the Rust reader state (the C implementation holds none). Safe to call twice. */
void loos_iso9660_unmount(struct loos_iso9660 *fs);
#endif

/* Resolves an absolute or relative slash-separated path.  Names are matched
 * case-insensitively and a trailing ISO9660 ";1" version is optional. */
int loos_iso9660_lookup(const struct loos_iso9660 *fs,
                        const char *path,
                        struct loos_iso9660_entry *entry);

/* Reads a regular file at a byte offset.  Directories return EISDIR and reads
 * outside the declared file size are rejected before invoking the callback. */
int loos_iso9660_read(const struct loos_iso9660 *fs,
                      const struct loos_iso9660_entry *entry,
                      uint64_t offset,
                      void *buffer,
                      size_t length);

#ifdef __cplusplus
}
#endif
#endif
