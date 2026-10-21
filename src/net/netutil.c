#include "netutil.h"
#include <string.h>
#include <stdio.h>

bool ip4_parse(const char *s, ip4_addr *out)
{
    ip4_addr a;
    for (int i = 0; i < 4; i++) {
        if (*s < '0' || *s > '9') return false;
        unsigned v = 0, digits = 0;
        bool lead_zero = *s == '0';
        while (*s >= '0' && *s <= '9') { v = v * 10 + (unsigned)(*s++ - '0'); if (++digits > 3 || v > 255) return false; }
        if (lead_zero && digits > 1) return false;
        a.b[i] = (uint8_t)v;
        if (i < 3) { if (*s++ != '.') return false; }
    }
    if (*s) return false;
    *out = a;
    return true;
}

size_t ip4_format(ip4_addr a, char *out, size_t cap)
{
    int n = snprintf(out, cap, "%u.%u.%u.%u", a.b[0], a.b[1], a.b[2], a.b[3]);
    return n < 0 ? 0 : (size_t)n;
}

static int hexval(int c) { return c >= '0' && c <= '9' ? c - '0' : c >= 'a' && c <= 'f' ? c - 'a' + 10 : c >= 'A' && c <= 'F' ? c - 'A' + 10 : -1; }

bool ip6_parse(const char *s, ip6_addr *out)
{
    uint16_t words[8]; int nw = 0, gap = -1;
    memset(words, 0, sizeof words);
    if (s[0] == ':' && s[1] == ':') { gap = 0; s += 2; if (!*s) { memset(out, 0, sizeof *out); return true; } }
    else if (s[0] == ':') return false;
    for (;;) {
        /* embedded IPv4 as the last 32 bits */
        const char *dot = s; while (*dot && *dot != ':' && *dot != '.') dot++;
        if (*dot == '.') {
            ip4_addr v4;
            if (nw > 6 || !ip4_parse(s, &v4)) return false;
            words[nw++] = (uint16_t)(v4.b[0] << 8 | v4.b[1]); words[nw++] = (uint16_t)(v4.b[2] << 8 | v4.b[3]);
            s += strlen(s);
            break;
        }
        unsigned v = 0, digits = 0;
        while (hexval(*s) >= 0) { v = v * 16 + (unsigned)hexval(*s++); if (++digits > 4) return false; }
        if (!digits || nw >= 8) return false;
        words[nw++] = (uint16_t)v;
        if (!*s) break;
        if (*s != ':') return false;
        s++;
        if (*s == ':') { if (gap >= 0) return false; gap = nw; s++; if (!*s) break; }
        else if (!*s) return false;
    }
    if (gap < 0 ? nw != 8 : nw > 7) return false;
    uint16_t full[8]; memset(full, 0, sizeof full);
    if (gap < 0) memcpy(full, words, sizeof full);
    else { memcpy(full, words, (size_t)gap * 2); memcpy(full + 8 - (nw - gap), words + gap, (size_t)(nw - gap) * 2); }
    for (int i = 0; i < 8; i++) net_wr16(out->b + 2 * i, full[i]);
    return true;
}

size_t ip6_format(const ip6_addr *a, char *out, size_t cap)
{
    uint16_t w[8];
    for (int i = 0; i < 8; i++) w[i] = net_rd16(a->b + 2 * i);
    int best = -1, best_len = 0;
    for (int i = 0; i < 8;) {                                         /* longest run of zero words (>= 2) */
        if (w[i]) { i++; continue; }
        int j = i; while (j < 8 && !w[j]) j++;
        if (j - i > best_len) { best = i; best_len = j - i; }
        i = j;
    }
    if (best_len < 2) best = -1;
    char tmp[48]; size_t n = 0;
    for (int i = 0; i < 8; i++) {
        if (i == best) { tmp[n++] = ':'; if (i == 0) tmp[n++] = ':'; i += best_len - 1; continue; }
        n += (size_t)snprintf(tmp + n, sizeof tmp - n, "%x", w[i]);
        if (i < 7) tmp[n++] = ':';
    }
    tmp[n] = 0;
    if (cap) { strncpy(out, tmp, cap - 1); out[cap - 1] = 0; }
    return n;
}

static bool parse_prefix(const char *s, unsigned max, uint8_t *out)
{
    if (*s < '0' || *s > '9') return false;
    unsigned v = 0;
    while (*s >= '0' && *s <= '9') { v = v * 10 + (unsigned)(*s++ - '0'); if (v > max) return false; }
    if (*s) return false;
    *out = (uint8_t)v;
    return true;
}
bool ip4_cidr_parse(const char *s, ip4_cidr *out)
{
    char buf[40]; const char *slash = strchr(s, '/');
    size_t n = slash ? (size_t)(slash - s) : strlen(s);
    if (n >= sizeof buf) return false;
    memcpy(buf, s, n); buf[n] = 0;
    ip4_cidr c;
    if (!ip4_parse(buf, &c.addr)) return false;
    c.prefix = 32;
    if (slash && !parse_prefix(slash + 1, 32, &c.prefix)) return false;
    *out = c;
    return true;
}
bool ip6_cidr_parse(const char *s, ip6_cidr *out)
{
    char buf[64]; const char *slash = strchr(s, '/');
    size_t n = slash ? (size_t)(slash - s) : strlen(s);
    if (n >= sizeof buf) return false;
    memcpy(buf, s, n); buf[n] = 0;
    ip6_cidr c;
    if (!ip6_parse(buf, &c.addr)) return false;
    c.prefix = 128;
    if (slash && !parse_prefix(slash + 1, 128, &c.prefix)) return false;
    *out = c;
    return true;
}

uint32_t ip4_mask(unsigned prefix) { return prefix == 0 ? 0 : prefix >= 32 ? 0xffffffffu : ~(0xffffffffu >> prefix); }
bool ip4_in_cidr(ip4_addr a, ip4_cidr c) { uint32_t m = ip4_mask(c.prefix); return (ip4_u32(a) & m) == (ip4_u32(c.addr) & m); }
bool ip6_in_cidr(const ip6_addr *a, ip6_cidr c)
{
    unsigned full = c.prefix / 8, rem = c.prefix % 8;
    if (memcmp(a->b, c.addr.b, full)) return false;
    if (!rem) return true;
    uint8_t m = (uint8_t)(0xff << (8 - rem));
    return (a->b[full] & m) == (c.addr.b[full] & m);
}
bool ip4_is_private(ip4_addr a)
{
    uint32_t v = ip4_u32(a);
    return (v >> 24) == 10 || (v >> 20) == 0xac1 || (v >> 16) == 0xc0a8;
}
bool ip4_is_loopback(ip4_addr a) { return a.b[0] == 127; }
bool ip4_is_link_local(ip4_addr a) { return a.b[0] == 169 && a.b[1] == 254; }
bool ip4_is_multicast(ip4_addr a) { return (a.b[0] & 0xf0) == 0xe0; }
int ip4_prefix_from_mask(ip4_addr mask)
{
    uint32_t m = ip4_u32(mask);
    int p = 0;
    while (p < 32 && (m & (0x80000000u >> p))) p++;
    return (m << p) == 0 || p == 32 ? p : -1;
}

uint32_t net_checksum_add(const void *data, size_t len, uint32_t acc)
{
    const uint8_t *p = data;
    while (len > 1) { acc += (uint32_t)(p[0] << 8 | p[1]); p += 2; len -= 2; }
    if (len) acc += (uint32_t)(p[0] << 8);
    return acc;
}
uint16_t net_checksum_finish(uint32_t acc)
{
    while (acc >> 16) acc = (acc & 0xffff) + (acc >> 16);
    return (uint16_t)~acc;
}
uint16_t net_checksum(const void *data, size_t len, uint32_t seed) { return net_checksum_finish(net_checksum_add(data, len, seed)); }
