/* The disk registry: whichever controller finds a disk registers a struct blkdev here. */
#include "kernel.h"
#include <stdio.h>
#include <string.h>

#define MAX_DISKS 16
static struct blkdev disks[MAX_DISKS];
static int ndisks;

struct blkdev *blk_add(void)
{
    if (ndisks >= MAX_DISKS) return 0;
    struct blkdev *d = &disks[ndisks++];
    memset(d, 0, sizeof *d);
    d->sector_size_ok = 1;
    return d;
}
int blk_count(void) { return ndisks; }
struct blkdev *blk_get(int i) { return i >= 0 && i < ndisks ? &disks[i] : 0; }

/* legacy IDE disks (PIO) */
static int ata_blk_read(struct blkdev *d, uint64_t lba, uint32_t n, void *buf) { return ata_read(d->drv, lba, n, buf); }
static int ata_blk_write(struct blkdev *d, uint64_t lba, uint32_t n, const void *buf) { return ata_write(d->drv, lba, n, buf); }

int blk_init(void)
{
    ndisks = 0;
    int n = ata_init();
    for (int i = 0; i < n; i++) {
        struct ata_dev *a = ata_dev(i);
        struct blkdev *d = blk_add();
        if (!d) break;
        snprintf(d->name, sizeof d->name, "ide%d", i);
        snprintf(d->model, sizeof d->model, "%s", a->model);
        d->sectors = a->sectors; d->read = ata_blk_read; d->write = ata_blk_write; d->drv = a;
    }
    ahci_probe();
    nvme_probe();
    return ndisks;
}
