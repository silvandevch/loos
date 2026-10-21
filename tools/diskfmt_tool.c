/* Host tool: builds a UEFI disk the way the installer does (GPT, FAT32 ESP with EFI\BOOT and LOOS directories, btrfs data
 * partition) so the tests can run sgdisk, fsck.fat, mtools and btrfs check on the result.
 *   diskfmt_tool IMAGE SIZE_MIB ESP_MIB BOOTX64.EFI=path KERNEL.ELF=path ... */
#define _POSIX_C_SOURCE 200809L
#include "diskfmt.h"
#include "btrfs.h"
#include <fcntl.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

static int fd;
static int wr(void *o, uint64_t off, const void *d, size_t n) { (void)o; return pwrite(fd, d, n, (off_t)off) == (ssize_t)n ? 0 : -1; }
static int btrfs_wr(void *base, uint64_t off, const void *d, size_t n) { return wr(0, *(uint64_t *)base + off, d, n); }
static int file_read(void *o, uint64_t off, void *buf, size_t n) { return pread(*(int *)o, buf, n, (off_t)off) == (ssize_t)n ? 0 : -1; }
static void rnd(uint8_t *b, size_t n) { FILE *r = fopen("/dev/urandom", "rb"); if (!r || fread(b, 1, n, r) != n) exit(1); fclose(r); }

int main(int argc, char **argv)
{
    if (argc < 4) { fprintf(stderr, "usage: diskfmt_tool IMAGE SIZE_MIB ESP_MIB NAME=path...\n"); return 2; }
    uint64_t sectors = (strtoull(argv[2], 0, 10) << 20) / 512, esp = (strtoull(argv[3], 0, 10) << 20) / 512;
    fd = open(argv[1], O_RDWR | O_CREAT | O_TRUNC, 0644);
    if (fd < 0 || ftruncate(fd, (off_t)(sectors * 512)) != 0) return 1;
    struct gpt_part parts[2];
    memset(parts, 0, sizeof parts);
    uint64_t lo = gpt_first_usable_lba(), hi = gpt_last_usable_lba(sectors);
    memcpy(parts[0].type_guid, GPT_TYPE_ESP, 16); rnd(parts[0].unique_guid, 16);
    parts[0].first_lba = lo; parts[0].last_lba = lo + esp - 1; strcpy(parts[0].name, "EFI System");
    memcpy(parts[1].type_guid, GPT_TYPE_LINUX_DATA, 16); rnd(parts[1].unique_guid, 16);
    parts[1].first_lba = (lo + esp + 2047) & ~2047ull; parts[1].last_lba = hi; strcpy(parts[1].name, "LoOS data");
    uint8_t guid[16]; rnd(guid, 16);
    if (gpt_write(sectors, guid, parts, 2, 0, wr, 0)) { fprintf(stderr, "gpt failed\n"); return 1; }

    static struct fat32 fs;
    uint32_t *fat = malloc((size_t)fat32_fat_entries(esp * 512) * 4);
    if (fat32_format(&fs, wr, 0, lo * 512, esp * 512, 0x4c6f4f53u, "LOOSESP", 1700000000, fat)) { fprintf(stderr, "fat32 format failed\n"); return 1; }
    uint32_t efi = fat32_mkdir(&fs, 2, "EFI"), boot = fat32_mkdir(&fs, efi, "BOOT"), loos = fat32_mkdir(&fs, 2, "LOOS");
    for (int i = 4; i < argc; i++) {
        char *eq = strchr(argv[i], '='); if (!eq) return 2;
        *eq = 0;
        int f = open(eq + 1, O_RDONLY); if (f < 0) { perror(eq + 1); return 1; }
        off_t size = lseek(f, 0, SEEK_END);
        uint32_t dir = !strcmp(argv[i], "BOOTX64.EFI") ? boot : loos;
        if (fat32_write_file(&fs, dir, argv[i], (uint64_t)size, file_read, &f, 0)) { fprintf(stderr, "write %s failed\n", argv[i]); return 1; }
        close(f);
    }
    if (fat32_finish(&fs)) { fprintf(stderr, "finish failed\n"); return 1; }

    struct loos_btrfs_mkfs_params p;
    memset(&p, 0, sizeof p);
    p.device_bytes = (parts[1].last_lba - parts[1].first_lba + 1) * 512; p.label = "LoOS"; p.unix_time = 1700000000;
    rnd(p.fsid, 16); rnd(p.dev_uuid, 16); rnd(p.chunk_tree_uuid, 16); rnd(p.subvol_uuid, 16);
    uint64_t base = parts[1].first_lba * 512;
    if (loos_btrfs_mkfs(&p, btrfs_wr, &base)) { fprintf(stderr, "btrfs mkfs failed\n"); return 1; }
    printf("%llu %llu\n", (unsigned long long)(lo * 512), (unsigned long long)base);      /* ESP offset, data partition offset */
    close(fd);
    return 0;
}
