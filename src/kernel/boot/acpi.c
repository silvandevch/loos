/* Minimal ACPI table reader: RSDP -> (X)RSDT -> MADT, for processor enumeration. Everything is validated
 * (signature, checksum, length bounds) because firmware tables are not trusted input. */
#include "kernel.h"

struct sdt { char sig[4]; uint32_t length; uint8_t rev, checksum; char oem[6], oemtab[8]; uint32_t oemrev, creator, creatorrev; };

static uint8_t sum8(const void *p, size_t n) { const uint8_t *b = p; uint8_t s = 0; while (n--) s = (uint8_t)(s + *b++); return s; }
static int sig_is(const char *s, const char *w) { return s[0] == w[0] && s[1] == w[1] && s[2] == w[2] && s[3] == w[3]; }

static const struct sdt *checked_sdt(uint64_t phys)
{
    if (!phys || phys >= 0x100000000ull) return 0;          /* only the identity-mapped window is reachable */
    const struct sdt *t = (const struct sdt *)(uintptr_t)phys;
    if (t->length < sizeof *t || t->length > 1u << 20 || sum8(t, t->length) != 0) return 0;
    return t;
}

int acpi_parse_madt(uint64_t rsdp_phys, struct acpi_cpus *out)
{
    out->count = 0; out->lapic_phys = 0xfee00000; out->ok = 0;
    if (!rsdp_phys) return -1;
    const uint8_t *r = (const uint8_t *)(uintptr_t)rsdp_phys;
    if (r[0] != 'R' || r[1] != 'S' || sum8(r, 20) != 0) return -1;
    int xsdt = r[15] >= 2 && sum8(r, 36) == 0;
    uint64_t root = xsdt ? *(const uint64_t *)(r + 24) : *(const uint32_t *)(r + 16);
    const struct sdt *rt = checked_sdt(root);
    if (!rt) return -1;
    size_t esz = xsdt ? 8 : 4, n = (rt->length - sizeof *rt) / esz;
    const uint8_t *ent = (const uint8_t *)rt + sizeof *rt;
    for (size_t i = 0; i < n; i++) {
        uint64_t a = xsdt ? ((const uint64_t *)ent)[i] : ((const uint32_t *)ent)[i];
        const struct sdt *t = checked_sdt(a);
        if (!t || !sig_is(t->sig, "APIC") || t->length < sizeof *t + 8) continue;
        const uint8_t *p = (const uint8_t *)t + sizeof *t;
        const uint8_t *end = (const uint8_t *)t + t->length;
        out->lapic_phys = *(const uint32_t *)p;
        for (p += 8; p + 2 <= end && p[1] >= 2 && p + p[1] <= end; p += p[1]) {
            if (p[0] == 0 && p[1] >= 8) {                    /* processor local APIC */
                uint32_t flags = *(const uint32_t *)(p + 4);
                if ((flags & 3) && out->count < ACPI_MAX_CPUS) out->apic_id[out->count++] = p[3];
            } else if (p[0] == 5 && p[1] >= 12) {            /* local APIC address override */
                out->lapic_phys = *(const uint64_t *)(p + 4);
            }
        }
        out->ok = 1;
        return 0;
    }
    return -1;
}
