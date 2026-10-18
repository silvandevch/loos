#include "block.h"

static bool power_of_two(uint32_t value)
{
    return value != 0 && (value & (value - 1U)) == 0;
}

int loos_block_validate(const struct loos_block_device *device,
                        uint64_t offset,
                        size_t length,
                        bool write)
{
    if (device == NULL || !power_of_two(device->sector_size) ||
        device->sector_size < 512U || device->read == NULL ||
        (write && device->write == NULL)) {
        return LOOS_BLOCK_EINVAL;
    }
    if (offset % device->sector_size != 0 ||
        (length != 0 && length % device->sector_size != 0)) {
        return LOOS_BLOCK_EALIGN;
    }
    if (offset > device->capacity || length > device->capacity - offset) {
        return LOOS_BLOCK_EBOUNDS;
    }
    return LOOS_BLOCK_OK;
}

int loos_block_read(const struct loos_block_device *device,
                    uint64_t offset,
                    void *buffer,
                    size_t length)
{
    if (length != 0 && buffer == NULL) {
        return LOOS_BLOCK_EINVAL;
    }
    int status = loos_block_validate(device, offset, length, false);
    if (status != LOOS_BLOCK_OK) {
        return status;
    }
    return device->read(device->opaque, offset, buffer, length) == 0
               ? LOOS_BLOCK_OK
               : LOOS_BLOCK_EIO;
}

int loos_block_write(const struct loos_block_device *device,
                     uint64_t offset,
                     const void *buffer,
                     size_t length)
{
    if (length != 0 && buffer == NULL) {
        return LOOS_BLOCK_EINVAL;
    }
    int status = loos_block_validate(device, offset, length, true);
    if (status != LOOS_BLOCK_OK) {
        return status;
    }
    return device->write(device->opaque, offset, buffer, length) == 0
               ? LOOS_BLOCK_OK
               : LOOS_BLOCK_EIO;
}

int loos_block_flush(const struct loos_block_device *device)
{
    if (device == NULL || device->flush == NULL) {
        return LOOS_BLOCK_EINVAL;
    }
    return device->flush(device->opaque) == 0 ? LOOS_BLOCK_OK
                                               : LOOS_BLOCK_EIO;
}
