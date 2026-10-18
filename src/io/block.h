#ifndef LOOS_BLOCK_H
#define LOOS_BLOCK_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

enum loos_block_status {
    LOOS_BLOCK_OK = 0,
    LOOS_BLOCK_EINVAL = -1,
    LOOS_BLOCK_EIO = -2,
    LOOS_BLOCK_EALIGN = -3,
    LOOS_BLOCK_EBOUNDS = -4,
    LOOS_BLOCK_EBUSY = -5,
};

typedef int (*loos_block_read_fn)(void *opaque,
                                  uint64_t offset,
                                  void *buffer,
                                  size_t length);
typedef int (*loos_block_write_fn)(void *opaque,
                                   uint64_t offset,
                                   const void *buffer,
                                   size_t length);
typedef int (*loos_block_flush_fn)(void *opaque);

struct loos_block_device {
    uint64_t capacity;
    uint32_t sector_size;
    void *opaque;
    loos_block_read_fn read;
    loos_block_write_fn write;
    loos_block_flush_fn flush;
};

int loos_block_validate(const struct loos_block_device *device,
                        uint64_t offset,
                        size_t length,
                        bool write);
int loos_block_read(const struct loos_block_device *device,
                    uint64_t offset,
                    void *buffer,
                    size_t length);
int loos_block_write(const struct loos_block_device *device,
                     uint64_t offset,
                     const void *buffer,
                     size_t length);
int loos_block_flush(const struct loos_block_device *device);

#endif
