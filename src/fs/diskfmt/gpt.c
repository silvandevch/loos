#include "diskfmt.h"
#include <string.h>

/* mixed-endian on-disk GUID order: time_low, time_mid, time_hi LE, then the rest as written */
const uint8_t GPT_TYPE_ESP[16] = { 0x28, 0x73, 0x2a, 0xc1, 0x1f, 0xf8, 0xd2, 0x11, 0xba, 0x4b, 0x00, 0xa0, 0xc9, 0x3e, 0xc9, 0x3b };
const uint8_t GPT_TYPE_LINUX_DATA[16] = { 0xaf, 0x3d, 0xc6, 0x0f, 0x83, 0x84, 0x72, 0x47, 0x8e, 0x79, 0x3d, 0x69, 0xd8, 0x47, 0x7e, 0x4f };

#define ENTRY_SIZE 128u
#define ENTRY_COUNT 128u
#define ENTRY_SECTORS (ENTRY_SIZE * ENTRY_COUNT / GPT_SECTOR)          /* 32 */

static uint32_t crc32(const uint8_t *d, size_t n)
{
    uint32_t c = 0xffffffffu;
    while (n--) { c ^= *d++; for (int i = 0; i < 8; i++) c = (c >> 1) ^ (0xedb88320u & (0u - (c & 1))); }
    return ~c;
}
static void p32(uint8_t *b, size_t o, uint32_t v) { for (int i = 0; i < 4; i++) b[o + i] = (uint8_t)(v >> (8 * i)); }
static void p64(uint8_t *b, size_t o, uint64_t v) { for (int i = 0; i < 8; i++) b[o + i] = (uint8_t)(v >> (8 * i)); }

uint64_t gpt_first_usable_lba(void) { return 2048; }
uint64_t gpt_last_usable_lba(uint64_t disk_sectors) { return disk_sectors - 1 - ENTRY_SECTORS - 1; }

static void header(uint8_t *h, uint64_t disk_sectors, const uint8_t *guid, uint32_t entries_crc, bool primary)
{
    memset(h, 0, GPT_SECTOR);
    memcpy(h, "EFI PART", 8);
    p32(h, 8, 0x00010000u); p32(h, 12, 92);
    p64(h, 24, primary ? 1 : disk_sectors - 1);                         /* this header */
    p64(h, 32, primary ? disk_sectors - 1 : 1);                         /* the other one */
    p64(h, 40, gpt_first_usable_lba()); p64(h, 48, gpt_last_usable_lba(disk_sectors));
    memcpy(h + 56, guid, 16);
    p64(h, 72, primary ? 2 : disk_sectors - 1 - ENTRY_SECTORS);         /* entry array start */
    p32(h, 80, ENTRY_COUNT); p32(h, 84, ENTRY_SIZE); p32(h, 88, entries_crc);
    p32(h, 16, crc32(h, 92));
}

int gpt_write(uint64_t disk_sectors, const uint8_t disk_guid[16], const struct gpt_part *parts, int nparts, const uint8_t *boot_code,
              diskfmt_write_fn write, void *opaque)
{
    if (!write || nparts < 0 || nparts > GPT_MAX_PARTS || disk_sectors < 4096) return -22;
    uint64_t lo = gpt_first_usable_lba(), hi = gpt_last_usable_lba(disk_sectors);
    for (int i = 0; i < nparts; i++) {
        if (parts[i].first_lba < lo || parts[i].last_lba > hi || parts[i].first_lba > parts[i].last_lba) return -22;
        for (int j = 0; j < i; j++) if (parts[i].first_lba <= parts[j].last_lba && parts[j].first_lba <= parts[i].last_lba) return -22;
    }
    static uint8_t entries[ENTRY_SIZE * ENTRY_COUNT];
    memset(entries, 0, sizeof entries);
    for (int i = 0; i < nparts; i++) {
        uint8_t *e = entries + (size_t)i * ENTRY_SIZE;
        memcpy(e, parts[i].type_guid, 16); memcpy(e + 16, parts[i].unique_guid, 16);
        p64(e, 32, parts[i].first_lba); p64(e, 40, parts[i].last_lba);
        for (int k = 0; k < 35 && parts[i].name[k]; k++) e[56 + 2 * k] = (uint8_t)parts[i].name[k];
    }
    uint32_t ecrc = crc32(entries, sizeof entries);

    uint8_t mbr[GPT_SECTOR];                                            /* protective MBR: one 0xEE partition spanning the disk */
    memset(mbr, 0, sizeof mbr);
    if (boot_code) memcpy(mbr, boot_code, 440);
    uint8_t *pe = mbr + 446;
    pe[1] = 0; pe[2] = 2; pe[3] = 0; pe[4] = 0xee; pe[5] = 0xff; pe[6] = 0xff; pe[7] = 0xff;
    p32(pe, 8, 1); p32(pe, 12, disk_sectors - 1 > 0xffffffffull ? 0xffffffffu : (uint32_t)(disk_sectors - 1));
    mbr[510] = 0x55; mbr[511] = 0xaa;
    uint8_t h[GPT_SECTOR];
    if (write(opaque, 0, mbr, sizeof mbr)) return -5;
    header(h, disk_sectors, disk_guid, ecrc, true);
    if (write(opaque, 1 * GPT_SECTOR, h, sizeof h)) return -5;
    if (write(opaque, 2 * GPT_SECTOR, entries, sizeof entries)) return -5;
    if (write(opaque, (disk_sectors - 1 - ENTRY_SECTORS) * GPT_SECTOR, entries, sizeof entries)) return -5;
    header(h, disk_sectors, disk_guid, ecrc, false);
    if (write(opaque, (disk_sectors - 1) * GPT_SECTOR, h, sizeof h)) return -5;
    return 0;
}
