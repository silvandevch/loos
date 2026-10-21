/* AHCI (SATA) driver: polled, one command slot, DMA through a bounce buffer. Finds every SATA disk on every AHCI controller. */
#include "kernel.h"
#include <stdio.h>
#include <string.h>

#define HBA_GHC 0x04
#define HBA_PI 0x0c
#define PORT(i) (0x100u + 0x80u * (i))
#define P_CLB 0x00
#define P_FB 0x08
#define P_IS 0x10
#define P_CMD 0x18
#define P_TFD 0x20
#define P_SIG 0x24
#define P_SSTS 0x28
#define P_SERR 0x30
#define P_CI 0x38
#define CMD_ST 0x1u
#define CMD_FRE 0x10u
#define CMD_FR 0x4000u
#define CMD_CR 0x8000u
#define BOUNCE_SECTORS 128u

struct ahci_port {
    volatile uint8_t *regs;
    uint8_t *clb, *fb, *ctba;                 /* command list (1 slot used), FIS receive area, command table */
    uint8_t *bounce;
    uint64_t sectors;
};

static uint32_t rd(volatile uint8_t *base, uint32_t off) { return *(volatile uint32_t *)(base + off); }
static void wr(volatile uint8_t *base, uint32_t off, uint32_t v) { *(volatile uint32_t *)(base + off) = v; }

static int wait_clear(volatile uint8_t *regs, uint32_t off, uint32_t mask, uint32_t ms)
{
    uint32_t t0 = time_ms();
    while (rd(regs, off) & mask) if (time_ms() - t0 > ms) return -1;
    return 0;
}

static void port_stop(volatile uint8_t *r)
{
    wr(r, P_CMD, rd(r, P_CMD) & ~CMD_ST);
    wait_clear(r, P_CMD, CMD_CR, 500);
    wr(r, P_CMD, rd(r, P_CMD) & ~CMD_FRE);
    wait_clear(r, P_CMD, CMD_FR, 500);
}

/* one command with at most one data buffer (DMA address `buf`, `bytes` long) */
static int issue(struct ahci_port *p, uint8_t cmd, uint64_t lba, uint32_t count, uint8_t *buf, uint32_t bytes, int write)
{
    uint32_t *hdr = (uint32_t *)p->clb;
    hdr[0] = 5u | (write ? 1u << 6 : 0) | (bytes ? 1u << 16 : 0);          /* FIS length 5 dwords, W, one PRDT entry */
    hdr[1] = 0;
    hdr[2] = (uint32_t)(uintptr_t)p->ctba; hdr[3] = 0;
    uint8_t *t = p->ctba;
    memset(t, 0, 0x80 + 16);
    t[0] = 0x27; t[1] = 0x80; t[2] = cmd; t[7] = 0x40;                       /* host-to-device register FIS, command bit, LBA mode */
    t[4] = (uint8_t)lba; t[5] = (uint8_t)(lba >> 8); t[6] = (uint8_t)(lba >> 16);
    t[8] = (uint8_t)(lba >> 24); t[9] = (uint8_t)(lba >> 32); t[10] = (uint8_t)(lba >> 40);
    t[12] = (uint8_t)count; t[13] = (uint8_t)(count >> 8);
    if (bytes) {
        uint32_t *prd = (uint32_t *)(t + 0x80);
        prd[0] = (uint32_t)(uintptr_t)buf; prd[1] = 0; prd[2] = 0; prd[3] = bytes - 1;
    }
    wr(p->regs, P_IS, 0xffffffffu);
    wr(p->regs, P_CI, 1);
    uint32_t t0 = time_ms();
    for (;;) {
        if (!(rd(p->regs, P_CI) & 1)) break;
        if (rd(p->regs, P_IS) & (1u << 30)) return -1;                       /* task file error */
        if (time_ms() - t0 > 5000) return -1;
    }
    return (rd(p->regs, P_TFD) & 0x01) ? -1 : 0;
}

static int ahci_rw(struct blkdev *d, uint64_t lba, uint32_t n, void *buf, int write)
{
    struct ahci_port *p = d->drv;
    if (lba + n > p->sectors) return -1;
    uint8_t *b = buf;
    while (n) {
        uint32_t c = n > BOUNCE_SECTORS ? BOUNCE_SECTORS : n;
        if (write) memcpy(p->bounce, b, (size_t)c * 512);
        if (issue(p, write ? 0x35 : 0x25, lba, c, p->bounce, c * 512, write) != 0) return -1;     /* WRITE/READ DMA EXT */
        if (!write) memcpy(b, p->bounce, (size_t)c * 512);
        b += (size_t)c * 512; lba += c; n -= c;
    }
    if (write) { if (issue(p, 0xea, 0, 0, 0, 0, 0) != 0) return -1; }                              /* FLUSH CACHE EXT */
    return 0;
}
static int ahci_read(struct blkdev *d, uint64_t lba, uint32_t n, void *buf) { return ahci_rw(d, lba, n, buf, 0); }
static int ahci_write(struct blkdev *d, uint64_t lba, uint32_t n, const void *buf) { return ahci_rw(d, lba, n, (void *)(uintptr_t)buf, 1); }

static int start_port(volatile uint8_t *base, uint32_t idx, int *count)
{
    volatile uint8_t *r = base + PORT(idx);
    uint32_t ssts = rd(r, P_SSTS);
    if ((ssts & 0xf) != 3 || ((ssts >> 8) & 0xf) != 1) return 0;           /* no device / no communication */
    if (rd(r, P_SIG) != 0x00000101u) return 0;                             /* not a plain SATA disk (ATAPI, port multiplier, ...) */
    port_stop(r);
    struct ahci_port *p = kalloc(sizeof *p, 16);
    uint8_t *mem = kalloc(1024 + 256 + 256 + 4096 + 128 * 512, 4096);
    if (!p || !mem) return 0;
    memset(mem, 0, 1024 + 256 + 256);
    p->regs = r; p->clb = mem; p->fb = mem + 1024; p->ctba = mem + 1280; p->bounce = mem + 4096;
    wr(r, P_CLB, (uint32_t)(uintptr_t)p->clb); wr(r, P_CLB + 4, 0);
    wr(r, P_FB, (uint32_t)(uintptr_t)p->fb); wr(r, P_FB + 4, 0);
    wr(r, P_SERR, 0xffffffffu); wr(r, P_IS, 0xffffffffu);
    wr(r, P_CMD, rd(r, P_CMD) | CMD_FRE);
    wr(r, P_CMD, rd(r, P_CMD) | CMD_ST);
    uint16_t id[256];
    if (issue(p, 0xec, 0, 0, p->bounce, 512, 0) != 0) return 0;            /* IDENTIFY DEVICE */
    memcpy(id, p->bounce, 512);
    int lba48 = (id[83] & (1 << 10)) != 0;
    p->sectors = lba48 ? ((uint64_t)id[103] << 48 | (uint64_t)id[102] << 32 | (uint64_t)id[101] << 16 | id[100])
                       : ((uint64_t)id[61] << 16 | id[60]);
    if (!p->sectors || !lba48) return 0;
    struct blkdev *d = blk_add();
    if (!d) return 0;
    snprintf(d->name, sizeof d->name, "sata%d", *count);
    for (int i = 0; i < 20; i++) { d->model[2 * i] = (char)(id[27 + i] >> 8); d->model[2 * i + 1] = (char)(id[27 + i] & 0xff); }
    d->model[40] = 0;
    for (int i = 39; i >= 0 && (d->model[i] == ' ' || d->model[i] == 0); i--) d->model[i] = 0;
    d->sectors = p->sectors; d->read = ahci_read; d->write = ahci_write; d->drv = p;
    (*count)++;
    return 1;
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
        volatile uint8_t *base = (volatile uint8_t *)(uintptr_t)bar;
        wr(base, HBA_GHC, rd(base, HBA_GHC) | 0x80000000u);                 /* AHCI enable */
        uint32_t pi = rd(base, HBA_PI);
        for (uint32_t port = 0; port < 32; port++) if (pi & (1u << port)) start_port(base, port, &found);
    }
    return found;
}
