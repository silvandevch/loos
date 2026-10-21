/* PCI configuration space through the legacy 0xCF8/0xCFC mechanism. */
#include "kernel.h"

static uint32_t addr(uint8_t bus, uint8_t dev, uint8_t fn, uint8_t off)
{
    return 0x80000000u | (uint32_t)bus << 16 | (uint32_t)dev << 11 | (uint32_t)fn << 8 | (off & 0xfc);
}

static uint32_t cfg_read(uint8_t bus, uint8_t dev, uint8_t fn, uint8_t off) { outl(0xcf8, addr(bus, dev, fn, off)); return inl(0xcfc); }
static void cfg_write(uint8_t bus, uint8_t dev, uint8_t fn, uint8_t off, uint32_t v) { outl(0xcf8, addr(bus, dev, fn, off)); outl(0xcfc, v); }

uint32_t pci_read32(const struct pci_dev *d, uint8_t off) { return cfg_read(d->bus, d->dev, d->fn, off); }
void pci_write32(const struct pci_dev *d, uint8_t off, uint32_t v) { cfg_write(d->bus, d->dev, d->fn, off, v); }

int pci_scan(struct pci_dev *out, int max)
{
    int n = 0;
    for (int bus = 0; bus < 256; bus++)
        for (int dev = 0; dev < 32; dev++) {
            uint32_t id = cfg_read((uint8_t)bus, (uint8_t)dev, 0, 0);
            if ((id & 0xffff) == 0xffff) continue;
            uint8_t hdr = (uint8_t)(cfg_read((uint8_t)bus, (uint8_t)dev, 0, 0x0c) >> 16);
            int fns = (hdr & 0x80) ? 8 : 1;
            for (int fn = 0; fn < fns; fn++) {
                id = cfg_read((uint8_t)bus, (uint8_t)dev, (uint8_t)fn, 0);
                if ((id & 0xffff) == 0xffff || n >= max) continue;
                uint32_t cls = cfg_read((uint8_t)bus, (uint8_t)dev, (uint8_t)fn, 8);
                out[n++] = (struct pci_dev){ (uint8_t)bus, (uint8_t)dev, (uint8_t)fn, (uint16_t)(id & 0xffff), (uint16_t)(id >> 16),
                                             (uint8_t)(cls >> 24), (uint8_t)(cls >> 16), (uint8_t)(cls >> 8) };
            }
        }
    return n;
}

uint64_t pci_bar(const struct pci_dev *d, int index, uint64_t *size)
{
    uint8_t off = (uint8_t)(0x10 + 4 * index);
    uint32_t lo = pci_read32(d, off);
    if (lo & 1) return 0;                                                  /* I/O BAR */
    int is64 = ((lo >> 1) & 3) == 2;
    uint32_t hi = is64 ? pci_read32(d, (uint8_t)(off + 4)) : 0;
    if (size) {                                                            /* size the BAR: write all ones, read back, restore */
        uint16_t cmd = (uint16_t)pci_read32(d, 4);
        pci_write32(d, 4, cmd & ~7u);                                      /* decode off while probing */
        pci_write32(d, off, 0xffffffffu);
        uint32_t m_lo = pci_read32(d, off), m_hi = 0xffffffffu;
        if (is64) { pci_write32(d, (uint8_t)(off + 4), 0xffffffffu); m_hi = pci_read32(d, (uint8_t)(off + 4)); }
        pci_write32(d, off, lo);
        if (is64) pci_write32(d, (uint8_t)(off + 4), hi);
        pci_write32(d, 4, (pci_read32(d, 4) & ~0xffffu) | cmd);
        uint64_t mask = ((uint64_t)m_hi << 32 | m_lo) & ~0xfull;
        *size = (~mask + 1) & (is64 ? ~0ull : 0xffffffffull);
    }
    return ((uint64_t)hi << 32 | lo) & ~0xfull;
}

void pci_enable(const struct pci_dev *d)
{
    uint32_t cmd = pci_read32(d, 4);
    pci_write32(d, 4, (cmd & 0xffffu) | 0x6u);                             /* memory space + bus master */
}
