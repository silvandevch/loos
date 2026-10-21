/* printf engine: C11 conversions (d i u o x X c s p n % f F e E g G a A), flags - + space # 0, width and
 * precision (including *), length modifiers hh h l ll j z t L. Floating point is rendered from the exact decimal
 * expansion of the value with round-half-even, so output matches a correctly rounding C library digit for
 * digit. Output goes through a sink callback so the stdio and string front ends share one implementation. */
#include "libc.h"
#include <string.h>
#include <stdlib.h>
#include <wchar.h>
#include <limits.h>
#include <math.h>

typedef struct { __fmt_sink sink; void *ctx; size_t count; int error; } out_t;

static void put(out_t *o, const char *s, size_t n)
{
    if (!n) return;
    o->sink(o->ctx, s, n);
    if (o->count > INT_MAX - n) o->error = 1;
    o->count += n;
}
static void pad(out_t *o, char c, size_t n)
{
    char buf[32];
    memset(buf, c, sizeof buf);
    while (n) { size_t k = n < sizeof buf ? n : sizeof buf; put(o, buf, k); n -= k; }
}

/* ---- exact decimal expansion of a double ---- */
#define LIMBS 96
typedef struct { uint32_t l[LIMBS]; int n; } big;

static void big_mul(big *b, uint32_t m)
{
    uint64_t carry = 0;
    for (int i = 0; i < b->n; i++) { uint64_t v = (uint64_t)b->l[i] * m + carry; b->l[i] = (uint32_t)(v % 1000000000u); carry = v / 1000000000u; }
    while (carry && b->n < LIMBS) { b->l[b->n++] = (uint32_t)(carry % 1000000000u); carry /= 1000000000u; }
}

/* value = 0.d[0]d[1]... x 10^*exp10, digits without trailing zeros; returns digit count (0 for zero) */
static int dtoa_exact(double v, char *d, int *exp10)
{
    uint64_t bits, mant; int ex, e2, scale = 0, nd = 0;
    big b; char tmp[10];
    memcpy(&bits, &v, 8);
    ex = (int)((bits >> 52) & 0x7ff); mant = bits & 0xFFFFFFFFFFFFFull;
    if (ex) { mant |= 1ull << 52; e2 = ex - 1075; } else e2 = -1074;
    *exp10 = 0;
    if (!mant) return 0;
    while (!(mant & 1) && e2 < 0) { mant >>= 1; e2++; }
    b.n = 0;
    while (mant) { b.l[b.n++] = (uint32_t)(mant % 1000000000u); mant /= 1000000000u; }
    if (e2 >= 0) { while (e2 >= 29) { big_mul(&b, 1u << 29); e2 -= 29; } if (e2) big_mul(&b, 1u << e2); }
    else { int k = -e2; scale = -k; while (k >= 13) { big_mul(&b, 1220703125u); k -= 13; } while (k--) big_mul(&b, 5); }
    for (int i = b.n - 1; i >= 0; i--) {
        uint32_t x = b.l[i];
        for (int j = 8; j >= 0; j--) { tmp[j] = (char)('0' + x % 10); x /= 10; }
        for (int j = 0; j < 9; j++) { if (!nd && tmp[j] == '0') continue; d[nd++] = tmp[j]; }
    }
    *exp10 = nd + scale;
    while (nd > 0 && d[nd - 1] == '0') nd--;
    return nd;
}

/* keep the first `keep` digits, rounding half to even on the exact value */
static int round_digits(char *d, int nd, int keep, int *exp10)
{
    if (keep >= nd) return nd;
    if (keep < 0) return 0;
    int up = d[keep] > '5' || (d[keep] == '5' && (nd > keep + 1 || (keep > 0 && ((d[keep - 1] - '0') & 1))));
    nd = keep;
    if (up) {
        int i;
        for (i = keep - 1; i >= 0 && d[i] == '9'; i--) { }
        if (i < 0) { d[0] = '1'; nd = 1; (*exp10)++; } else { d[i]++; nd = i + 1; }
    }
    while (nd > 0 && d[nd - 1] == '0') nd--;
    return nd;
}

typedef struct { int left, plus, space, zero, alt, width, prec; } spec;

static void emit_num(out_t *o, const spec *s, char sign, const char *prefix, const char *body, size_t blen, int zeros_ok)
{
    size_t total = (sign ? 1 : 0) + strlen(prefix) + blen;
    size_t padn = (size_t)s->width > total ? (size_t)s->width - total : 0;
    if (!s->left && !(zeros_ok && s->zero)) pad(o, ' ', padn);
    if (sign) put(o, &sign, 1);
    put(o, prefix, strlen(prefix));
    if (!s->left && zeros_ok && s->zero) pad(o, '0', padn);
    put(o, body, blen);
    if (s->left) pad(o, ' ', padn);
}

#define MAXPREC 1100
static void fmt_float(out_t *o, const spec *s, double v, char conv)
{
    static char body[MAXPREC + 360 + 16], dig[800], tmp[800];
    int exp10, nd, n = 0;
    int prec = s->prec < 0 ? 6 : (s->prec > MAXPREC ? MAXPREC : s->prec);
    int upper = conv == 'E' || conv == 'G' || conv == 'F' || conv == 'A';
    char sign = 0, lc = (char)(conv | 0x20);
    uint64_t bits;
    memcpy(&bits, &v, 8);
    if (bits >> 63) sign = '-'; else if (s->plus) sign = '+'; else if (s->space) sign = ' ';
    if (((bits >> 52) & 0x7ff) == 0x7ff) {
        const char *t = (bits & 0xFFFFFFFFFFFFFull) ? (upper ? "NAN" : "nan") : (upper ? "INF" : "inf");
        emit_num(o, s, sign, "", t, 3, 0);
        return;
    }
    if (lc == 'a') {                                                  /* hexadecimal floating point */
        const char *hx = upper ? "0123456789ABCDEF" : "0123456789abcdef";
        uint64_t mant = bits & 0xFFFFFFFFFFFFFull; int ex = (int)((bits >> 52) & 0x7ff), lead = 1;
        if (ex == 0) { if (mant) { ex = -1022; lead = 0; } else { ex = 0; lead = 0; } } else ex -= 1023;
        int digits = 13;
        if (s->prec >= 0 && s->prec < 13) {                            /* round the 52-bit fraction to prec hex digits */
            int drop = (13 - s->prec) * 4;
            uint64_t full = ((uint64_t)lead << 52) | mant, half = 1ULL << (drop - 1), rem = full & ((1ULL << drop) - 1);
            full >>= drop;
            if (rem > half || (rem == half && (full & 1))) full++;
            full <<= drop;
            lead = (int)(full >> 52); mant = full & 0xFFFFFFFFFFFFFull;
            digits = s->prec;
        } else if (s->prec < 0) { while (digits > 0 && !((mant >> ((13 - digits) * 4)) & 0xF)) digits--; }
        else digits = s->prec;
        body[n++] = (char)('0' + lead);
        if (digits > 0 || s->alt) body[n++] = '.';
        for (int i = 0; i < digits; i++) body[n++] = i < 13 ? hx[(mant >> ((12 - i) * 4)) & 0xF] : '0';
        body[n++] = upper ? 'P' : 'p';
        body[n++] = ex < 0 ? '-' : '+';
        { char t[8]; int k = 0; unsigned e = (unsigned)(ex < 0 ? -ex : ex); do t[k++] = (char)('0' + e % 10); while (e /= 10); while (k) body[n++] = t[--k]; }
        emit_num(o, s, sign, upper ? "0X" : "0x", body, (size_t)n, 1);
        return;
    }
    nd = dtoa_exact(v, dig, &exp10);
    if (lc == 'g') {
        int P = s->prec < 0 ? 6 : (prec == 0 ? 1 : prec), X, e2 = exp10, nd2;
        memcpy(tmp, dig, (size_t)nd);
        nd2 = round_digits(tmp, nd, P, &e2);
        X = nd2 ? e2 - 1 : 0;
        if (X < -4 || X >= P) { lc = 'e'; prec = P - 1; } else { lc = 'f'; prec = P - 1 - X; }
        if (!s->alt) {
            int used = nd2 ? nd2 : 1;
            if (lc == 'e') prec = used - 1; else { prec = used - 1 - X; if (prec < 0) prec = 0; }
        }
    }
    if (lc == 'f') {
        int keep = exp10 + prec;
        nd = round_digits(dig, nd, keep, &exp10);
        int ip = exp10 > 0 ? exp10 : 1;
        for (int k = 0; k < ip; k++) body[n++] = (exp10 > 0 && k < nd) ? dig[k] : '0';
        if (prec > 0 || s->alt) body[n++] = '.';
        for (int k = 0; k < prec; k++) { int idx = exp10 + k; body[n++] = (idx >= 0 && idx < nd) ? dig[idx] : '0'; }
    } else {
        nd = round_digits(dig, nd, prec + 1, &exp10);
        int e = nd ? exp10 - 1 : 0;
        body[n++] = nd ? dig[0] : '0';
        if (prec > 0 || s->alt) body[n++] = '.';
        for (int i = 1; i <= prec; i++) body[n++] = i < nd ? dig[i] : '0';
        body[n++] = upper ? 'E' : 'e';
        body[n++] = e < 0 ? '-' : '+';
        if (e < 0) e = -e;
        if (e >= 100) body[n++] = (char)('0' + e / 100);
        body[n++] = (char)('0' + (e / 10) % 10);
        body[n++] = (char)('0' + e % 10);
    }
    emit_num(o, s, sign, "", body, (size_t)n, 1);
}

static void fmt_int(out_t *o, const spec *s, uint64_t u, int is_signed, int neg, int base, int upper)
{
    char rev[80], body[96];
    int rn = 0, bn = 0;
    const char *dg = upper ? "0123456789ABCDEF" : "0123456789abcdef", *prefix = "";
    char sign = 0;
    if (is_signed) { if (neg) sign = '-'; else if (s->plus) sign = '+'; else if (s->space) sign = ' '; }
    if (!(s->prec == 0 && u == 0)) { if (u == 0) rev[rn++] = '0'; while (u && rn < 70) { rev[rn++] = dg[u % (unsigned)base]; u /= (unsigned)base; } }
    if (base == 8 && s->alt && (rn == 0 || rev[rn - 1] != '0')) rev[rn++] = '0';
    if (base == 16 && s->alt && rn && !(rn == 1 && rev[0] == '0')) prefix = upper ? "0X" : "0x";
    for (int i = 0; i < s->prec - rn && bn < 90; i++) body[bn++] = '0';
    while (rn) body[bn++] = rev[--rn];
    emit_num(o, s, sign, prefix, body, (size_t)bn, s->prec < 0);
}

static size_t utf8_encode(char *out, unsigned c)
{
    if (c < 0x80) { out[0] = (char)c; return 1; }
    if (c < 0x800) { out[0] = (char)(0xC0 | c >> 6); out[1] = (char)(0x80 | (c & 63)); return 2; }
    if (c < 0x10000) { out[0] = (char)(0xE0 | c >> 12); out[1] = (char)(0x80 | ((c >> 6) & 63)); out[2] = (char)(0x80 | (c & 63)); return 3; }
    out[0] = (char)(0xF0 | c >> 18); out[1] = (char)(0x80 | ((c >> 12) & 63)); out[2] = (char)(0x80 | ((c >> 6) & 63)); out[3] = (char)(0x80 | (c & 63));
    return 4;
}

int __vformat(__fmt_sink sink, void *ctx, const char *fmt, va_list ap)
{
    out_t o = { sink, ctx, 0, 0 };
    for (const char *p = fmt; *p;) {
        if (*p != '%') { const char *q = p; while (*q && *q != '%') q++; put(&o, p, (size_t)(q - p)); p = q; continue; }
        p++;
        spec s = { 0, 0, 0, 0, 0, 0, -1 };
        for (;; p++) {
            if (*p == '-') s.left = 1; else if (*p == '+') s.plus = 1; else if (*p == ' ') s.space = 1;
            else if (*p == '#') s.alt = 1; else if (*p == '0') s.zero = 1; else break;
        }
        if (*p == '*') { int w = va_arg(ap, int); if (w < 0) { s.left = 1; w = -w; } s.width = w; p++; }
        else while (*p >= '0' && *p <= '9') { s.width = s.width * 10 + (*p++ - '0'); if (s.width > 1 << 24) s.width = 1 << 24; }
        if (*p == '.') {
            p++; s.prec = 0;
            if (*p == '*') { int pr = va_arg(ap, int); s.prec = pr < 0 ? -1 : pr; p++; }
            else while (*p >= '0' && *p <= '9') { s.prec = s.prec * 10 + (*p++ - '0'); if (s.prec > 1 << 24) s.prec = 1 << 24; }
        }
        if (s.left) s.zero = 0;
        int len = 0;                                     /* -2 hh, -1 h, 0 int, 1 l, 2 ll, 3 j, 4 z, 5 t, 6 L */
        for (;; p++) {
            if (*p == 'h') len = len == -1 ? -2 : -1;
            else if (*p == 'l') len = len == 1 ? 2 : 1;
            else if (*p == 'j') len = 3; else if (*p == 'z') len = 4; else if (*p == 't') len = 5; else if (*p == 'L') len = 6;
            else break;
        }
        char c = *p;
        if (!c) break;
        p++;
        switch (c) {
        case '%': put(&o, "%", 1); break;
        case 'c': {
            char buf[4]; size_t n;
            unsigned ch = va_arg(ap, int);
            n = len == 1 ? utf8_encode(buf, ch) : (buf[0] = (char)ch, 1);
            size_t padn = (size_t)s.width > n ? (size_t)s.width - n : 0;
            if (!s.left) pad(&o, s.zero ? '0' : ' ', padn);
            put(&o, buf, n);
            if (s.left) pad(&o, ' ', padn);
            break;
        }
        case 's': {
            if (len == 1) {                                         /* wide string -> UTF-8 */
                const wchar_t *ws = va_arg(ap, const wchar_t *);
                char tmp[4]; size_t total = 0;
                if (!ws) ws = L"(null)";
                for (const wchar_t *q = ws; *q; q++) { size_t n = utf8_encode(tmp, (unsigned)*q); if (s.prec >= 0 && total + n > (size_t)s.prec) break; total += n; }
                size_t padn = (size_t)s.width > total ? (size_t)s.width - total : 0;
                if (!s.left) pad(&o, s.zero ? '0' : ' ', padn);
                for (size_t done = 0; *ws && done < total; ws++) { size_t n = utf8_encode(tmp, (unsigned)*ws); put(&o, tmp, n); done += n; }
                if (s.left) pad(&o, ' ', padn);
                break;
            }
            const char *str = va_arg(ap, const char *);
            if (!str) str = s.prec >= 0 && s.prec < 6 ? "" : "(null)";
            size_t n = s.prec >= 0 ? strnlen(str, (size_t)s.prec) : strlen(str);
            size_t padn = (size_t)s.width > n ? (size_t)s.width - n : 0;
            if (!s.left) pad(&o, s.zero ? '0' : ' ', padn);
            put(&o, str, n);
            if (s.left) pad(&o, ' ', padn);
            break;
        }
        case 'd': case 'i': {
            long long v;
            switch (len) {
            case -2: v = (signed char)va_arg(ap, int); break;
            case -1: v = (short)va_arg(ap, int); break;
            case 1: v = va_arg(ap, long); break;
            case 2: v = va_arg(ap, long long); break;
            case 3: v = va_arg(ap, intmax_t); break;
            case 4: v = va_arg(ap, ptrdiff_t); break;
            case 5: v = va_arg(ap, ptrdiff_t); break;
            default: v = va_arg(ap, int);
            }
            fmt_int(&o, &s, v < 0 ? 0 - (uint64_t)v : (uint64_t)v, 1, v < 0, 10, 0);
            break;
        }
        case 'u': case 'o': case 'x': case 'X': {
            uint64_t v;
            switch (len) {
            case -2: v = (unsigned char)va_arg(ap, unsigned); break;
            case -1: v = (unsigned short)va_arg(ap, unsigned); break;
            case 1: v = va_arg(ap, unsigned long); break;
            case 2: v = va_arg(ap, unsigned long long); break;
            case 3: v = va_arg(ap, uintmax_t); break;
            case 4: v = va_arg(ap, size_t); break;
            case 5: v = (uint64_t)va_arg(ap, ptrdiff_t); break;
            default: v = va_arg(ap, unsigned);
            }
            fmt_int(&o, &s, v, 0, 0, c == 'u' ? 10 : c == 'o' ? 8 : 16, c == 'X');
            break;
        }
        case 'p': {
            void *v = va_arg(ap, void *);
            if (!v) { spec t = s; t.zero = 0; emit_num(&o, &t, 0, "", "(nil)", 5, 0); }
            else { spec t = s; t.alt = 1; fmt_int(&o, &t, (uintptr_t)v, 0, 0, 16, 0); }
            break;
        }
        case 'f': case 'F': case 'e': case 'E': case 'g': case 'G': case 'a': case 'A': {
            double v = len == 6 ? (double)va_arg(ap, long double) : va_arg(ap, double);
            fmt_float(&o, &s, v, c);
            break;
        }
        case 'n': {
            void *t = va_arg(ap, void *);
            switch (len) {
            case -2: *(signed char *)t = (signed char)o.count; break;
            case -1: *(short *)t = (short)o.count; break;
            case 1: *(long *)t = (long)o.count; break;
            case 2: *(long long *)t = (long long)o.count; break;
            case 3: *(intmax_t *)t = (intmax_t)o.count; break;
            case 4: *(size_t *)t = o.count; break;
            case 5: *(ptrdiff_t *)t = (ptrdiff_t)o.count; break;
            default: *(int *)t = (int)o.count;
            }
            break;
        }
        default:                                                     /* unknown conversion: emit verbatim */
            put(&o, "%", 1); put(&o, &c, 1);
        }
    }
    return o.error ? -1 : (int)o.count;
}
