#include "iso9660.h"

#include <string.h>

#define ISO9660_PRIMARY_DESCRIPTOR_OFFSET UINT64_C(32768)
#define ISO9660_DESCRIPTOR_SIZE LOOS_ISO9660_BLOCK_SIZE
#define ISO9660_ROOT_RECORD_OFFSET UINT32_C(156)
#define ISO9660_RECORD_MIN_SIZE UINT32_C(34)
#define ISO9660_DIRECTORY_FLAG UINT8_C(0x02)

static uint16_t read_le16(const uint8_t *p)
{
    return (uint16_t)p[0] | ((uint16_t)p[1] << 8);
}

static uint16_t read_be16(const uint8_t *p)
{
    return ((uint16_t)p[0] << 8) | (uint16_t)p[1];
}

static uint32_t read_le32(const uint8_t *p)
{
    return (uint32_t)p[0] | ((uint32_t)p[1] << 8) |
           ((uint32_t)p[2] << 16) | ((uint32_t)p[3] << 24);
}

static uint32_t read_be32(const uint8_t *p)
{
    return ((uint32_t)p[0] << 24) | ((uint32_t)p[1] << 16) |
           ((uint32_t)p[2] << 8) | (uint32_t)p[3];
}

static int read_both16(const uint8_t *p, uint16_t *value)
{
    uint16_t little = read_le16(p);
    uint16_t big = read_be16(p + 2);
    if (little != big) return LOOS_ISO9660_ECORRUPT;
    *value = little;
    return LOOS_ISO9660_OK;
}

static int read_both32(const uint8_t *p, uint32_t *value)
{
    uint32_t little = read_le32(p);
    uint32_t big = read_be32(p + 4);
    if (little != big) return LOOS_ISO9660_ECORRUPT;
    *value = little;
    return LOOS_ISO9660_OK;
}

static int checked_extent(const struct loos_iso9660 *fs,
                          uint64_t extent,
                          uint64_t size)
{
    uint64_t offset;

    if (!fs->mounted || extent >= fs->volume_blocks) return LOOS_ISO9660_ECORRUPT;
    if (extent > UINT64_MAX / LOOS_ISO9660_BLOCK_SIZE) return LOOS_ISO9660_EOVERFLOW;
    offset = extent * LOOS_ISO9660_BLOCK_SIZE;
    if (size > fs->volume_bytes - offset) return LOOS_ISO9660_ECORRUPT;
    return LOOS_ISO9660_OK;
}

static int read_physical(const struct loos_iso9660 *fs,
                         uint64_t offset,
                         void *buffer,
                         size_t length)
{
    if (length == 0) return LOOS_ISO9660_OK;
    if (buffer == NULL || fs->io.read_at == NULL) return LOOS_ISO9660_EINVAL;
    if (offset > fs->volume_bytes || (uint64_t)length > fs->volume_bytes - offset) {
        return LOOS_ISO9660_EOVERFLOW;
    }
    return fs->io.read_at(fs->io.opaque, offset, buffer, length) == 0
               ? LOOS_ISO9660_OK
               : LOOS_ISO9660_EIO;
}

static int parse_record(const uint8_t *record,
                        size_t available,
                        struct loos_iso9660_entry *entry,
                        size_t *record_length)
{
    uint8_t length;
    uint32_t extent, size;
    int status;

    if (available == 0) return LOOS_ISO9660_ECORRUPT;
    length = record[0];
    if (length == 0) {
        *record_length = 0;
        return LOOS_ISO9660_OK;
    }
    if (length < ISO9660_RECORD_MIN_SIZE || length > available) return LOOS_ISO9660_ECORRUPT;
    if ((size_t)33U + record[32] > length) return LOOS_ISO9660_ECORRUPT;
    status = read_both32(record + 2, &extent);
    if (status != LOOS_ISO9660_OK) return status;
    status = read_both32(record + 10, &size);
    if (status != LOOS_ISO9660_OK) return status;

    memset(entry, 0, sizeof *entry);
    entry->extent = extent;
    entry->size = size;
    entry->flags = record[25];
    entry->name_length = record[32];
    if (entry->name_length > LOOS_ISO9660_MAX_NAME) return LOOS_ISO9660_ECORRUPT;
    memcpy(entry->name, record + 33, entry->name_length);
    entry->name[entry->name_length] = '\0';
    *record_length = length;
    return LOOS_ISO9660_OK;
}

static size_t bounded_strlen(const char *text, size_t limit)
{
    size_t length = 0;
    while (length <= limit && text[length] != '\0') length++;
    return length;
}

static int next_component(const char **cursor, size_t *length)
{
    const char *start;
    const char *p = *cursor;

    while (*p == '/') p++;
    if (*p == '\0') {
        *cursor = p;
        *length = 0;
        return 0;
    }
    start = p;
    while (*p != '\0' && *p != '/') p++;
    *cursor = p;
    *length = (size_t)(p - start);
    return 1;
}

static char fold_ascii(char value)
{
    if (value >= 'a' && value <= 'z') return (char)(value - ('a' - 'A'));
    return value;
}

static bool entry_name_equal(const struct loos_iso9660_entry *entry,
                             const char *component,
                             size_t component_length)
{
    size_t entry_length = entry->name_length;
    size_t requested_length = component_length;
    size_t i;

    for (i = 0; i < requested_length; i++) {
        if (component[i] == ';') {
            if (entry_length != requested_length) return false;
            for (i = 0; i < entry_length; i++) {
                if (fold_ascii(entry->name[i]) != fold_ascii(component[i])) return false;
            }
            return true;
        }
    }
    for (i = 0; i < entry_length; i++) {
        if (entry->name[i] == ';') {
            entry_length = i;
            break;
        }
    }
    if (entry_length != requested_length) return false;
    for (i = 0; i < entry_length; i++) {
        if (fold_ascii(entry->name[i]) != fold_ascii(component[i])) return false;
    }
    return true;
}

static int find_in_directory(const struct loos_iso9660 *fs,
                             const struct loos_iso9660_entry *directory,
                             const char *component,
                             size_t component_length,
                             struct loos_iso9660_entry *found)
{
    uint8_t block[LOOS_ISO9660_BLOCK_SIZE];
    uint64_t block_count, block_index;
    int status;

    if ((directory->flags & ISO9660_DIRECTORY_FLAG) == 0) return LOOS_ISO9660_ENOTDIR;
    if (directory->size > LOOS_ISO9660_MAX_DIRECTORY_BYTES) return LOOS_ISO9660_ELIMIT;
    status = checked_extent(fs, directory->extent, directory->size);
    if (status != LOOS_ISO9660_OK) return status;
    block_count = (directory->size + LOOS_ISO9660_BLOCK_SIZE - 1U) / LOOS_ISO9660_BLOCK_SIZE;
    for (block_index = 0; block_index < block_count; block_index++) {
        uint64_t physical = directory->extent * LOOS_ISO9660_BLOCK_SIZE +
                            block_index * LOOS_ISO9660_BLOCK_SIZE;
        size_t used = LOOS_ISO9660_BLOCK_SIZE;
        size_t position;

        status = read_physical(fs, physical, block, sizeof block);
        if (status != LOOS_ISO9660_OK) return status;
        if (directory->size - block_index * LOOS_ISO9660_BLOCK_SIZE < used) {
            used = (size_t)(directory->size - block_index * LOOS_ISO9660_BLOCK_SIZE);
        }
        for (position = 0; position < used;) {
            struct loos_iso9660_entry candidate;
            size_t record_length;

            status = parse_record(block + position, used - position, &candidate, &record_length);
            if (status != LOOS_ISO9660_OK) return status;
            if (record_length == 0) break;
            if (candidate.name_length > 1U ||
                (candidate.name_length == 1U && (unsigned char)candidate.name[0] > 1U)) {
                if (entry_name_equal(&candidate, component, component_length)) {
                    *found = candidate;
                    return checked_extent(fs, candidate.extent, candidate.size);
                }
            }
            position += record_length;
        }
    }
    return LOOS_ISO9660_ENOTFOUND;
}

int loos_iso9660_mount(struct loos_iso9660 *fs, const struct loos_iso9660_io *io)
{
    uint8_t descriptor[ISO9660_DESCRIPTOR_SIZE];
    uint16_t block_size;
    uint32_t volume_blocks;
    size_t i;
    int status;

    if (fs == NULL || io == NULL || io->read_at == NULL) return LOOS_ISO9660_EINVAL;
    memset(fs, 0, sizeof *fs);
    fs->io = *io;
    status = read_physical(&(struct loos_iso9660){.io = *io, .volume_bytes = UINT64_MAX},
                           ISO9660_PRIMARY_DESCRIPTOR_OFFSET,
                           descriptor,
                           sizeof descriptor);
    if (status != LOOS_ISO9660_OK) return status;
    if (descriptor[0] != 1U || memcmp(descriptor + 1, "CD001", 5) != 0 || descriptor[6] != 1U) {
        return LOOS_ISO9660_EBADFS;
    }
    status = read_both16(descriptor + 128, &block_size);
    if (status != LOOS_ISO9660_OK) return status;
    if (block_size != LOOS_ISO9660_BLOCK_SIZE) return LOOS_ISO9660_ENOTSUP;
    status = read_both32(descriptor + 80, &volume_blocks);
    if (status != LOOS_ISO9660_OK) return status;
    if (volume_blocks == 0U) return LOOS_ISO9660_EOVERFLOW;
    fs->volume_blocks = volume_blocks;
    fs->volume_bytes = (uint64_t)volume_blocks * LOOS_ISO9660_BLOCK_SIZE;
    i = sizeof fs->volume_id - 1U;
    while (i > 0U && (descriptor[40 + i - 1U] == ' ' || descriptor[40 + i - 1U] == '\0')) i--;
    memcpy(fs->volume_id, descriptor + 40, i);
    fs->volume_id[i] = '\0';
    status = parse_record(descriptor + ISO9660_ROOT_RECORD_OFFSET,
                          sizeof descriptor - ISO9660_ROOT_RECORD_OFFSET,
                          &fs->root,
                          &i);
    if (status != LOOS_ISO9660_OK) return status;
    if ((fs->root.flags & ISO9660_DIRECTORY_FLAG) == 0 || fs->root.size == 0U) {
        return LOOS_ISO9660_ECORRUPT;
    }
    fs->mounted = true;
    status = checked_extent(fs, fs->root.extent, fs->root.size);
    if (status != LOOS_ISO9660_OK) fs->mounted = false;
    return status;
}

int loos_iso9660_lookup(const struct loos_iso9660 *fs,
                        const char *path,
                        struct loos_iso9660_entry *entry)
{
    const char *cursor;
    struct loos_iso9660_entry current;
    size_t path_length;
    uint32_t components = 0;

    if (fs == NULL || path == NULL || entry == NULL || !fs->mounted) return LOOS_ISO9660_EINVAL;
    path_length = bounded_strlen(path, LOOS_ISO9660_MAX_PATH);
    if (path_length > LOOS_ISO9660_MAX_PATH) return LOOS_ISO9660_ELIMIT;
    current = fs->root;
    cursor = path;
    for (;;) {
        size_t component_length;
        struct loos_iso9660_entry next;
        int has_component = next_component(&cursor, &component_length);
        int status;

        if (!has_component) {
            *entry = current;
            return LOOS_ISO9660_OK;
        }
        if (component_length > LOOS_ISO9660_MAX_NAME) return LOOS_ISO9660_ELIMIT;
        if (++components > LOOS_ISO9660_MAX_PATH_COMPONENTS) return LOOS_ISO9660_ELIMIT;
        status = find_in_directory(fs, &current, cursor - component_length,
                                   component_length, &next);
        if (status != LOOS_ISO9660_OK) return status;
        current = next;
    }
}

int loos_iso9660_read(const struct loos_iso9660 *fs,
                      const struct loos_iso9660_entry *entry,
                      uint64_t offset,
                      void *buffer,
                      size_t length)
{
    uint64_t physical;
    int status;

    if (fs == NULL || entry == NULL || !fs->mounted) return LOOS_ISO9660_EINVAL;
    if ((entry->flags & ISO9660_DIRECTORY_FLAG) != 0) return LOOS_ISO9660_EISDIR;
    if (offset > entry->size || (uint64_t)length > entry->size - offset) return LOOS_ISO9660_EOVERFLOW;
    status = checked_extent(fs, entry->extent, entry->size);
    if (status != LOOS_ISO9660_OK) return status;
    physical = entry->extent * LOOS_ISO9660_BLOCK_SIZE + offset;
    return read_physical(fs, physical, buffer, length);
}
