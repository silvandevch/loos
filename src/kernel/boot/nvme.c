/* NVMe driver: polled admin + one I/O queue pair, 512-byte-sector namespaces, transfers up to 64 KiB through a bounce buffer. */
#include "kernel.h"
#include <stdio.h>
#include <string.h>

#define QD 16u
#define BOUNCE_BYTES 65536u

struct nvme {
    volatile uint8_t *regs;
    uint32_t stride;
    uint8_t *asq, *acq, *isq, *icq;
    uint8_t *bounce, *prp_list;
    uint16_t asq_tail, acq_head, isq_tail, icq_head;
    uint8_t acq_phase, icq_phase;
    uint32_t nsid;
    uint64_t sectors;
    uint16_t cid;
};

static uint32_t rd32(struct nvme *n, uint32_t off) { return *(volatile uint32_t *)(n->regs + off); }
static void wr32(struct nvme *n, uint32_t off, uint32_t v) { *(volatile uint32_t *)(n->regs + off) = v; }
static uint64_t rd64(struct nvme *n, uint32_t off) { return *(volatile uint64_t *)(n->regs + off); }
static void wr64(struct nvme *n, uint32_t off, uint64_t v) { *(volatile uint64_t *)(n->regs + off) = v; }

static int wait_rdy(struct nvme *n, uint32_t want, uint32_t ms)
{
    uint32_t t0 = time_ms();
    while ((rd32(n, 0x1c) & 1) != want) if (time_ms() - t0 > ms) return -1;
    return 0;
}

/* submit one command (64 bytes, already built) on the admin (qid 0) or I/O (qid 1) queue and wait for its completion */
static int submit(struct nvme *n, int qid, const uint32_t cmd[16])
{
    uint8_t *sq = qid ? n->isq : n->asq, *cq = qid ? n->icq : n->acq;
    uint16_t *tail = qid ? &n->isq_tail : &n->asq_tail, *head = qid ? &n->icq_head : &n->acq_head;
    uint8_t *phase = qid ? &n->icq_phase : &n->acq_phase;
    uint32_t *slot = (uint32_t *)(sq + (size_t)*tail * 64);
    memcpy(slot, cmd, 64);
    slot[0] = (slot[0] & 0xffffu) | ((uint32_t)++n->cid << 16);
    *tail = (uint16_t)((*tail + 1) % QD);
    wr32(n, 0x1000 + (2u * (uint32_t)qid) * n->stride, *tail);                           /* SQ tail doorbell */
    uint32_t t0 = time_ms();
    for (;;) {
        volatile uint32_t *c = (volatile uint32_t *)(cq + (size_t)*head * 16);
        if (((c[3] >> 16) & 1) == *phase) {
            uint32_t status = c[3] >> 17;
            *head = (uint16_t)((*head + 1) % QD);
            if (*head == 0) *phase ^= 1;
            wr32(n, 0x1000 + (2u * (uint32_t)qid + 1) * n->stride, *head);               /* CQ head doorbell */
            return status ? -1 : 0;
        }
        if (time_ms() - t0 > 5000) return -1;
    }
}

static int rw(struct blkdev *d, uint64_t lba, uint32_t count, void *buf, int write)
{
    struct nvme *n = d->drv;
    if (lba + count > n->sectors) return -1;
    uint8_t *b = buf;
    while (count) {
        uint32_t c = count > BOUNCE_BYTES / 512 ? BOUNCE_BYTES / 512 : count;
        uint32_t bytes = c * 512, pages = (bytes + 4095) / 4096;
        if (write) memcpy(n->bounce, b, bytes);
        uint32_t cmd[16] = { 0 };
        cmd[0] = write ? 0x01 : 0x02; cmd[1] = n->nsid;
        uint64_t prp1 = (uint64_t)(uintptr_t)n->bounce;
        cmd[6] = (uint32_t)prp1; cmd[7] = (uint32_t)(prp1 >> 32);
        if (pages == 2) { uint64_t p2 = prp1 + 4096; cmd[8] = (uint32_t)p2; cmd[9] = (uint32_t)(p2 >> 32); }
        else if (pages > 2) {                                                              /* PRP list of the remaining pages */
            uint64_t *list = (uint64_t *)n->prp_list;
            for (uint32_t i = 1; i < pages; i++) list[i - 1] = prp1 + 4096ull * i;
            uint64_t p2 = (uint64_t)(uintptr_t)n->prp_list; cmd[8] = (uint32_t)p2; cmd[9] = (uint32_t)(p2 >> 32);
        }
        cmd[10] = (uint32_t)lba; cmd[11] = (uint32_t)(lba >> 32); cmd[12] = c - 1;
        if (submit(n, 1, cmd) != 0) return -1;
        if (!write) memcpy(b, n->bounce, bytes);
        b += bytes; lba += c; count -= c;
    }
    if (write) { uint32_t f[16] = { 0 }; f[0] = 0x00; f[1] = n->nsid; if (submit(n, 1, f) != 0) return -1; }   /* FLUSH */
    return 0;
}
static int nvme_read(struct blkdev *d, uint64_t lba, uint32_t c, void *buf) { return rw(d, lba, c, buf, 0); }
static int nvme_write(struct blkdev *d, uint64_t lba, uint32_t c, const void *buf) { return rw(d, lba, c, (void *)(uintptr_t)buf, 1); }

static int init_controller(struct pci_dev *pd, int *count)
{
    uint64_t size = 0, bar = pci_bar(pd, 0, &size);
    if (!bar || !size) return 0;
    if (vm_map_mmio(bar, size < 0x2000 ? 0x2000 : size) != 0) return 0;
    pci_enable(pd);
    struct nvme *n = kalloc(sizeof *n, 16);
    if (!n) return 0;
    memset(n, 0, sizeof *n);
    n->regs = (volatile uint8_t *)(uintptr_t)bar;
    uint64_t cap = rd64(n, 0);
    n->stride = 4u << ((cap >> 32) & 0xf);
    uint32_t to = (uint32_t)(((cap >> 24) & 0xff) * 500 + 500);
    wr32(n, 0x14, 0);                                                                    /* CC.EN = 0 */
    if (wait_rdy(n, 0, to) != 0) return 0;
    uint8_t *mem = kalloc(4096 * 4 + BOUNCE_BYTES + 4096, 4096);
    if (!mem) return 0;
    memset(mem, 0, 4096 * 4);
    n->asq = mem; n->acq = mem + 4096; n->isq = mem + 8192; n->icq = mem + 12288; n->bounce = mem + 16384; n->prp_list = mem + 16384 + BOUNCE_BYTES;
    n->acq_phase = n->icq_phase = 1;
    wr32(n, 0x24, (QD - 1) | ((QD - 1) << 16));
    wr64(n, 0x28, (uint64_t)(uintptr_t)n->asq); wr64(n, 0x30, (uint64_t)(uintptr_t)n->acq);
    wr32(n, 0x14, 1 | (6u << 16) | (4u << 20));                                          /* EN, 64-byte SQE, 16-byte CQE, 4 KiB pages */
    if (wait_rdy(n, 1, to) != 0) return 0;
    uint32_t cmd[16];
    memset(cmd, 0, sizeof cmd);                                                          /* create the I/O completion queue 1 */
    cmd[0] = 0x05; cmd[6] = (uint32_t)(uintptr_t)n->icq; cmd[10] = ((QD - 1) << 16) | 1; cmd[11] = 1;
    if (submit(n, 0, cmd) != 0) return 0;
    memset(cmd, 0, sizeof cmd);                                                          /* ...and the submission queue 1 */
    cmd[0] = 0x01; cmd[6] = (uint32_t)(uintptr_t)n->isq; cmd[10] = ((QD - 1) << 16) | 1; cmd[11] = (1u << 16) | 1;
    if (submit(n, 0, cmd) != 0) return 0;

    uint8_t *id = n->bounce;                                                             /* identify namespace 1 */
    memset(cmd, 0, sizeof cmd);
    cmd[0] = 0x06; cmd[1] = 1; cmd[6] = (uint32_t)(uintptr_t)id; cmd[10] = 0;
    if (submit(n, 0, cmd) != 0) return 0;
    n->nsid = 1;
    n->sectors = *(uint64_t *)id;
    uint8_t flbas = id[26] & 0xf;
    uint32_t lbaf = *(uint32_t *)(id + 128 + 4 * flbas);
    uint32_t lbads = (lbaf >> 16) & 0xff;
    int ok = lbads == 9;                                                                 /* 512-byte sectors only */
    char model[41] = "";
    memset(cmd, 0, sizeof cmd);                                                          /* identify controller for the model string */
    cmd[0] = 0x06; cmd[6] = (uint32_t)(uintptr_t)n->bounce; cmd[10] = 1;
    if (submit(n, 0, cmd) == 0) { memcpy(model, n->bounce + 24, 40); model[40] = 0; for (int i = 39; i >= 0 && (model[i] == ' ' || !model[i]); i--) model[i] = 0; }
    struct blkdev *d = blk_add();
    if (!d) return 0;
    snprintf(d->name, sizeof d->name, "nvme%d", *count);
    snprintf(d->model, sizeof d->model, "%s", model[0] ? model : "NVMe disk");
    d->sectors = ok ? n->sectors : n->sectors << (lbads - 9);
    d->sector_size_ok = ok;
    d->read = nvme_read; d->write = nvme_write; d->drv = n;
    (*count)++;
    return 1;
}

int nvme_probe(void)
{
    static struct pci_dev devs[128];
    int n = pci_scan(devs, 128), found = 0;
    for (int i = 0; i < n; i++)
        if (devs[i].class_code == 0x01 && devs[i].subclass == 0x08 && devs[i].prog_if == 0x02) init_controller(&devs[i], &found);
    return found;
}
