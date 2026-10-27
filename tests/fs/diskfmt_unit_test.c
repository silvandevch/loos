#include "diskfmt.h"

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define CHECK(condition, message) \
    do { \
        if (!(condition)) { \
            fprintf(stderr, "FAIL: %s\n", message); \
            return 1; \
        } \
    } while (0)

struct io_capture {
    uint64_t base;
    uint8_t boot[GPT_SECTOR];
    uint8_t fsinfo[GPT_SECTOR];
    uint8_t file[16384];
    uint64_t file_offset;
    size_t file_length;
    int writes;
};

static int capture_write(void *opaque, uint64_t offset, const void *data, size_t length)
{
    struct io_capture *io = opaque;
    io->writes++;
    if (offset == io->base && length == GPT_SECTOR) memcpy(io->boot, data, length);
    if (offset == io->base + GPT_SECTOR && length == GPT_SECTOR) memcpy(io->fsinfo, data, length);
    if (offset == io->file_offset && length <= sizeof io->file) {
        memcpy(io->file, data, length);
        io->file_length = length;
    }
    return 0;
}

struct source {
    const uint8_t *data;
    size_t length;
};

static int source_read(void *opaque, uint64_t offset, void *buffer, size_t length)
{
    struct source *source = opaque;
    if (offset > source->length || length > source->length - (size_t)offset) return -1;
    memcpy(buffer, source->data + offset, length);
    return 0;
}

static int failing_read(void *opaque, uint64_t offset, void *buffer, size_t length)
{
    (void)opaque;
    (void)offset;
    (void)buffer;
    (void)length;
    return -1;
}

static int test_gpt(void)
{
    const uint64_t sectors = 8192;
    struct io_capture io;
    memset(&io, 0, sizeof io);
    uint8_t disk_guid[16], unique_a[16], unique_b[16];
    for (int i = 0; i < 16; i++) {
        disk_guid[i] = (uint8_t)(i + 1);
        unique_a[i] = (uint8_t)(0x20 + i);
        unique_b[i] = (uint8_t)(0x40 + i);
    }
    struct gpt_part parts[2];
    memset(parts, 0, sizeof parts);
    memcpy(parts[0].type_guid, GPT_TYPE_ESP, 16);
    memcpy(parts[0].unique_guid, unique_a, 16);
    parts[0].first_lba = 2048;
    parts[0].last_lba = 4095;
    strcpy(parts[0].name, "EFI System");
    memcpy(parts[1].type_guid, GPT_TYPE_LINUX_DATA, 16);
    memcpy(parts[1].unique_guid, unique_b, 16);
    parts[1].first_lba = 4096;
    parts[1].last_lba = gpt_last_usable_lba(sectors);
    strcpy(parts[1].name, "LoOS data");

    CHECK(gpt_last_usable_lba(33) == 0, "small GPT size has no usable range");
    CHECK(gpt_write(sectors, disk_guid, parts, 2, NULL, capture_write, &io) == 0, "write valid GPT");
    CHECK(io.writes == 5, "GPT writes MBR, two arrays and two headers");
    CHECK(io.boot[510] == 0x55 && io.boot[511] == 0xaa && io.boot[450] == 0xee, "protective MBR");
    CHECK(!memcmp(io.boot + 446 + 8, "\1\0\0\0", 4), "protective MBR starts at LBA 1");
    CHECK(memcmp(io.boot + 446 + 4, "\xee", 1) == 0, "protective partition type");

    uint8_t zero_guid[16] = {0};
    CHECK(gpt_write(sectors, zero_guid, parts, 2, NULL, capture_write, &io) < 0, "reject zero disk GUID");
    CHECK(gpt_write(sectors, disk_guid, NULL, 1, NULL, capture_write, &io) < 0, "reject missing partition array");
    parts[1].first_lba = 3000;
    CHECK(gpt_write(sectors, disk_guid, parts, 2, NULL, capture_write, &io) < 0, "reject overlapping partitions");
    parts[1].first_lba = 4096;
    memcpy(parts[1].unique_guid, unique_a, 16);
    CHECK(gpt_write(sectors, disk_guid, parts, 2, NULL, capture_write, &io) < 0, "reject duplicate partition GUID");
    return 0;
}

static int test_fat32(void)
{
    const uint64_t size = 300ull << 20;
    uint32_t entries = fat32_fat_entries(size);
    CHECK(entries > 65525, "valid ESP geometry has a FAT32-sized table");
    CHECK(fat32_fat_entries(FAT32_MIN_BYTES) == 0, "reject geometry below FAT32 cluster threshold");
    CHECK(fat32_fat_entries(size - 1) == 0, "reject unaligned FAT32 size");

    uint32_t *fat = calloc(entries, sizeof *fat);
    CHECK(fat != NULL, "allocate caller FAT table");
    struct io_capture io;
    memset(&io, 0, sizeof io);
    io.base = 1ull << 20;
    CHECK(fat32_format(NULL, capture_write, &io, io.base, size, 1, "ESP", 0, fat) < 0, "reject missing FAT32 state");
    CHECK(fat32_format(&(struct fat32){0}, capture_write, &io, UINT64_MAX, size, 1, "ESP", 0, fat) < 0, "reject base overflow");

    struct fat32 fs;
    CHECK(fat32_format(&fs, capture_write, &io, io.base, size, 0x12345678, "LOOSESP", 9999999999ull, fat) == 0, "format FAT32 ESP");
    CHECK(io.boot[510] == 0x55 && io.boot[511] == 0xaa && io.boot[82] == 'F', "FAT32 boot sector signature and type");
    CHECK(fat32_cluster_offset(&fs, 1) == 0 && fat32_cluster_offset(&fs, 2) != 0, "cluster offset bounds");

    uint8_t payload[] = "hello from LoOS";
    struct source source = {payload, sizeof payload - 1};
    uint32_t first = 0;
    io.file_offset = fat32_cluster_offset(&fs, fs.next_free);
    CHECK(fat32_write_file(&fs, 2, "HELLO.TXT", source.length, source_read, &source, &first) == 0, "write file");
    CHECK(fat32_finish(&fs) == 0, "finish FAT32");
    CHECK(io.file_length >= source.length && !memcmp(io.file, payload, source.length), "file payload reaches device callback");
    CHECK(fat[0] == 0x0ffffff8u && fat[1] == 0x0fffffffu && fat[first] == 0x0fffffffu, "FAT reserved and file entries");

    uint32_t reused = 0;
    reused = 123;
    CHECK(fat32_write_file(&fs, 2, "BAD.BIN", 1, failing_read, NULL, &reused) < 0 && reused == 0, "propagate file reader failure");
    CHECK(fat32_write_file(&fs, 2, "NEXT.BIN", 1, source_read, &source, &reused) == 0 && reused == first + 1,
          "failed allocation is released for reuse");
    CHECK(fat32_write_file(&fs, 2, "NULL.BIN", 1, NULL, NULL, NULL) < 0, "reject missing file reader");
    free(fat);

    uint16_t date = 0, time = 0;
    fat32_dos_time(0, &date, &time);
    CHECK(date == (uint16_t)((1 << 5) | 1) && time == 0, "clamp DOS time below 1980");
    fat32_dos_time(UINT64_MAX, &date, &time);
    CHECK(date == (uint16_t)((127 << 9) | (12 << 5) | 31) && time == (uint16_t)(23 << 11 | 59 << 5 | 29),
          "clamp DOS time above 2107");
    return 0;
}

int main(void)
{
    CHECK(test_gpt() == 0, "GPT tests");
    CHECK(test_fat32() == 0, "FAT32 tests");
    puts("diskfmt_unit: all passed");
    return 0;
}
