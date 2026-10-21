/* Correctly rounded decimal/hex -> binary floating point conversion (round-half-even, any length input).
 * Decimal input is converted exactly with big integers: D * 10^e is a big multiplication for e >= 0 and a
 * big long division by 5^k (then a power-of-two scale) for e < 0; the 64-bit quotient plus a sticky bit goes to
 * one rounding step shared by double and float. Inputs of <=15 digits with |e|<=22 take Clinger's exact fast path. */
#include "libc.h"
#include <stdlib.h>
#include <string.h>
#include <ctype.h>
#include <errno.h>
#include <math.h>

#define BN 200
typedef struct { uint32_t l[BN]; int n; } bn;

static void bn_trim(bn *a) { while (a->n > 0 && a->l[a->n - 1] == 0) a->n--; }
static void bn_set(bn *a, uint64_t v) { a->n = 0; while (v) { a->l[a->n++] = (uint32_t)v; v >>= 32; } }
static void bn_mul_small(bn *a, uint32_t m)
{
    uint64_t c = 0;
    for (int i = 0; i < a->n; i++) { uint64_t v = (uint64_t)a->l[i] * m + c; a->l[i] = (uint32_t)v; c = v >> 32; }
    if (c && a->n < BN) a->l[a->n++] = (uint32_t)c;
}
static void bn_add_small(bn *a, uint32_t v)
{
    uint64_t c = v;
    for (int i = 0; c && i < a->n; i++) { c += a->l[i]; a->l[i] = (uint32_t)c; c >>= 32; }
    if (c && a->n < BN) a->l[a->n++] = (uint32_t)c;
}
static int bn_bits(const bn *a)
{
    if (!a->n) return 0;
    return (a->n - 1) * 32 + 32 - __builtin_clz(a->l[a->n - 1]);
}
static void bn_shl(bn *a, int s)
{
    int w = s / 32, b = s % 32;
    if (!a->n) return;
    int top = a->n + w + 1 > BN ? BN : a->n + w + 1;
    for (int i = top - 1; i >= 0; i--) {
        uint64_t v = 0;
        int src = i - w;
        if (src >= 0 && src < a->n) v = (uint64_t)a->l[src] << b;
        if (b && src - 1 >= 0 && src - 1 < a->n) v |= (uint64_t)a->l[src - 1] >> (32 - b);
        a->l[i] = (uint32_t)v;
    }
    a->n = top;
    bn_trim(a);
}
static int bn_cmp(const bn *a, const bn *b)
{
    if (a->n != b->n) return a->n < b->n ? -1 : 1;
    for (int i = a->n - 1; i >= 0; i--) if (a->l[i] != b->l[i]) return a->l[i] < b->l[i] ? -1 : 1;
    return 0;
}
static void bn_sub(bn *a, const bn *b)               /* a -= b, a >= b */
{
    int64_t br = 0;
    for (int i = 0; i < a->n; i++) {
        int64_t v = (int64_t)a->l[i] - (i < b->n ? b->l[i] : 0) - br;
        br = v < 0; if (v < 0) v += (int64_t)1 << 32;
        a->l[i] = (uint32_t)v;
    }
    bn_trim(a);
}
static int bn_bit(const bn *a, int i) { return i / 32 < a->n ? (int)((a->l[i / 32] >> (i % 32)) & 1) : 0; }

/* big = D * 10^e (e>=0) */
static void bn_mul_pow10(bn *a, int e)
{
    while (e >= 9) { bn_mul_small(a, 1000000000u); e -= 9; }
    static const uint32_t p[] = { 1, 10, 100, 1000, 10000, 100000, 1000000, 10000000, 100000000 };
    if (e) bn_mul_small(a, p[e]);
}

/* Round mant (MSB at bit 63, value = mant * 2^e2, plus sticky) to `p` significand bits, returning the
 * result as a double (exactly representable even when p == 24). */
static double assemble(uint64_t mant, int sticky, long e2, int mbits, int *range)
{
    const int emin = mbits == 53 ? -1022 : -126, emax = mbits == 53 ? 1023 : 127, bias = emax;
    long E = e2 + 63;
    int p = mbits;
    if (E > emax) { *range = 1; return HUGE_VAL; }
    if (E < emin) p = mbits - (int)(emin - E);
    unsigned __int128 r = 0;
    if (p >= 0) {
        int shift = 64 - p;
        unsigned __int128 t = mant;
        if (shift >= 64 + 1) r = 0;
        else {
            r = t >> shift;
            unsigned __int128 rem = t & ((((unsigned __int128)1) << shift) - 1), half = ((unsigned __int128)1) << (shift - 1);
            if (rem > half || (rem == half && (sticky || (r & 1)))) r++;
            else if (rem == half && !sticky && !(r & 1)) { }
        }
    }
    uint64_t rr = (uint64_t)r;
    uint64_t bits;
    if (rr == 0) { *range = 1; return 0.0; }
    if (E < emin) { bits = rr; if (rr < (1ULL << (mbits - 1))) *range = 1; }       /* subnormal (rr == 2^(m-1) is min normal) */
    else {
        bits = ((uint64_t)(E + bias) << (mbits - 1)) + (rr - (1ULL << (mbits - 1)));
        if ((bits >> (mbits - 1)) >= (uint64_t)(2 * bias + 1)) { *range = 1; return HUGE_VAL; }
    }
    if (mbits == 53) { double d; memcpy(&d, &bits, 8); return d; }
    { uint32_t b32 = (uint32_t)bits; float f; memcpy(&f, &b32, 4); return (double)f; }
}

/* Take the top 64 bits of a big integer; sticky collects the rest. Returns binary exponent adjustment. */
static long bn_top64(const bn *a, uint64_t *mant, int *sticky)
{
    int bits = bn_bits(a);
    uint64_t m = 0;
    *sticky = 0;
    if (bits <= 64) {
        for (int i = a->n - 1; i >= 0; i--) m = (m << 32) | a->l[i];
        m <<= 64 - bits;
        *mant = m;
        return -(64 - bits);
    }
    int sh = bits - 64;
    for (int i = 0; i < 64; i++) m |= (uint64_t)bn_bit(a, sh + i) << i;
    for (int i = 0; i < sh && !*sticky; i++) if (bn_bit(a, i)) *sticky = 1;
    *mant = m;
    return sh;
}

static double parse_hex(const char *p, const char **endp, int neg, int mbits, int *range)
{
    uint64_t m = 0; int sticky = 0, any = 0, seen_dot = 0; long e2 = 0;
    for (;; p++) {
        int c = (unsigned char)*p, d;
        if (c == '.' && !seen_dot) { seen_dot = 1; continue; }
        if (!isxdigit(c)) break;
        d = isdigit(c) ? c - '0' : tolower(c) - 'a' + 10;
        any = 1;
        if (m >> 60) { if (d) sticky = 1; if (!seen_dot) e2 += 4; }
        else { m = (m << 4) | (unsigned)d; if (seen_dot) e2 -= 4; }
    }
    if (!any) { *endp = 0; return 0; }
    if (*p == 'p' || *p == 'P') {
        const char *q = p + 1; int es = 0; long ev = 0;
        if (*q == '+' || *q == '-') es = *q++ == '-';
        if (isdigit((unsigned char)*q)) { while (isdigit((unsigned char)*q)) { if (ev < 100000) ev = ev * 10 + (*q - '0'); q++; } e2 += es ? -ev : ev; p = q; }
    }
    *endp = p;
    double r;
    if (!m) r = 0.0;
    else {
        int lz = __builtin_clzll(m);
        m <<= lz; e2 -= lz;
        r = assemble(m, sticky, e2, mbits, range);
    }
    return neg ? -r : r;
}

double __strtod_core(const char *s, char **end, int mbits, int *range_err)
{
    const char *p = s;
    int neg = 0, nd = 0, dropped_nz = 0, seen_dot = 0, any = 0;
    long e10 = 0, dropped_int = 0;
    enum { MAXD = 780 };
    static char dig[MAXD + 2];
    *range_err = 0;
    while (isspace((unsigned char)*p)) p++;
    if (*p == '+' || *p == '-') neg = *p++ == '-';
    if (!strncasecmp(p, "inf", 3)) {
        p += 3;
        if (!strncasecmp(p, "inity", 5)) p += 5;
        if (end) *end = (char *)p;
        return neg ? -HUGE_VAL : HUGE_VAL;
    }
    if (!strncasecmp(p, "nan", 3)) {
        p += 3;
        if (*p == '(') { const char *q = p + 1; while (isalnum((unsigned char)*q) || *q == '_') q++; if (*q == ')') p = q + 1; }
        if (end) *end = (char *)p;
        return neg ? -__builtin_nan("") : __builtin_nan("");
    }
    if (p[0] == '0' && (p[1] == 'x' || p[1] == 'X')) {
        const char *e; double r = parse_hex(p + 2, &e, neg, mbits, range_err);
        if (e) { if (end) *end = (char *)e; return r; }
        if (end) *end = (char *)(p + 1);                 /* just the "0" */
        return neg ? -0.0 : 0.0;
    }
    for (;; p++) {
        int c = (unsigned char)*p;
        if (c == '.' && !seen_dot) { seen_dot = 1; continue; }
        if (!isdigit(c)) break;
        any = 1;
        if (c == '0' && nd == 0) { if (seen_dot) e10--; continue; }       /* leading zeros */
        if (nd < MAXD) { dig[nd++] = (char)c; if (seen_dot) e10--; }
        else { if (c != '0') dropped_nz = 1; if (!seen_dot) dropped_int++; }
    }
    if (!any) { if (end) *end = (char *)s; return 0.0; }
    if (*p == 'e' || *p == 'E') {
        const char *q = p + 1; int es = 0; long ev = 0;
        if (*q == '+' || *q == '-') es = *q++ == '-';
        if (isdigit((unsigned char)*q)) {
            while (isdigit((unsigned char)*q)) { if (ev < 100000) ev = ev * 10 + (*q - '0'); q++; }
            e10 += es ? -ev : ev;
            p = q;
        }
    }
    if (end) *end = (char *)p;
    e10 += dropped_int;
    if (dropped_nz) { dig[nd++] = '1'; e10--; }
    if (nd == 0) return neg ? -0.0 : 0.0;
    while (nd > 1 && dig[nd - 1] == '0') { nd--; e10++; }

    long mag = nd + e10;
    if (mag > 310) { *range_err = 1; return neg ? -HUGE_VAL : HUGE_VAL; }
    if (mag < -330) { *range_err = 1; return neg ? -0.0 : 0.0; }

    if (mbits == 53 && nd <= 15 && e10 >= -22 && e10 <= 22) {              /* Clinger fast path */
        static const double p10[] = { 1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7, 1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14,
                                      1e15, 1e16, 1e17, 1e18, 1e19, 1e20, 1e21, 1e22 };
        double v = 0;
        for (int i = 0; i < nd; i++) v = v * 10 + (dig[i] - '0');
        v = e10 < 0 ? v / p10[-e10] : v * p10[e10];
        return neg ? -v : v;
    }

    static bn D, P5, R;
    D.n = 0;
    for (int i = 0; i < nd;) {
        uint32_t chunk = 0, mul = 1; int k = 0;
        for (; k < 9 && i < nd; k++, i++) { chunk = chunk * 10 + (uint32_t)(dig[i] - '0'); mul *= 10; }
        bn_mul_small(&D, mul);
        bn_add_small(&D, chunk);
    }
    uint64_t mant; int sticky; long adj, e2;
    if (e10 >= 0) {
        bn_mul_pow10(&D, (int)e10);
        adj = bn_top64(&D, &mant, &sticky);
        e2 = adj;
    } else {
        int k = (int)-e10;
        bn_set(&P5, 1);
        for (int i = k; i >= 13; i -= 13) bn_mul_small(&P5, 1220703125u);   /* 5^13 */
        bn_mul_small(&P5, 1);
        for (int i = k % 13; i > 0; i--) bn_mul_small(&P5, 5);
        int m = 70 + bn_bits(&P5) - bn_bits(&D);
        if (m < 0) m = 0;
        bn_shl(&D, m);
        /* bitwise long division N / P5: quotient into Q (bit i), remainder in R */
        static bn Q;
        Q.n = 0; R.n = 0;
        int top = bn_bits(&D);
        for (int i = top - 1; i >= 0; i--) {
            bn_shl(&R, 1);
            if (bn_bit(&D, i)) { if (!R.n) { R.l[0] = 1; R.n = 1; } else R.l[0] |= 1; }
            if (bn_cmp(&R, &P5) >= 0) {
                bn_sub(&R, &P5);
                int w = i / 32;
                while (Q.n <= w) Q.l[Q.n++] = 0;
                Q.l[w] |= 1u << (i % 32);
            }
        }
        bn_trim(&Q);
        adj = bn_top64(&Q, &mant, &sticky);
        if (R.n) sticky = 1;
        e2 = adj - m - k;
    }
    double r = assemble(mant, sticky, e2, mbits, range_err);
    return neg ? -r : r;
}

double strtod(const char *restrict s, char **restrict end)
{
    int re; double r = __strtod_core(s, end, 53, &re);
    if (re) errno = ERANGE;
    return r;
}
float strtof(const char *restrict s, char **restrict end)
{
    int re; double r = __strtod_core(s, end, 24, &re);
    if (re) errno = ERANGE;
    return (float)r;
}
long double strtold(const char *restrict s, char **restrict end) { return strtod(s, end); }
