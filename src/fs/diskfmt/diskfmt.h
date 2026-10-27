/* Disk formatting for the installer: GUID partition tables and a FAT32 formatter/writer for the EFI System Partition.
 * Pure C, no allocation (callers supply buffers), all I/O through a byte-offset write callback, so the same code runs in the
 * kernel and in the host tests (which check the result with sgdisk, fsck.fat and mtools). */
#ifndef LOOS_DISKFMT_H
#define LOOS_DISKFMT_H
#include <stddef.h>
#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef int (*diskfmt_write_fn)(void *opaque, uint64_t offset, const void *data, size_t length);

/* ------------------------------------------------------------------------------------------------ GPT */
#define GPT_SECTOR 512u
#define GPT_MAX_PARTS 8

struct gpt_part {
    uint64_t first_lba, last_lba;           /* inclusive */
    uint8_t type_guid[16];                  /* on-disk (mixed-endian) byte order: use the GPT_TYPE_* constants */
    uint8_t unique_guid[16];
    char name[36];                          /* ASCII, stored as UTF-16LE */
};

extern const uint8_t GPT_TYPE_ESP[16];              /* C12A7328-F81F-11D2-BA4B-00A0C93EC93B */
extern const uint8_t GPT_TYPE_LINUX_DATA[16];       /* 0FC63DAF-8483-4772-8E79-3D69D8477DE4 (btrfs) */

/* First usable LBA of a disk (after the protective MBR, header and entry array), 1 MiB aligned. */
uint64_t gpt_first_usable_lba(void);
/* Last usable LBA of a disk with `disk_sectors` sectors (before the backup entries and header). */
uint64_t gpt_last_usable_lba(uint64_t disk_sectors);

/* Writes protective MBR, primary header + entries, backup entries + header. Partitions must lie inside the usable range
 * without overlapping and must have non-zero, distinct GUIDs. `boot_code` (optional, 440 bytes) is placed in the MBR
 * for legacy boot; pass NULL for UEFI-only. Invalid geometry, GUIDs, names, or arguments return -22. */
int gpt_write(uint64_t disk_sectors, const uint8_t disk_guid[16], const struct gpt_part *parts, int nparts,
              const uint8_t *boot_code, diskfmt_write_fn write, void *opaque);

/* ------------------------------------------------------------------------------------------------ FAT32 */
#define FAT32_MIN_BYTES (40ull << 20)

#define FAT32_MAX_DIRS 8
#define FAT32_MAX_CLUSTER_BYTES 16384
struct fat32_dir { uint32_t cluster, used; uint8_t data[FAT32_MAX_CLUSTER_BYTES]; };

struct fat32 {
    diskfmt_write_fn write; void *opaque;
    uint64_t base;                          /* byte offset of the partition on the device */
    uint32_t sectors_total, sectors_per_cluster, reserved, fat_sectors, clusters;
    uint32_t next_free;                     /* next cluster to hand out */
    uint32_t *fat;                          /* caller-provided, fat32_fat_entries() entries */
    uint16_t dos_date, dos_time;
    int ndirs;
    struct fat32_dir dirs[FAT32_MAX_DIRS];
};

/* Number of FAT entries (4 bytes each) needed for a partition of `bytes` bytes, or 0 when the size is not a valid
 * FAT32 geometry (including a non-sector-aligned size or a volume too small for FAT32). */
uint32_t fat32_fat_entries(uint64_t bytes);

/* Formats the partition at byte offset `base` of size `bytes` (volume id and label are given by the caller) and
 * prepares `fs` for writing files; the FAT lives in `fat` (zero-filled by this call) until fat32_finish(). The base
 * and size must not overflow a 64-bit byte offset and the size must describe a valid FAT32 geometry. */
int fat32_format(struct fat32 *fs, diskfmt_write_fn write, void *opaque, uint64_t base, uint64_t bytes, uint32_t volume_id,
                 const char *label, uint64_t unix_time, uint32_t *fat);
/* Creates a directory (8.3 name, upper case) inside `parent` (2 = root); returns its first cluster or 0. */
uint32_t fat32_mkdir(struct fat32 *fs, uint32_t parent, const char *name83);
/* Writes a file inside `dir`; the data comes in through `read` so large files need no staging buffer. */
typedef int (*fat32_read_fn)(void *opaque, uint64_t offset, void *buf, size_t length);
/* first_cluster (optional) receives the start of the file's contiguous cluster run, for read-back verification. */
int fat32_write_file(struct fat32 *fs, uint32_t dir, const char *name83, uint64_t size, fat32_read_fn read, void *rd_opaque, uint32_t *first_cluster);
/* Byte offset on the device of a cluster's data. */
uint64_t fat32_cluster_offset(const struct fat32 *fs, uint32_t cluster);
/* Writes both FAT copies and the FSInfo sector. Call once after the last file. */
int fat32_finish(struct fat32 *fs);

/* Unix seconds -> DOS date/time fields (1980..2107, 2 s resolution). */
void fat32_dos_time(uint64_t unix_time, uint16_t *date, uint16_t *time);

#ifdef __cplusplus
}
#endif
#endif
