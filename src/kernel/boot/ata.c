/* ATA PIO driver (legacy IDE I/O ports): identify, read and write LBA sectors on the four classic channels. Enough to
 * install LoOS from one disk to another; AHCI/NVMe drivers replace it for real hardware (see docs/DRIVERS.md). */
#include "kernel.h"

#define ST_ERR 0x01
#define ST_DRQ 0x08
#define ST_DF  0x20
#define ST_BSY 0x80

static struct ata_dev devs[4];
static int ndevs;

static inline void insw(uint16_t port, void *buf, int count)
{
    __asm__ volatile("rep insw" : "+D"(buf), "+c"(count) : "d"(port) : "memory");
}
static inline void outsw(uint16_t port, const void *buf, int count)
{
    __asm__ volatile("rep outsw" : "+S"(buf), "+c"(count) : "d"(port) : "memory");
}

static void delay400(uint16_t ctl) { for (int i = 0; i < 4; i++) (void)inb(ctl); }

static int wait_not_busy(uint16_t io, uint32_t ms)
{
    uint32_t t0 = time_ms();
    for (;;) {
        uint8_t s = inb(io + 7);
        if (!(s & ST_BSY)) return s;
        if (time_ms() - t0 > ms) return -1;
    }
}

static int wait_drq(uint16_t io, uint16_t ctl, uint32_t ms)
{
    delay400(ctl);
    uint32_t t0 = time_ms();
    for (;;) {
        uint8_t s = inb(io + 7);
        if (s & (ST_ERR | ST_DF)) return -1;
        if (!(s & ST_BSY) && (s & ST_DRQ)) return 0;
        if (time_ms() - t0 > ms) return -1;
    }
}

static void select_drive(const struct ata_dev *d, uint8_t extra)
{
    outb(d->io + 6, (uint8_t)(0xa0 | extra | (d->slave << 4)));
    delay400(d->ctl);
}

static int identify(struct ata_dev *d)
{
    select_drive(d, 0);
    for (int r = 2; r <= 5; r++) outb(d->io + r, 0);
    outb(d->io + 7, 0xec);
    uint8_t s = inb(d->io + 7);
    if (s == 0 || s == 0xff) return -1;                                   /* nothing there (or a floating bus) */
    if (wait_not_busy(d->io, 1000) < 0) return -1;
    if (inb(d->io + 4) || inb(d->io + 5)) return -1;                       /* ATAPI / SATA signature: not a plain ATA disk */
    if (wait_drq(d->io, d->ctl, 1000) < 0) return -1;
    uint16_t id[256];
    insw(d->io, id, 256);
    d->lba48 = (id[83] & (1 << 10)) != 0;
    d->sectors = d->lba48 ? ((uint64_t)id[103] << 48 | (uint64_t)id[102] << 32 | (uint64_t)id[101] << 16 | id[100])
                          : ((uint64_t)id[61] << 16 | id[60]);
    for (int i = 0; i < 20; i++) { d->model[2 * i] = (char)(id[27 + i] >> 8); d->model[2 * i + 1] = (char)(id[27 + i] & 0xff); }
    d->model[40] = 0;
    for (int i = 39; i >= 0 && (d->model[i] == ' ' || d->model[i] == 0); i--) d->model[i] = 0;   /* trim padding */
    return d->sectors ? 0 : -1;
}

int ata_init(void)
{
    static const struct { uint16_t io, ctl; } chan[2] = { { 0x1f0, 0x3f6 }, { 0x170, 0x376 } };
    ndevs = 0;
    for (int c = 0; c < 2; c++)
        for (int s = 0; s < 2; s++) {
            struct ata_dev *d = &devs[ndevs];
            d->io = chan[c].io; d->ctl = chan[c].ctl; d->slave = s; d->channel = c;
            outb(d->ctl, 0);                                               /* enable interrupts off (polling), no reset */
            if (identify(d) == 0) { d->index = ndevs; ndevs++; }
        }
    return ndevs;
}

int ata_count(void) { return ndevs; }
struct ata_dev *ata_dev(int i) { return i >= 0 && i < ndevs ? &devs[i] : 0; }

static void setup_lba(const struct ata_dev *d, uint64_t lba, uint32_t count, int write)
{
    if (d->lba48) {
        select_drive(d, 0x40);
        outb(d->io + 2, (uint8_t)(count >> 8)); outb(d->io + 3, (uint8_t)(lba >> 24)); outb(d->io + 4, (uint8_t)(lba >> 32)); outb(d->io + 5, (uint8_t)(lba >> 40));
        outb(d->io + 2, (uint8_t)count); outb(d->io + 3, (uint8_t)lba); outb(d->io + 4, (uint8_t)(lba >> 8)); outb(d->io + 5, (uint8_t)(lba >> 16));
        outb(d->io + 7, write ? 0x34 : 0x24);
    } else {
        select_drive(d, (uint8_t)(0x40 | ((lba >> 24) & 0x0f)));
        outb(d->io + 2, (uint8_t)count); outb(d->io + 3, (uint8_t)lba); outb(d->io + 4, (uint8_t)(lba >> 8)); outb(d->io + 5, (uint8_t)(lba >> 16));
        outb(d->io + 7, write ? 0x30 : 0x20);
    }
}

int ata_read(const struct ata_dev *d, uint64_t lba, uint32_t count, void *buf)
{
    if (lba + count > d->sectors) return -1;
    uint8_t *p = buf;
    while (count) {
        uint32_t n = count > 128 ? 128 : count;
        if (wait_not_busy(d->io, 1000) < 0) return -1;
        setup_lba(d, lba, n, 0);
        for (uint32_t i = 0; i < n; i++) {
            if (wait_drq(d->io, d->ctl, 30000) < 0) return -1;
            insw(d->io, p, 256);
            p += 512;
        }
        lba += n; count -= n;
    }
    return 0;
}

int ata_write(const struct ata_dev *d, uint64_t lba, uint32_t count, const void *buf)
{
    if (lba + count > d->sectors) return -1;
    const uint8_t *p = buf;
    while (count) {
        uint32_t n = count > 128 ? 128 : count;
        if (wait_not_busy(d->io, 30000) < 0) return -1;
        setup_lba(d, lba, n, 1);
        for (uint32_t i = 0; i < n; i++) {
            if (wait_drq(d->io, d->ctl, 30000) < 0) return -1;
            outsw(d->io, p, 256);
            p += 512;
        }
        if (wait_not_busy(d->io, 30000) < 0) return -1;
        lba += n; count -= n;
    }
    outb(d->io + 7, d->lba48 ? 0xea : 0xe7);                               /* flush the drive's write cache */
    return wait_not_busy(d->io, 5000) < 0 ? -1 : 0;
}
