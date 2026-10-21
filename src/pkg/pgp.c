#include "pgp.h"
#include "../security/crypto.h"
#include <stdlib.h>
#include <string.h>

/* ------------------------------------------------------------------------------------------ hashes ---- */
typedef struct { uint32_t h[5]; uint64_t bits; uint8_t buf[64]; size_t n; } sha1_t;
static uint32_t rol32(uint32_t x, int r) { return (x << r) | (x >> (32 - r)); }
static void sha1_block(sha1_t *c, const uint8_t *p)
{
    uint32_t w[80], a = c->h[0], b = c->h[1], cc = c->h[2], d = c->h[3], e = c->h[4];
    for (int i = 0; i < 16; i++) w[i] = (uint32_t)p[4 * i] << 24 | (uint32_t)p[4 * i + 1] << 16 | (uint32_t)p[4 * i + 2] << 8 | p[4 * i + 3];
    for (int i = 16; i < 80; i++) w[i] = rol32(w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16], 1);
    for (int i = 0; i < 80; i++) {
        uint32_t f, k;
        if (i < 20) { f = (b & cc) | (~b & d); k = 0x5a827999u; } else if (i < 40) { f = b ^ cc ^ d; k = 0x6ed9eba1u; }
        else if (i < 60) { f = (b & cc) | (b & d) | (cc & d); k = 0x8f1bbcdcu; } else { f = b ^ cc ^ d; k = 0xca62c1d6u; }
        uint32_t t = rol32(a, 5) + f + e + k + w[i];
        e = d; d = cc; cc = rol32(b, 30); b = a; a = t;
    }
    c->h[0] += a; c->h[1] += b; c->h[2] += cc; c->h[3] += d; c->h[4] += e;
}
static void sha1_init(sha1_t *c) { c->h[0] = 0x67452301u; c->h[1] = 0xefcdab89u; c->h[2] = 0x98badcfeu; c->h[3] = 0x10325476u; c->h[4] = 0xc3d2e1f0u; c->bits = 0; c->n = 0; }
static void sha1_update(sha1_t *c, const void *d, size_t len)
{
    const uint8_t *p = d; c->bits += (uint64_t)len * 8;
    while (len) { size_t k = 64 - c->n < len ? 64 - c->n : len; memcpy(c->buf + c->n, p, k); c->n += k; p += k; len -= k; if (c->n == 64) { sha1_block(c, c->buf); c->n = 0; } }
}
static void sha1_final(sha1_t *c, uint8_t out[20])
{
    uint64_t bits = c->bits; uint8_t pad = 0x80;
    sha1_update(c, &pad, 1); pad = 0;
    while (c->n != 56) sha1_update(c, &pad, 1);
    uint8_t l[8]; for (int i = 0; i < 8; i++) l[i] = (uint8_t)(bits >> (56 - 8 * i));
    sha1_update(c, l, 8);
    for (int i = 0; i < 5; i++) { out[4 * i] = (uint8_t)(c->h[i] >> 24); out[4 * i + 1] = (uint8_t)(c->h[i] >> 16); out[4 * i + 2] = (uint8_t)(c->h[i] >> 8); out[4 * i + 3] = (uint8_t)c->h[i]; }
}

/* SHA-384/512. The round constants are derived at start-up (fractional parts of the cube roots of the first 80 primes, the
 * initial values from the square roots of the first 16) with exact integer arithmetic, so there is no table to mistype. */
typedef struct { uint64_t h[8]; uint64_t lo, hi; uint8_t buf[128]; size_t n; int out_len; } sha512_t;
static uint64_t K512[80], H512[8], H384[8]; static bool consts_ready;

typedef struct { uint32_t l[8]; } big256;                                       /* 256-bit magnitude, little endian */
static void b_mul(const big256 *a, const big256 *b, big256 *o, bool *ovf)
{
    uint64_t t[16] = { 0 };
    for (int i = 0; i < 8; i++) { uint64_t c = 0; for (int j = 0; j < 8; j++) { uint64_t v = t[i + j] + (uint64_t)a->l[i] * b->l[j] + c; t[i + j] = v & 0xffffffffu; c = v >> 32; } t[i + 8] += c; }
    *ovf = false; for (int i = 8; i < 16; i++) if (t[i]) *ovf = true;
    for (int i = 0; i < 8; i++) o->l[i] = (uint32_t)t[i];
}
static int b_cmp(const big256 *a, const big256 *b) { for (int i = 7; i >= 0; i--) if (a->l[i] != b->l[i]) return a->l[i] < b->l[i] ? -1 : 1; return 0; }
static void b_shl(big256 *a, unsigned bits)                                       /* a <<= bits (bits < 256) */
{
    big256 r; memset(&r, 0, sizeof r);
    unsigned w = bits / 32, s = bits % 32;
    for (int i = 7; i >= 0; i--) { uint64_t v = 0; int src = i - (int)w; if (src >= 0) v = (uint64_t)a->l[src] << s; if (s && src - 1 >= 0) v |= (uint64_t)a->l[src - 1] >> (32 - s); r.l[i] = (uint32_t)v; }
    *a = r;
}
static uint64_t iroot_frac(unsigned prime, int root)                              /* low 64 bits of floor(prime^(1/root) * 2^64) */
{
    big256 target; memset(&target, 0, sizeof target); target.l[0] = prime;
    b_shl(&target, root == 2 ? 128 : 192);
    uint64_t hi = 0, x_hi_bits = root == 2 ? 72 : 68; (void)hi;
    big256 x; memset(&x, 0, sizeof x);
    for (int bit = (int)x_hi_bits - 1; bit >= 0; bit--) {
        big256 trial = x; trial.l[bit / 32] |= 1u << (bit % 32);
        big256 p, q; bool o1, o2;
        b_mul(&trial, &trial, &p, &o1);
        if (root == 3) b_mul(&p, &trial, &q, &o2); else { q = p; o2 = false; }
        if (!o1 && !o2 && b_cmp(&q, &target) <= 0) x = trial;
    }
    return (uint64_t)x.l[0] | (uint64_t)x.l[1] << 32;
}
static void init_consts(void)
{
    if (consts_ready) return;
    unsigned primes[80]; int np = 0;
    for (unsigned c = 2; np < 80; c++) { bool ok = true; for (int i = 0; i < np; i++) if (c % primes[i] == 0) { ok = false; break; } if (ok) primes[np++] = c; }
    for (int i = 0; i < 80; i++) K512[i] = iroot_frac(primes[i], 3);
    for (int i = 0; i < 8; i++) { H512[i] = iroot_frac(primes[i], 2); H384[i] = iroot_frac(primes[i + 8], 2); }
    consts_ready = true;
}
static uint64_t ror64(uint64_t x, int r) { return (x >> r) | (x << (64 - r)); }
static void sha512_block(sha512_t *c, const uint8_t *p)
{
    uint64_t w[80], a = c->h[0], b = c->h[1], cc = c->h[2], d = c->h[3], e = c->h[4], f = c->h[5], g = c->h[6], h = c->h[7];
    for (int i = 0; i < 16; i++) { uint64_t v = 0; for (int k = 0; k < 8; k++) v = v << 8 | p[8 * i + k]; w[i] = v; }
    for (int i = 16; i < 80; i++) {
        uint64_t s0 = ror64(w[i - 15], 1) ^ ror64(w[i - 15], 8) ^ (w[i - 15] >> 7), s1 = ror64(w[i - 2], 19) ^ ror64(w[i - 2], 61) ^ (w[i - 2] >> 6);
        w[i] = w[i - 16] + s0 + w[i - 7] + s1;
    }
    for (int i = 0; i < 80; i++) {
        uint64_t S1 = ror64(e, 14) ^ ror64(e, 18) ^ ror64(e, 41), ch = (e & f) ^ (~e & g), t1 = h + S1 + ch + K512[i] + w[i];
        uint64_t S0 = ror64(a, 28) ^ ror64(a, 34) ^ ror64(a, 39), mj = (a & b) ^ (a & cc) ^ (b & cc), t2 = S0 + mj;
        h = g; g = f; f = e; e = d + t1; d = cc; cc = b; b = a; a = t1 + t2;
    }
    c->h[0] += a; c->h[1] += b; c->h[2] += cc; c->h[3] += d; c->h[4] += e; c->h[5] += f; c->h[6] += g; c->h[7] += h;
}
static void sha512_init(sha512_t *c, int out_len) { init_consts(); memcpy(c->h, out_len == 48 ? H384 : H512, sizeof c->h); c->lo = c->hi = 0; c->n = 0; c->out_len = out_len; }
static void sha512_update(sha512_t *c, const void *d, size_t len)
{
    const uint8_t *p = d;
    uint64_t add = (uint64_t)len << 3; c->lo += add; if (c->lo < add) c->hi++; c->hi += (uint64_t)len >> 61;
    while (len) { size_t k = 128 - c->n < len ? 128 - c->n : len; memcpy(c->buf + c->n, p, k); c->n += k; p += k; len -= k; if (c->n == 128) { sha512_block(c, c->buf); c->n = 0; } }
}
static void sha512_final(sha512_t *c, uint8_t out[64])
{
    uint64_t lo = c->lo, hi = c->hi; uint8_t pad = 0x80;
    sha512_update(c, &pad, 1); pad = 0;
    while (c->n != 112) sha512_update(c, &pad, 1);
    uint8_t l[16]; for (int i = 0; i < 8; i++) { l[i] = (uint8_t)(hi >> (56 - 8 * i)); l[8 + i] = (uint8_t)(lo >> (56 - 8 * i)); }
    sha512_update(c, l, 16);
    for (int i = 0; i < 8; i++) for (int k = 0; k < 8; k++) out[8 * i + k] = (uint8_t)(c->h[i] >> (56 - 8 * k));
}

enum { HA_SHA1 = 2, HA_SHA256 = 8, HA_SHA384 = 9, HA_SHA512 = 10 };
typedef struct { int algo; union { struct loos_sha256 s256; sha512_t s512; sha1_t s1; } u; } hctx;
static bool h_init(hctx *h, int algo)
{
    h->algo = algo;
    switch (algo) { case HA_SHA256: loos_sha256_init(&h->u.s256); return true; case HA_SHA384: sha512_init(&h->u.s512, 48); return true; case HA_SHA512: sha512_init(&h->u.s512, 64); return true; case HA_SHA1: sha1_init(&h->u.s1); return true; default: return false; }
}
static void h_update(hctx *h, const void *d, size_t n)
{
    switch (h->algo) { case HA_SHA256: loos_sha256_update(&h->u.s256, d, n); break; case HA_SHA384: case HA_SHA512: sha512_update(&h->u.s512, d, n); break; default: sha1_update(&h->u.s1, d, n); }
}
static size_t h_final(hctx *h, uint8_t *out)
{
    switch (h->algo) {
    case HA_SHA256: loos_sha256_final(&h->u.s256, out); return 32;
    case HA_SHA384: { uint8_t t[64]; sha512_final(&h->u.s512, t); memcpy(out, t, 48); return 48; }
    case HA_SHA512: sha512_final(&h->u.s512, out); return 64;
    default: sha1_final(&h->u.s1, out); return 20;
    }
}

/* ------------------------------------------------------------------------------------------ armor ---- */
static int b64v(int c) { if (c >= 'A' && c <= 'Z') return c - 'A'; if (c >= 'a' && c <= 'z') return c - 'a' + 26; if (c >= '0' && c <= '9') return c - '0' + 52; if (c == '+') return 62; if (c == '/') return 63; return -1; }
static uint32_t crc24(const uint8_t *d, size_t n)
{
    uint32_t crc = 0xb704ceu;
    while (n--) { crc ^= (uint32_t)(*d++) << 16; for (int i = 0; i < 8; i++) { crc <<= 1; if (crc & 0x1000000u) crc ^= 0x1864cfbu; } }
    return crc & 0xffffffu;
}
int pgp_dearmor(const char *in, size_t len, uint8_t *out, size_t cap, char *label, size_t label_cap)
{
    const char *p = in, *end = in + len;
    while (p < end && (*p == ' ' || *p == '\n' || *p == '\r' || *p == '\t')) p++;
    if (end - p < 15 || strncmp(p, "-----BEGIN PGP ", 15)) return -1;
    const char *le = memchr(p + 15, '-', (size_t)(end - p - 15));
    if (!le || strncmp(le, "-----", 5)) return -1;
    if (label && label_cap) { size_t l = (size_t)(le - (p + 15)); if (l >= label_cap) l = label_cap - 1; memcpy(label, p + 15, l); label[l] = 0; }
    p = le + 5;
    while (p < end && *p != '\n') p++;
    if (p < end) p++;
    for (;;) {                                                                    /* armor headers up to the blank line */
        const char *e = p; while (e < end && *e != '\n') e++;
        if (e == p || (e - p == 1 && *p == '\r')) { p = e < end ? e + 1 : e; break; }
        if (!memchr(p, ':', (size_t)(e - p))) break;                              /* no headers at all */
        p = e < end ? e + 1 : e;
    }
    size_t o = 0; uint32_t acc = 0; int bits = 0; bool crc_seen = false; uint32_t crc_val = 0;
    while (p < end) {
        if (*p == '=' ) {                                                         /* CRC-24 line */
            uint32_t v = 0; int k = 0; const char *q = p + 1;
            while (q < end && k < 4 && b64v((unsigned char)*q) >= 0) { v = v << 6 | (uint32_t)b64v((unsigned char)*q); q++; k++; }
            if (k == 4) { crc_seen = true; crc_val = v; p = q; continue; }
        }
        if (*p == '-') break;                                                     /* END line */
        int v = b64v((unsigned char)*p);
        if (v >= 0) { acc = acc << 6 | (uint32_t)v; bits += 6; if (bits >= 8) { bits -= 8; if (o >= cap) return -1; out[o++] = (uint8_t)(acc >> bits); acc &= (1u << bits) - 1; } }
        else if (*p != '=' && *p != '\n' && *p != '\r' && *p != ' ') return -1;
        p++;
    }
    if (crc_seen && crc24(out, o) != crc_val) return -1;
    return (int)o;
}

/* ------------------------------------------------------------------------------------------ RSA ------ */
#define MAXL 128                                                                  /* 4096 bits */
static int bn_len32(const uint8_t *b, size_t n, uint32_t *l)
{
    while (n && !*b) { b++; n--; }
    int L = (int)((n + 3) / 4);
    if (L > MAXL) return -1;
    memset(l, 0, (size_t)MAXL * 4);
    for (size_t i = 0; i < n; i++) l[(n - 1 - i) / 4] |= (uint32_t)b[i] << (8 * ((n - 1 - i) % 4));
    return L;
}
static uint32_t inv32(uint32_t n0) { uint32_t x = n0; for (int i = 0; i < 5; i++) x *= 2 - n0 * x; return x; }
static void mont_mul(const uint32_t *a, const uint32_t *b, const uint32_t *n, uint32_t ninv, int L, uint32_t *out)
{
    uint32_t t[MAXL + 2]; memset(t, 0, (size_t)(L + 2) * 4);
    for (int i = 0; i < L; i++) {
        uint64_t c = 0;
        for (int j = 0; j < L; j++) { uint64_t v = (uint64_t)t[j] + (uint64_t)a[j] * b[i] + c; t[j] = (uint32_t)v; c = v >> 32; }
        uint64_t v = (uint64_t)t[L] + c; t[L] = (uint32_t)v; t[L + 1] = (uint32_t)(v >> 32);
        uint32_t m = t[0] * ninv; c = 0;
        { uint64_t w = (uint64_t)t[0] + (uint64_t)m * n[0]; c = w >> 32; }
        for (int j = 1; j < L; j++) { uint64_t w = (uint64_t)t[j] + (uint64_t)m * n[j] + c; t[j - 1] = (uint32_t)w; c = w >> 32; }
        uint64_t w = (uint64_t)t[L] + c; t[L - 1] = (uint32_t)w; t[L] = t[L + 1] + (uint32_t)(w >> 32);
    }
    bool ge = t[L] != 0;
    if (!ge) { ge = true; for (int j = L - 1; j >= 0; j--) if (t[j] != n[j]) { ge = t[j] > n[j]; break; } }
    if (ge) { int64_t br = 0; for (int j = 0; j < L; j++) { int64_t d = (int64_t)t[j] - n[j] - br; br = d < 0; t[j] = (uint32_t)d; } }
    memcpy(out, t, (size_t)L * 4);
}
bool pgp_rsa_public(const uint8_t *nb, size_t nlen, const uint8_t *eb, size_t elen, const uint8_t *in, size_t inlen, uint8_t *out, size_t outlen)
{
    static uint32_t n[MAXL], s[MAXL], r2[MAXL], sm[MAXL], acc[MAXL], one[MAXL];
    int L = bn_len32(nb, nlen, n); if (L <= 0 || !(n[0] & 1)) return false;
    if (bn_len32(in, inlen, s) < 0) return false;
    for (int j = L - 1; j >= 0; j--) { if (s[j] != n[j]) { if (s[j] > n[j]) return false; break; } if (j == 0) return false; }   /* s must be < n */
    uint32_t ninv = (uint32_t)(0u - inv32(n[0]));
    /* r2 = 2^(64L) mod n by repeated doubling */
    memset(r2, 0, sizeof r2); r2[0] = 1;
    for (int i = 0; i < 64 * L; i++) {
        uint32_t carry = 0;
        for (int j = 0; j < L; j++) { uint32_t nc = r2[j] >> 31; r2[j] = r2[j] << 1 | carry; carry = nc; }
        bool ge = carry != 0;
        if (!ge) { ge = true; for (int j = L - 1; j >= 0; j--) if (r2[j] != n[j]) { ge = r2[j] > n[j]; break; } }
        if (ge) { int64_t br = 0; for (int j = 0; j < L; j++) { int64_t d = (int64_t)r2[j] - n[j] - br; br = d < 0; r2[j] = (uint32_t)d; } }
    }
    memset(one, 0, sizeof one); one[0] = 1;
    mont_mul(s, r2, n, ninv, L, sm);
    mont_mul(one, r2, n, ninv, L, acc);
    size_t e0 = 0; while (e0 < elen && !eb[e0]) e0++;
    if (elen - e0 > 4 || e0 == elen) return false;
    for (size_t i = e0; i < elen; i++) for (int bit = 7; bit >= 0; bit--) {
        mont_mul(acc, acc, n, ninv, L, acc);
        if ((eb[i] >> bit) & 1) mont_mul(acc, sm, n, ninv, L, acc);
    }
    mont_mul(acc, one, n, ninv, L, acc);
    for (size_t i = 0; i < outlen; i++) { size_t pos = outlen - 1 - i; out[pos] = (size_t)(i / 4) < (size_t)MAXL ? (uint8_t)(acc[i / 4] >> (8 * (i % 4))) : 0; }
    for (size_t i = outlen; i < (size_t)MAXL * 4; i++) if ((acc[i / 4] >> (8 * (i % 4))) & 0xff) return false;     /* result larger than the modulus size */
    return true;
}

/* ------------------------------------------------------------------------------------------ packets --- */
typedef struct { int tag; const uint8_t *body; size_t len; } packet;
static int next_packet(const uint8_t *d, size_t n, size_t *pos, packet *p)
{
    if (*pos >= n) return 0;
    uint8_t h = d[*pos];
    if (!(h & 0x80)) return -1;
    size_t i = *pos + 1, len;
    if (h & 0x40) {
        p->tag = h & 0x3f;
        if (i >= n) return -1;
        uint8_t l0 = d[i++];
        if (l0 < 192) len = l0;
        else if (l0 < 224) { if (i >= n) return -1; len = (size_t)(l0 - 192) * 256 + d[i++] + 192; }
        else if (l0 == 255) { if (i + 4 > n) return -1; len = (size_t)d[i] << 24 | (size_t)d[i + 1] << 16 | (size_t)d[i + 2] << 8 | d[i + 3]; i += 4; }
        else return -1;                                                           /* partial lengths are not used for keys/signatures */
    } else {
        p->tag = (h >> 2) & 15;
        int lt = h & 3;
        if (lt == 0) { if (i >= n) return -1; len = d[i++]; }
        else if (lt == 1) { if (i + 2 > n) return -1; len = (size_t)d[i] << 8 | d[i + 1]; i += 2; }
        else if (lt == 2) { if (i + 4 > n) return -1; len = (size_t)d[i] << 24 | (size_t)d[i + 1] << 16 | (size_t)d[i + 2] << 8 | d[i + 3]; i += 4; }
        else return -1;
    }
    if (len > n - i) return -1;
    p->body = d + i; p->len = len; *pos = i + len;
    return 1;
}
static bool read_mpi(const uint8_t *b, size_t n, size_t *pos, const uint8_t **out, size_t *olen, int *bits)
{
    if (*pos + 2 > n) return false;
    int bt = b[*pos] << 8 | b[*pos + 1]; size_t l = (size_t)(bt + 7) / 8;
    if (*pos + 2 + l > n) return false;
    *out = b + *pos + 2; *olen = l; if (bits) *bits = bt; *pos += 2 + l;
    return true;
}

typedef struct { uint32_t created, sig_exp, key_exp; bool have_created, have_sig_exp, have_key_exp; uint8_t issuer[8]; bool have_issuer; uint8_t issuer_fpr[20]; bool have_fpr; uint8_t flags; bool have_flags; } subpk;
static bool parse_subpackets(const uint8_t *d, size_t n, subpk *s)
{
    size_t i = 0;
    while (i < n) {
        size_t len = d[i++];
        if (len >= 192 && len < 255) { if (i >= n) return false; len = (len - 192) * 256 + d[i++] + 192; }
        else if (len == 255) { if (i + 4 > n) return false; len = (size_t)d[i] << 24 | (size_t)d[i + 1] << 16 | (size_t)d[i + 2] << 8 | d[i + 3]; i += 4; }
        if (len == 0 || len > n - i) return false;
        uint8_t type = d[i] & 0x7f; const uint8_t *v = d + i + 1; size_t vl = len - 1;
        switch (type) {
        case 2: if (vl == 4) { s->created = (uint32_t)v[0] << 24 | (uint32_t)v[1] << 16 | (uint32_t)v[2] << 8 | v[3]; s->have_created = true; } break;
        case 3: if (vl == 4) { s->sig_exp = (uint32_t)v[0] << 24 | (uint32_t)v[1] << 16 | (uint32_t)v[2] << 8 | v[3]; s->have_sig_exp = true; } break;
        case 9: if (vl == 4) { s->key_exp = (uint32_t)v[0] << 24 | (uint32_t)v[1] << 16 | (uint32_t)v[2] << 8 | v[3]; s->have_key_exp = true; } break;
        case 16: if (vl == 8) { memcpy(s->issuer, v, 8); s->have_issuer = true; } break;
        case 27: if (vl >= 1) { s->flags = v[0]; s->have_flags = true; } break;
        case 33: if (vl == 21 && v[0] == 4) { memcpy(s->issuer_fpr, v + 1, 20); s->have_fpr = true; } break;
        default: break;
        }
        i += len;
    }
    return true;
}

void pgp_keyring_init(pgp_keyring *r) { memset(r, 0, sizeof *r); r->min_bits = 2048; }
void pgp_keyring_free(pgp_keyring *r) { for (int i = 0; i < r->n; i++) { free(r->k[i].n); free(r->k[i].e); } free(r->k); memset(r, 0, sizeof *r); }
void pgp_fingerprint_hex(const uint8_t f[20], char out[41]) { static const char d[] = "0123456789ABCDEF"; for (int i = 0; i < 20; i++) { out[2 * i] = d[f[i] >> 4]; out[2 * i + 1] = d[f[i] & 15]; } out[40] = 0; }
const pgp_key *pgp_keyring_find(const pgp_keyring *r, const uint8_t id[8]) { for (int i = 0; i < r->n; i++) if (!memcmp(r->k[i].keyid, id, 8)) return &r->k[i]; return NULL; }

static bool add_key_packet(pgp_keyring *r, const packet *p, bool sub, int primary)
{
    if (p->len < 6 || p->body[0] != 4) return false;                              /* only v4 keys */
    pgp_key k; memset(&k, 0, sizeof k);
    k.created = (uint32_t)p->body[1] << 24 | (uint32_t)p->body[2] << 16 | (uint32_t)p->body[3] << 8 | p->body[4];
    k.algo = p->body[5]; k.is_subkey = sub; k.primary = primary; k.can_sign = !sub;
    size_t pos = 6;
    if (k.algo != 1 && k.algo != 3) { k.n = k.e = NULL; }
    else {
        const uint8_t *n, *e; size_t nl, el; int nb;
        if (!read_mpi(p->body, p->len, &pos, &n, &nl, &nb) || !read_mpi(p->body, p->len, &pos, &e, &el, NULL)) return false;
        if (nb < r->min_bits) k.algo = 0xff;                                      /* too short: kept so the failure is reported as a weak key */
        k.n = malloc(nl); k.e = malloc(el); if (!k.n || !k.e) abort();
        memcpy(k.n, n, nl); memcpy(k.e, e, el); k.nlen = nl; k.elen = el;
    }
    sha1_t h; sha1_init(&h);
    uint8_t pre[3] = { 0x99, (uint8_t)(p->len >> 8), (uint8_t)p->len };
    sha1_update(&h, pre, 3); sha1_update(&h, p->body, p->len); sha1_final(&h, k.fpr);
    memcpy(k.keyid, k.fpr + 12, 8);
    if (r->n == r->cap) { r->cap = r->cap ? r->cap * 2 : 8; r->k = realloc(r->k, (size_t)r->cap * sizeof *r->k); if (!r->k) abort(); }
    r->k[r->n++] = k;
    return true;
}

int pgp_keyring_add(pgp_keyring *r, const uint8_t *data, size_t len)
{
    uint8_t *bin = NULL; const uint8_t *d = data; size_t n = len;
    const char *c = (const char *)data;
    size_t w = 0; while (w < len && (c[w] == ' ' || c[w] == '\n' || c[w] == '\r' || c[w] == '\t')) w++;
    if (len - w > 5 && !memcmp(c + w, "-----", 5)) {
        bin = malloc(len); if (!bin) abort();
        int dl = pgp_dearmor(c, len, bin, len, NULL, 0);
        if (dl < 0) { free(bin); return -1; }
        d = bin; n = (size_t)dl;
    }
    int added = 0, primary = -1; size_t pos = 0; packet p; int rc;
    while ((rc = next_packet(d, n, &pos, &p)) > 0) {
        if (p.tag == 6) { if (add_key_packet(r, &p, false, -1)) { primary = r->n - 1; added++; } else primary = -1; }
        else if (p.tag == 14) { if (primary >= 0 && add_key_packet(r, &p, true, primary)) added++; }
        else if (p.tag == 2 && r->n && p.len > 10 && p.body[0] == 4) {                         /* self-signatures carry flags, expiry and revocations */
            int type = p.body[1];
            size_t hl = (size_t)p.body[4] << 8 | p.body[5];
            if (6 + hl > p.len) continue;
            subpk sp; memset(&sp, 0, sizeof sp);
            if (!parse_subpackets(p.body + 6, hl, &sp)) continue;
            pgp_key *last = &r->k[r->n - 1];
            if (type == 0x20 && primary >= 0 && primary < r->n) r->k[primary].revoked = true;           /* key revocation */
            else if (type == 0x28 && last->is_subkey) last->revoked = true;                                /* subkey revocation */
            else if (type == 0x18 && last->is_subkey) {                                                     /* subkey binding */
                last->can_sign = sp.have_flags ? (sp.flags & 0x02) != 0 : false;
                if (sp.have_key_exp) last->expires_after = sp.key_exp;
            } else if ((type >= 0x10 && type <= 0x13) && primary >= 0 && !last->is_subkey) {              /* user id self-certification */
                if (sp.have_flags) last->can_sign = (sp.flags & 0x02) != 0 || last->can_sign;
                if (sp.have_key_exp) last->expires_after = sp.key_exp;
            }
        }
    }
    free(bin);
    return rc < 0 ? -1 : added;
}

const char *pgp_result_text(pgp_result r)
{
    switch (r) {
    case PGP_OK: return "good signature"; case PGP_BAD_SIGNATURE: return "BAD signature"; case PGP_NO_KEY: return "signed with a key that is not in the trusted keyring";
    case PGP_KEY_EXPIRED: return "signing key has expired"; case PGP_KEY_REVOKED: return "signing key has been revoked"; case PGP_SIG_EXPIRED: return "signature has expired";
    case PGP_SIG_FROM_FUTURE: return "signature was made in the future"; case PGP_UNSUPPORTED: return "unsupported signature or key algorithm";
    case PGP_WEAK_HASH: return "signature uses a weak hash (SHA-1)"; case PGP_WEAK_KEY: return "signing key is too weak"; case PGP_MALFORMED: return "malformed signature data";
    case PGP_NOT_A_SIGNER: return "the key is not allowed to sign"; default: return "error";
    }
}

/* ------------------------------------------------------------------------------------------ verify ---- */
typedef struct { const uint8_t *body; size_t len; int type, halgo, palgo; const uint8_t *hashed; size_t hashed_len; subpk hs, us; const uint8_t *mpi; size_t mpi_len; uint8_t left[2]; } sigpkt;

static pgp_result parse_sig(const uint8_t *sig, size_t len, sigpkt *s)
{
    size_t pos = 0; packet p; int rc;
    while ((rc = next_packet(sig, len, &pos, &p)) > 0) if (p.tag == 2) break;
    if (rc <= 0) return PGP_MALFORMED;
    if (p.len < 11) return PGP_MALFORMED;
    if (p.body[0] != 4) return PGP_UNSUPPORTED;                                    /* v3/v5/v6 signatures are not handled */
    memset(s, 0, sizeof *s);
    s->body = p.body; s->len = p.len; s->type = p.body[1]; s->palgo = p.body[2]; s->halgo = p.body[3];
    size_t hl = (size_t)p.body[4] << 8 | p.body[5];
    if (6 + hl + 2 > p.len) return PGP_MALFORMED;
    s->hashed = p.body + 6; s->hashed_len = hl;
    if (!parse_subpackets(s->hashed, hl, &s->hs)) return PGP_MALFORMED;
    size_t ul = (size_t)p.body[6 + hl] << 8 | p.body[7 + hl];
    if (8 + hl + ul + 2 > p.len) return PGP_MALFORMED;
    if (!parse_subpackets(p.body + 8 + hl, ul, &s->us)) return PGP_MALFORMED;
    memcpy(s->left, p.body + 8 + hl + ul, 2);
    size_t mp = 8 + hl + ul + 2; const uint8_t *m; size_t ml;
    if (!read_mpi(p.body, p.len, &mp, &m, &ml, NULL)) return PGP_MALFORMED;
    s->mpi = m; s->mpi_len = ml;
    return PGP_OK;
}

static const uint8_t DI_SHA256[] = { 0x30, 0x31, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x01, 0x05, 0x00, 0x04, 0x20 };
static const uint8_t DI_SHA384[] = { 0x30, 0x41, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x02, 0x05, 0x00, 0x04, 0x30 };
static const uint8_t DI_SHA512[] = { 0x30, 0x51, 0x30, 0x0d, 0x06, 0x09, 0x60, 0x86, 0x48, 0x01, 0x65, 0x03, 0x04, 0x02, 0x03, 0x05, 0x00, 0x04, 0x40 };

static pgp_result check_with_key(const sigpkt *s, const uint8_t *digest, size_t dl, const pgp_key *k, time_t now, pgp_verdict *v)
{
    if (k->algo == 0xff) return PGP_WEAK_KEY;
    if (k->algo != 1 || !k->n) return PGP_UNSUPPORTED;
    const pgp_key *pk = k->is_subkey ? NULL : k; (void)pk;
    if (!k->can_sign) return PGP_NOT_A_SIGNER;
    if (k->revoked) return PGP_KEY_REVOKED;
    uint32_t sc = s->hs.have_created ? s->hs.created : 0;
    if (!s->hs.have_created) return PGP_MALFORMED;
    if ((time_t)sc > now + 600) return PGP_SIG_FROM_FUTURE;
    if (s->hs.have_sig_exp && s->hs.sig_exp && (time_t)sc + (time_t)s->hs.sig_exp < now) return PGP_SIG_EXPIRED;
    if (sc < k->created) return PGP_BAD_SIGNATURE;                                  /* signed before the key existed */
    if (k->expires_after && (time_t)k->created + (time_t)k->expires_after < (time_t)sc) return PGP_KEY_EXPIRED;
    if (k->expires_after && (time_t)k->created + (time_t)k->expires_after < now) return PGP_KEY_EXPIRED;
    size_t klen = k->nlen; while (klen && !k->n[klen - 1 - (k->nlen - klen)] ) break;
    size_t nb = 0; { size_t i = 0; while (i < k->nlen && !k->n[i]) i++; nb = k->nlen - i; }
    uint8_t em[MAXL * 4], rec[MAXL * 4];
    if (nb < 128 || nb > MAXL * 4 || s->mpi_len > nb) return PGP_BAD_SIGNATURE;
    (void)klen;
    const uint8_t *di; size_t dil;
    switch (s->halgo) { case HA_SHA256: di = DI_SHA256; dil = sizeof DI_SHA256; break; case HA_SHA384: di = DI_SHA384; dil = sizeof DI_SHA384; break; case HA_SHA512: di = DI_SHA512; dil = sizeof DI_SHA512; break; default: return PGP_UNSUPPORTED; }
    /* expected EM = 00 01 FF.. 00 DigestInfo || digest */
    if (nb < dil + dl + 11) return PGP_BAD_SIGNATURE;
    memset(em, 0xff, nb); em[0] = 0; em[1] = 1; em[nb - dl - dil - 1] = 0;
    memcpy(em + nb - dl - dil, di, dil); memcpy(em + nb - dl, digest, dl);
    uint8_t padded[MAXL * 4]; memset(padded, 0, nb); memcpy(padded + nb - s->mpi_len, s->mpi, s->mpi_len);
    if (!pgp_rsa_public(k->n, k->nlen, k->e, k->elen, padded, nb, rec, nb)) return PGP_BAD_SIGNATURE;
    if (memcmp(rec, em, nb) != 0) return PGP_BAD_SIGNATURE;
    if (v) { memcpy(v->keyid, k->keyid, 8); memcpy(v->fpr, k->fpr, 20); v->have_fpr = true; v->sig_created = sc; v->hash_algo = (uint8_t)s->halgo; }
    return PGP_OK;
}

/* hash the signed data plus the v4 signature trailer, then try every signing key that matches the issuer */
static pgp_result verify_core(const pgp_keyring *ring, const sigpkt *s, const uint8_t *data, size_t dlen, const uint8_t *data2, size_t d2len, time_t now, pgp_verdict *v)
{
    if (s->type != 0x00 && s->type != 0x01) return PGP_UNSUPPORTED;
    if (s->palgo != 1 && s->palgo != 3) return PGP_UNSUPPORTED;
    if (s->halgo == HA_SHA1) return PGP_WEAK_HASH;
    hctx h; if (!h_init(&h, s->halgo)) return PGP_UNSUPPORTED;
    h_update(&h, data, dlen);
    if (d2len) h_update(&h, data2, d2len);
    uint8_t tr[6] = { 4, (uint8_t)s->type, (uint8_t)s->palgo, (uint8_t)s->halgo, (uint8_t)(s->hashed_len >> 8), (uint8_t)s->hashed_len };
    h_update(&h, tr, 6); h_update(&h, s->hashed, s->hashed_len);
    uint32_t total = (uint32_t)(6 + s->hashed_len); uint8_t fin[6] = { 4, 0xff, (uint8_t)(total >> 24), (uint8_t)(total >> 16), (uint8_t)(total >> 8), (uint8_t)total };
    h_update(&h, fin, 6);
    uint8_t digest[64]; size_t dl = h_final(&h, digest);
    if (digest[0] != s->left[0] || digest[1] != s->left[1]) return PGP_BAD_SIGNATURE;
    uint8_t id[8]; bool have_id = false;
    if (s->hs.have_fpr) { memcpy(id, s->hs.issuer_fpr + 12, 8); have_id = true; }
    else if (s->hs.have_issuer) { memcpy(id, s->hs.issuer, 8); have_id = true; }
    else if (s->us.have_issuer) { memcpy(id, s->us.issuer, 8); have_id = true; }
    if (!have_id) return PGP_MALFORMED;
    pgp_result best = PGP_NO_KEY;
    for (int i = 0; i < ring->n; i++) {
        if (memcmp(ring->k[i].keyid, id, 8)) continue;
        const pgp_key *k = &ring->k[i];
        pgp_key eff = *k;                                                          /* a subkey inherits expiry/revocation from its primary */
        if (k->is_subkey && k->primary >= 0 && k->primary < ring->n) {
            const pgp_key *pm = &ring->k[k->primary];
            if (pm->revoked) eff.revoked = true;
            if (pm->expires_after && (!eff.expires_after || (time_t)pm->created + pm->expires_after < (time_t)eff.created + eff.expires_after)) { eff.created = pm->created; eff.expires_after = pm->expires_after; }
        }
        pgp_result r = check_with_key(s, digest, dl, &eff, now, v);
        if (r == PGP_OK) return PGP_OK;
        if (best == PGP_NO_KEY || r == PGP_KEY_REVOKED || r == PGP_KEY_EXPIRED) best = r;
        if (r == PGP_BAD_SIGNATURE && best == PGP_NO_KEY) best = r;
    }
    return best;
}

pgp_result pgp_verify_detached(const pgp_keyring *ring, const uint8_t *sig, size_t sig_len, const uint8_t *data, size_t data_len, time_t now, pgp_verdict *v)
{
    pgp_verdict local; if (!v) v = &local;
    memset(v, 0, sizeof *v);
    uint8_t *bin = NULL; const uint8_t *sg = sig; size_t sl = sig_len;
    const char *c = (const char *)sig; size_t w = 0; while (w < sig_len && (c[w] == ' ' || c[w] == '\n' || c[w] == '\r')) w++;
    if (sig_len - w > 5 && !memcmp(c + w, "-----", 5)) {
        bin = malloc(sig_len); if (!bin) abort();
        int dl = pgp_dearmor(c, sig_len, bin, sig_len, NULL, 0);
        if (dl < 0) { free(bin); v->result = PGP_MALFORMED; return v->result; }
        sg = bin; sl = (size_t)dl;
    }
    sigpkt s; pgp_result r = parse_sig(sg, sl, &s);
    if (r == PGP_OK) {
        if (s.type == 0x01) r = PGP_UNSUPPORTED;                                    /* text-mode detached signatures are not used for Release files */
        else r = verify_core(ring, &s, data, data_len, NULL, 0, now, v);
    }
    free(bin);
    v->result = r;
    return r;
}

pgp_result pgp_verify_clearsigned(const pgp_keyring *ring, const char *text, size_t len, time_t now, pgp_verdict *v, char *content, size_t cap, size_t *clen)
{
    pgp_verdict local; if (!v) v = &local;
    memset(v, 0, sizeof *v);
    #define FAIL(x) do { v->result = (x); return (x); } while (0)
    static const char HDR[] = "-----BEGIN PGP SIGNED MESSAGE-----", SIGH[] = "-----BEGIN PGP SIGNATURE-----";
    size_t p = 0;
    while (p < len && (text[p] == '\n' || text[p] == '\r' || text[p] == ' ')) p++;
    if (len - p < sizeof HDR - 1 || memcmp(text + p, HDR, sizeof HDR - 1)) FAIL(PGP_MALFORMED);      /* nothing unsigned may precede the signed text */
    p += sizeof HDR - 1;
    while (p < len && text[p] != '\n') p++;
    p++;
    for (;;) {                                                                        /* armor headers (Hash: ...) until the blank line */
        size_t e = p; while (e < len && text[e] != '\n') e++;
        size_t le = e; if (le > p && text[le - 1] == '\r') le--;
        if (p >= len) FAIL(PGP_MALFORMED);
        if (le == p) { p = e + 1; break; }
        if (strncmp(text + p, "Hash:", 5)) FAIL(PGP_MALFORMED);
        p = e + 1;
    }
    size_t body_start = p, sig_at = len;
    for (size_t q = body_start; q + sizeof SIGH - 1 <= len; q = (size_t)((const char *)memchr(text + q, '\n', len - q) ? (const char *)memchr(text + q, '\n', len - q) - text + 1 : len)) {
        if (!memcmp(text + q, SIGH, sizeof SIGH - 1) && (q == body_start || text[q - 1] == '\n')) { sig_at = q; break; }
        if (q >= len) break;
    }
    if (sig_at == len) FAIL(PGP_MALFORMED);
    /* signed text = lines between, dash-unescaped, final line ending excluded */
    size_t end = sig_at; if (end > body_start && text[end - 1] == '\n') end--; if (end > body_start && text[end - 1] == '\r') end--;
    size_t cap_need = (end - body_start) + 2;
    uint8_t *canon = malloc(cap_need * 2 + 4); char *plain = malloc(cap_need + 1); if (!canon || !plain) abort();
    size_t cn = 0, pn = 0, q = body_start;
    while (q <= end) {
        size_t e = q; while (e < end && text[e] != '\n') e++;
        size_t le = e; if (le > q && text[le - 1] == '\r') le--;
        size_t ls = q;
        if (le > ls + 1 && text[ls] == '-' && text[ls + 1] == ' ') ls += 2;           /* dash escaping */
        else if (le > ls && text[ls] == '-') { free(canon); free(plain); FAIL(PGP_MALFORMED); }   /* an unescaped dash line could smuggle a second armor header */
        size_t te = le; while (te > ls && (text[te - 1] == ' ' || text[te - 1] == '\t')) te--;
        memcpy(canon + cn, text + ls, te - ls); cn += te - ls;
        memcpy(plain + pn, text + ls, le - ls); pn += le - ls;
        if (e < end) { canon[cn++] = '\r'; canon[cn++] = '\n'; plain[pn++] = '\n'; }
        if (e >= end) break;
        q = e + 1;
    }
    plain[pn++] = '\n';
    uint8_t *sigbin = malloc(len - sig_at + 1); if (!sigbin) abort();
    int sl = pgp_dearmor(text + sig_at, len - sig_at, sigbin, len - sig_at, NULL, 0);
    pgp_result r = PGP_MALFORMED;
    if (sl > 0) {
        sigpkt s;
        r = parse_sig(sigbin, (size_t)sl, &s);
        if (r == PGP_OK) r = s.type == 0x01 ? verify_core(ring, &s, canon, cn, NULL, 0, now, v) : PGP_UNSUPPORTED;
    }
    free(sigbin); free(canon);
    if (r == PGP_OK) {
        if (pn > cap) r = PGP_MALFORMED; else { memcpy(content, plain, pn); if (clen) *clen = pn; }
    }
    free(plain);
    v->result = r;
    return r;
    #undef FAIL
}
