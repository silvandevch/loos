/* AHCI and NVMe disks, driven by the Rust drivers (crates/loos-drivers) through crates/kffi: this file only finds the
 * controllers on PCI, hands them mapped registers and DMA memory, and registers each disk with the block layer. */
#include "kernel.h"
#include <stdio.h>
#include <string.h>

extern int32_t loos_ahci_attach(uint64_t mmio, void *dma, uint64_t phys, uint64_t len);
extern int32_t loos_ahci_disks(int32_t ctl);
extern int32_t loos_ahci_info(int32_t ctl, int32_t disk, uint64_t *sectors, char *model, uint32_t cap);
extern int32_t loos_ahci_rw(int32_t ctl, int32_t disk, uint64_t lba, uint32_t n, void *buf, int32_t write);
extern int32_t loos_nvme_attach(uint64_t mmio, void *dma, uint64_t phys, uint64_t len);
extern int32_t loos_nvme_info(int32_t ctl, uint64_t *sectors, uint32_t *sector_size, char *model, uint32_t cap);
extern int32_t loos_nvme_rw(int32_t ctl, uint64_t lba, uint32_t n, void *buf, int32_t write);

#define AHCI_ARENA (0x11000u * 8)                 /* loos_drivers::ahci::PORT_REGION per possible disk */
#define NVME_ARENA 0x26000u                       /* loos_drivers::nvme::ARENA_BYTES */

struct rdisk { int32_t ctl, disk; };

static int ahci_read(struct blkdev *d, uint64_t lba, uint32_t n, void *buf) { struct rdisk *r = d->drv; return loos_ahci_rw(r->ctl, r->disk, lba, n, buf, 0); }
static int ahci_write(struct blkdev *d, uint64_t lba, uint32_t n, const void *buf) { struct rdisk *r = d->drv; return loos_ahci_rw(r->ctl, r->disk, lba, n, (void *)(uintptr_t)buf, 1); }
static int nvme_read(struct blkdev *d, uint64_t lba, uint32_t n, void *buf) { struct rdisk *r = d->drv; return loos_nvme_rw(r->ctl, lba, n, buf, 0); }
static int nvme_write(struct blkdev *d, uint64_t lba, uint32_t n, const void *buf) { struct rdisk *r = d->drv; return loos_nvme_rw(r->ctl, lba, n, (void *)(uintptr_t)buf, 1); }

static void *arena(uint64_t len)
{
    void *p = kalloc(len, 4096);
    if (p) memset(p, 0, len);
    return p;
}

int ahci_probe(void)
{
    static struct pci_dev devs[128];
    int n = pci_scan(devs, 128), found = 0;
    for (int i = 0; i < n; i++) {
        if (devs[i].class_code != 0x01 || devs[i].subclass != 0x06 || devs[i].prog_if != 0x01) continue;
        uint64_t size = 0, bar = pci_bar(&devs[i], 5, &size);
        if (!bar || !size) continue;
        if (vm_map_mmio(bar, size < 0x2000 ? 0x2000 : size) != 0) continue;
        pci_enable(&devs[i]);
        void *dma = arena(AHCI_ARENA);
        if (!dma) continue;
        int32_t ctl = loos_ahci_attach(bar, dma, (uint64_t)(uintptr_t)dma, AHCI_ARENA);
        if (ctl < 0) { serial_puts("ahci: controller init failed\n"); continue; }
        for (int32_t k = 0, nd = loos_ahci_disks(ctl); k < nd; k++) {
            struct blkdev *d = blk_add();
            struct rdisk *r = kalloc(sizeof *r, 8);
            if (!d || !r) break;
            r->ctl = ctl; r->disk = k;
            snprintf(d->name, sizeof d->name, "sata%d", found);
            loos_ahci_info(ctl, k, &d->sectors, d->model, sizeof d->model);
            d->read = ahci_read; d->write = ahci_write; d->drv = r;
            found++;
        }
    }
    return found;
}

int nvme_probe(void)
{
    static struct pci_dev devs[128];
    int n = pci_scan(devs, 128), found = 0;
    for (int i = 0; i < n; i++) {
        if (devs[i].class_code != 0x01 || devs[i].subclass != 0x08 || devs[i].prog_if != 0x02) continue;
        uint64_t size = 0, bar = pci_bar(&devs[i], 0, &size);
        if (!bar || !size) continue;
        if (vm_map_mmio(bar, size < 0x2000 ? 0x2000 : size) != 0) continue;
        pci_enable(&devs[i]);
        void *dma = arena(NVME_ARENA);
        if (!dma) continue;
        int32_t ctl = loos_nvme_attach(bar, dma, (uint64_t)(uintptr_t)dma, NVME_ARENA);
        if (ctl < 0) { serial_puts("nvme: controller init failed\n"); continue; }
        struct blkdev *d = blk_add();
        struct rdisk *r = kalloc(sizeof *r, 8);
        if (!d || !r) continue;
        uint32_t ss = 0;
        r->ctl = ctl; r->disk = 0;
        snprintf(d->name, sizeof d->name, "nvme%d", found);
        loos_nvme_info(ctl, &d->sectors, &ss, d->model, sizeof d->model);
        d->sector_size_ok = ss == 512;
        d->read = nvme_read; d->write = nvme_write; d->drv = r;
        found++;
    }
    return found;
}
