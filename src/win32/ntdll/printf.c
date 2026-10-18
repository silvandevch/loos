/* ntdll's printf family (sprintf, _snprintf, _vsnprintf, swprintf, _snwprintf, ... and the _s forms) with
 * msvcrt semantics: `%s` is the width of the format string, `%S`/`%ws`/`%hs` flip it, `%ld` is 32-bit,
 * `%I64d`/`%Ix` are 64-bit/pointer-sized, `%p` prints 16 upper-case hex digits. Floating point is formatted
 * from the exact decimal expansion of the double (big-number arithmetic, round-half-even), so output matches
 * a correctly rounding C library digit for digit. Truncation follows the legacy contract: _vsnprintf returns
 * -1 when the result does not fit and only NUL-terminates when it does. */
#include "../include/loos_nt.h"
#include <string.h>

#ifdef _WIN32
int _fltused = 1;                 /* the MSVC-ABI marker that floating point is in use; no CRT supplies it here */
#endif

#define PF_MAXPREC 600

typedef struct { void *buf; size_t cap, n; int wide; } out_t;

static void put(out_t *o, unsigned c)
{
    if (o->n < o->cap) { if (o->wide) ((WCHAR *)o->buf)[o->n] = (WCHAR)c; else ((char *)o->buf)[o->n] = (char)c; }
    o->n++;
}
static void pad(out_t *o, int c, size_t n) { while (n--) put(o, (unsigned)c); }

/* ---- exact decimal expansion of a double ---- */
#define LIMBS 96
typedef struct { uint32_t l[LIMBS]; int n; } big;                    /* base 1e9, little endian */

static void big_mul(big *b, uint32_t m)
{
    uint64_t carry = 0; int i;
    for (i = 0; i < b->n; i++) { uint64_t v = (uint64_t)b->l[i] * m + carry; b->l[i] = (uint32_t)(v % 1000000000u); carry = v / 1000000000u; }
    while (carry && b->n < LIMBS) { b->l[b->n++] = (uint32_t)(carry % 1000000000u); carry /= 1000000000u; }
}

/* value = 0.d[0]d[1]... x 10^*exp10, digits without trailing zeros; returns digit count (0 for zero) */
static int dtoa_exact(double v, char *d, int *exp10)
{
    uint64_t bits, mant; int ex, e2, scale = 0, i, nd = 0, k;
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
    else { k = -e2; scale = -k; while (k >= 13) { big_mul(&b, 1220703125u); k -= 13; } while (k--) big_mul(&b, 5); }
    for (i = b.n - 1; i >= 0; i--) {
        int j; uint32_t x = b.l[i];
        for (j = 8; j >= 0; j--) { tmp[j] = (char)('0' + x % 10); x /= 10; }
        for (j = 0; j < 9; j++) { if (!nd && tmp[j] == '0') continue; d[nd++] = tmp[j]; }
    }
    *exp10 = nd + scale;
    while (nd > 0 && d[nd - 1] == '0') nd--;
    return nd;
}

/* keep the first `keep` digits, rounding half to even on the exact value; returns the new digit count */
static int round_digits(char *d, int nd, int keep, int *exp10)
{
    int up, i;
    if (keep >= nd) return nd;
    if (keep < 0) return 0;
    up = d[keep] > '5' || (d[keep] == '5' && (nd > keep + 1 || (keep > 0 && ((d[keep - 1] - '0') & 1))));
    nd = keep;
    if (up) {
        for (i = keep - 1; i >= 0 && d[i] == '9'; i--) ;
        if (i < 0) { d[0] = '1'; nd = 1; (*exp10)++; } else { d[i]++; nd = i + 1; }
    }
    while (nd > 0 && d[nd - 1] == '0') nd--;
    return nd;
}

typedef struct { int left, plus, space, zero, alt, width, prec; } spec;

/* emit sign + body with width handling; `zeros_ok` allows 0-padding between sign and body */
static void emit_num(out_t *o, const spec *s, char sign, const char *prefix, const char *body, size_t blen, int zeros_ok)
{
    size_t total = (sign ? 1 : 0) + strlen(prefix) + blen, padn = (size_t)s->width > total ? (size_t)s->width - total : 0;
    if (!s->left && !(zeros_ok && s->zero)) pad(o, ' ', padn);
    if (sign) put(o, (unsigned char)sign);
    while (*prefix) put(o, (unsigned char)*prefix++);
    if (!s->left && zeros_ok && s->zero) pad(o, '0', padn);
    { size_t i; for (i = 0; i < blen; i++) put(o, (unsigned char)body[i]); }
    if (s->left) pad(o, ' ', padn);
}

static void fmt_float(out_t *o, const spec *s, double v, char conv)
{
    static char body[PF_MAXPREC + 360 + 16];
    char dig[800]; int exp10, nd, prec = s->prec < 0 ? 6 : (s->prec > PF_MAXPREC ? PF_MAXPREC : s->prec);
    int upper = conv == 'E' || conv == 'G' || conv == 'F', n = 0, i;
    char sign = 0, lc = (char)(conv | 0x20);
    uint64_t bits;
    memcpy(&bits, &v, 8);
    if (bits >> 63) sign = '-'; else if (s->plus) sign = '+'; else if (s->space) sign = ' ';
    if (((bits >> 52) & 0x7ff) == 0x7ff) {                          /* inf / nan: never zero padded */
        const char *t = (bits & 0xFFFFFFFFFFFFFull) ? (upper ? "NAN" : "nan") : (upper ? "INF" : "inf");
        emit_num(o, s, sign, "", t, 3, 0);
        return;
    }
    nd = dtoa_exact(v, dig, &exp10);
    if (lc == 'g') {
        int P = s->prec < 0 ? 6 : (prec == 0 ? 1 : prec), X;
        char tmp[800]; int e2 = exp10, nd2;
        memcpy(tmp, dig, (size_t)nd);
        nd2 = round_digits(tmp, nd, P, &e2);
        X = nd2 ? e2 - 1 : 0;
        if (X < -4 || X >= P) { lc = 'e'; prec = P - 1; } else { lc = 'f'; prec = P - 1 - X; }
        if (!s->alt) {                                              /* %g drops trailing zeros: recompute the precision */
            int used = nd2 ? nd2 : 1;
            if (lc == 'e') prec = used - 1; else { prec = used - 1 - X; if (prec < 0) prec = 0; }
        }
    }
    if (lc == 'f') {
        int keep = exp10 + prec, ip, k;
        nd = round_digits(dig, nd, keep, &exp10);
        ip = exp10 > 0 ? exp10 : 1;
        for (k = 0; k < ip; k++) body[n++] = (exp10 > 0 && k < nd) ? dig[k] : '0';
        if (prec > 0 || s->alt) body[n++] = '.';
        for (k = 0; k < prec; k++) { int idx = exp10 + k; body[n++] = (idx >= 0 && idx < nd) ? dig[idx] : '0'; }
    } else {
        int e;
        nd = round_digits(dig, nd, prec + 1, &exp10);
        e = nd ? exp10 - 1 : 0;
        body[n++] = nd ? dig[0] : '0';
        if (prec > 0 || s->alt) body[n++] = '.';
        for (i = 1; i <= prec; i++) body[n++] = i < nd ? dig[i] : '0';
        body[n++] = upper ? 'E' : 'e';
        body[n++] = e < 0 ? '-' : '+';
        if (e < 0) e = -e;
        if (e >= 100) body[n++] = (char)('0' + e / 100);
        body[n++] = (char)('0' + (e / 10) % 10);
        body[n++] = (char)('0' + e % 10);
    }
    emit_num(o, s, sign, "", body, (size_t)n, 1);
}

static void fmt_int(out_t *o, const spec *s, uint64_t u, int is_signed, int neg, int base, int upper, int is_ptr)
{
    char rev[80], body[80]; int rn = 0, bn = 0, i;
    const char *dg = upper ? "0123456789ABCDEF" : "0123456789abcdef";
    const char *prefix = "";
    char sign = 0;
    int prec = s->prec;
    if (is_signed) { if (neg) sign = '-'; else if (s->plus) sign = '+'; else if (s->space) sign = ' '; }
    if (!(prec == 0 && u == 0)) { if (u == 0) rev[rn++] = '0'; while (u && rn < 70) { rev[rn++] = dg[u % (unsigned)base]; u /= (unsigned)base; } }
    if (is_ptr) prec = 16;
    while (prec > rn && rn < 70) rev[rn++] = '0';
    if (base == 8 && s->alt && (rn == 0 || rev[rn - 1] != '0')) rev[rn++] = '0';
    if (base == 16 && s->alt && !is_ptr && rn && !(rn == 1 && rev[0] == '0')) prefix = upper ? "0X" : "0x";
    for (i = rn - 1; i >= 0; i--) body[bn++] = rev[i];
    {   spec t = *s;
        if (s->prec >= 0 || is_ptr) t.zero = 0;                      /* an explicit precision disables 0-padding */
        emit_num(o, &t, sign, prefix, body, (size_t)bn, 1);
    }
}

static unsigned fetch(const void *f, int wide, size_t i) { return wide ? ((const WCHAR *)f)[i] : ((const unsigned char *)f)[i]; }

static void emit_text(out_t *o, const spec *s, const void *str, int str_wide, size_t len)
{
    size_t padn = (size_t)s->width > len ? (size_t)s->width - len : 0, i;
    if (!s->left) pad(o, s->zero ? '0' : ' ', padn);
    for (i = 0; i < len; i++) {
        unsigned c = str_wide ? ((const WCHAR *)str)[i] : ((const unsigned char *)str)[i];
        if (str_wide && !o->wide && c > 0x7f) c = '?';               /* C-locale wide to multibyte */
        put(o, c);
    }
    if (s->left) pad(o, ' ', padn);
}

static void engine(out_t *o, const void *fmt, LOOS_VA_LIST *ap)
{
    int w = o->wide;
    size_t i = 0;
    unsigned c;
    while ((c = fetch(fmt, w, i)) != 0) {
        spec s; int sz = 0 /* 1 h, 2 hh, 3 l, 4 ll/I64, 5 w, 6 I(ptr), 7 I32 */, done = 0;
        if (c != '%') { put(o, c); i++; continue; }
        i++;
        memset(&s, 0, sizeof s); s.prec = -1;
        for (;;) {                                                   /* flags */
            c = fetch(fmt, w, i);
            if (c == '-') s.left = 1; else if (c == '+') s.plus = 1; else if (c == ' ') s.space = 1; else if (c == '0') s.zero = 1; else if (c == '#') s.alt = 1; else break;
            i++;
        }
        if (fetch(fmt, w, i) == '*') { int v = LOOS_VA_ARG(*ap, int); i++; if (v < 0) { s.left = 1; v = -v; } s.width = v; }
        else while ((c = fetch(fmt, w, i)) >= '0' && c <= '9') { s.width = s.width * 10 + (int)(c - '0'); if (s.width > 100000) s.width = 100000; i++; }
        if (fetch(fmt, w, i) == '.') {
            i++; s.prec = 0;
            if (fetch(fmt, w, i) == '*') { int v = LOOS_VA_ARG(*ap, int); i++; s.prec = v < 0 ? -1 : v; }
            else while ((c = fetch(fmt, w, i)) >= '0' && c <= '9') { s.prec = s.prec * 10 + (int)(c - '0'); if (s.prec > 100000) s.prec = 100000; i++; }
        }
        for (; !done;) {                                             /* length modifiers */
            c = fetch(fmt, w, i);
            if (c == 'h') { sz = sz == 1 ? 2 : 1; i++; }
            else if (c == 'l') { sz = sz == 3 ? 4 : 3; i++; }
            else if (c == 'w') { sz = 5; i++; }
            else if (c == 'L' || c == 'j' || c == 'z' || c == 't' || c == 'q') { sz = (c == 'L') ? 0 : 6; i++; }
            else if (c == 'I') {
                if (fetch(fmt, w, i + 1) == '6' && fetch(fmt, w, i + 2) == '4') { sz = 4; i += 3; }
                else if (fetch(fmt, w, i + 1) == '3' && fetch(fmt, w, i + 2) == '2') { sz = 7; i += 3; }
                else { sz = 6; i++; }
            } else done = 1;
        }
        c = fetch(fmt, w, i);
        if (!c) break;
        i++;
        switch (c) {
        case '%': put(o, '%'); break;
        case 'd': case 'i': case 'u': case 'o': case 'x': case 'X': {
            int sg = (c == 'd' || c == 'i'), base = c == 'o' ? 8 : (c == 'x' || c == 'X') ? 16 : 10, neg = 0;
            uint64_t u;
            if (sz == 4 || sz == 6) u = LOOS_VA_ARG(*ap, uint64_t);
            else {
                uint32_t v = LOOS_VA_ARG(*ap, uint32_t);
                if (sz == 1) u = sg ? (uint64_t)(int64_t)(int16_t)v : (uint16_t)v;
                else if (sz == 2) u = sg ? (uint64_t)(int64_t)(int8_t)v : (uint8_t)v;
                else u = sg ? (uint64_t)(int64_t)(int32_t)v : v;
            }
            if (sg && (int64_t)u < 0) { neg = 1; u = 0 - u; }
            fmt_int(o, &s, u, sg, neg, base, c == 'X', 0);
            break;
        }
        case 'p': fmt_int(o, &s, LOOS_VA_ARG(*ap, uint64_t), 0, 0, 16, 1, 1); break;
        case 'c': case 'C': {
            unsigned ch = LOOS_VA_ARG(*ap, unsigned);
            int cw = (c == 'c') ? w : !w;
            if (sz == 1) cw = 0; else if (sz == 3 || sz == 5) cw = 1;
            if (cw) { WCHAR t = (WCHAR)ch; emit_text(o, &s, &t, 1, 1); } else { unsigned char t = (unsigned char)ch; emit_text(o, &s, &t, 0, 1); }
            break;
        }
        case 's': case 'S': case 'Z': {
            const void *p = LOOS_VA_ARG(*ap, const void *);
            int sw = (c == 's') ? w : !w;
            size_t n = 0;
            if (sz == 1) sw = 0; else if (sz == 3 || sz == 5) sw = 1;
            if (!p) { p = "(null)"; sw = 0; }
            if (sw) while (((const WCHAR *)p)[n] && (s.prec < 0 || (int)n < s.prec)) n++;
            else    while (((const char *)p)[n] && (s.prec < 0 || (int)n < s.prec)) n++;
            emit_text(o, &s, p, sw, n);
            break;
        }
        case 'e': case 'E': case 'f': case 'F': case 'g': case 'G': fmt_float(o, &s, LOOS_VA_ARG(*ap, double), (char)c); break;
        case 'n': (void)LOOS_VA_ARG(*ap, void *); break;             /* %n is not honoured */
        default: put(o, '%'); put(o, c); break;
        }
    }
}

/* returns the untruncated length; writes at most cap units and never a terminator */
static size_t run(void *buf, size_t cap, int wide, const void *fmt, LOOS_VA_LIST ap)
{
    out_t o; o.buf = buf; o.cap = buf ? cap : 0; o.n = 0; o.wide = wide;
    engine(&o, fmt, &ap);
    return o.n;
}
static void term(void *buf, size_t at, int wide) { if (wide) ((WCHAR *)buf)[at] = 0; else ((char *)buf)[at] = 0; }

/* legacy _vsnprintf: -1 when it does not fit, NUL only when there is room for it */
static int legacy(void *buf, size_t count, int wide, const void *fmt, LOOS_VA_LIST ap)
{
    size_t n = run(buf, count, wide, fmt, ap);
    if (!buf) return (int)n;
    if (n < count) { term(buf, n, wide); return (int)n; }
    return n == count ? (int)n : -1;
}
static int unbounded(void *buf, int wide, const void *fmt, LOOS_VA_LIST ap)
{
    size_t n = run(buf, (size_t)-1 >> 4, wide, fmt, ap);
    term(buf, n, wide);
    return (int)n;
}
/* secure: fails and empties the buffer when the result (plus NUL) does not fit */
static int secure(void *buf, size_t size, size_t count, int wide, const void *fmt, LOOS_VA_LIST ap)
{
    size_t lim, n;
    if (!buf || !size) return -1;
    lim = count == (size_t)-1 ? size - 1 : (count < size ? count : size - 1);
    n = run(buf, lim, wide, fmt, ap);
    if (n <= lim) { term(buf, n, wide); return (int)n; }
    if (count == (size_t)-1 || count < size) { term(buf, lim, wide); return -1; }   /* _TRUNCATE or an explicit short count */
    term(buf, 0, wide);
    return -1;
}

int LOOS_CRT LOOS_CRTNAME(_vsnprintf)(char *b, size_t n, const char *f, LOOS_VA_LIST ap) { return legacy(b, n, 0, f, ap); }
int LOOS_CRT LOOS_CRTNAME(_vsnwprintf)(WCHAR *b, size_t n, const WCHAR *f, LOOS_VA_LIST ap) { return legacy(b, n, 1, f, ap); }
int LOOS_CRT LOOS_CRTNAME(vsprintf)(char *b, const char *f, LOOS_VA_LIST ap) { return unbounded(b, 0, f, ap); }
int LOOS_CRT LOOS_CRTNAME(vswprintf)(WCHAR *b, const WCHAR *f, LOOS_VA_LIST ap) { return unbounded(b, 1, f, ap); }
int LOOS_CRT LOOS_CRTNAME(vsprintf_s)(char *b, size_t sz, const char *f, LOOS_VA_LIST ap) { return secure(b, sz, sz, 0, f, ap); }
int LOOS_CRT LOOS_CRTNAME(vswprintf_s)(WCHAR *b, size_t sz, const WCHAR *f, LOOS_VA_LIST ap) { return secure(b, sz, sz, 1, f, ap); }
int LOOS_CRT LOOS_CRTNAME(_vsnprintf_s)(char *b, size_t sz, size_t cnt, const char *f, LOOS_VA_LIST ap) { return secure(b, sz, cnt, 0, f, ap); }
int LOOS_CRT LOOS_CRTNAME(_vsnwprintf_s)(WCHAR *b, size_t sz, size_t cnt, const WCHAR *f, LOOS_VA_LIST ap) { return secure(b, sz, cnt, 1, f, ap); }

int LOOS_CRT LOOS_CRTNAME(sprintf)(char *b, const char *f, ...) { LOOS_VA_LIST ap; int r; LOOS_VA_START(ap, f); r = LOOS_CRTNAME(vsprintf)(b, f, ap); LOOS_VA_END(ap); return r; }
int LOOS_CRT LOOS_CRTNAME(swprintf)(WCHAR *b, const WCHAR *f, ...) { LOOS_VA_LIST ap; int r; LOOS_VA_START(ap, f); r = LOOS_CRTNAME(vswprintf)(b, f, ap); LOOS_VA_END(ap); return r; }
int LOOS_CRT LOOS_CRTNAME(_snprintf)(char *b, size_t n, const char *f, ...) { LOOS_VA_LIST ap; int r; LOOS_VA_START(ap, f); r = LOOS_CRTNAME(_vsnprintf)(b, n, f, ap); LOOS_VA_END(ap); return r; }
int LOOS_CRT LOOS_CRTNAME(_snwprintf)(WCHAR *b, size_t n, const WCHAR *f, ...) { LOOS_VA_LIST ap; int r; LOOS_VA_START(ap, f); r = LOOS_CRTNAME(_vsnwprintf)(b, n, f, ap); LOOS_VA_END(ap); return r; }
int LOOS_CRT LOOS_CRTNAME(sprintf_s)(char *b, size_t sz, const char *f, ...) { LOOS_VA_LIST ap; int r; LOOS_VA_START(ap, f); r = LOOS_CRTNAME(vsprintf_s)(b, sz, f, ap); LOOS_VA_END(ap); return r; }
int LOOS_CRT LOOS_CRTNAME(swprintf_s)(WCHAR *b, size_t sz, const WCHAR *f, ...) { LOOS_VA_LIST ap; int r; LOOS_VA_START(ap, f); r = LOOS_CRTNAME(vswprintf_s)(b, sz, f, ap); LOOS_VA_END(ap); return r; }
int LOOS_CRT LOOS_CRTNAME(_snprintf_s)(char *b, size_t sz, size_t cnt, const char *f, ...) { LOOS_VA_LIST ap; int r; LOOS_VA_START(ap, f); r = LOOS_CRTNAME(_vsnprintf_s)(b, sz, cnt, f, ap); LOOS_VA_END(ap); return r; }
int LOOS_CRT LOOS_CRTNAME(_snwprintf_s)(WCHAR *b, size_t sz, size_t cnt, const WCHAR *f, ...) { LOOS_VA_LIST ap; int r; LOOS_VA_START(ap, f); r = LOOS_CRTNAME(_vsnwprintf_s)(b, sz, cnt, f, ap); LOOS_VA_END(ap); return r; }
