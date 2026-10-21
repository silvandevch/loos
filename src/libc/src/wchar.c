/* Wide-character support (wchar_t is a 32-bit code point; the multibyte encoding is UTF-8, locale "C" and
 * "C.UTF-8" behave alike), plus the C locale and signals. */
#include <wchar.h>
#include <wctype.h>
#include <stdlib.h>
#include <string.h>
#include <errno.h>
#include <locale.h>
#include <signal.h>
#include "libc.h"

size_t wcslen(const wchar_t *s) { const wchar_t *p = s; while (*p) p++; return (size_t)(p - s); }
wchar_t *wcscpy(wchar_t *restrict d, const wchar_t *restrict s) { wchar_t *r = d; while ((*d++ = *s++)) { } return r; }
wchar_t *wcsncpy(wchar_t *restrict d, const wchar_t *restrict s, size_t n) { size_t i = 0; for (; i < n && s[i]; i++) d[i] = s[i]; for (; i < n; i++) d[i] = 0; return d; }
wchar_t *wcscat(wchar_t *restrict d, const wchar_t *restrict s) { wcscpy(d + wcslen(d), s); return d; }
int wcscmp(const wchar_t *a, const wchar_t *b) { while (*a && *a == *b) { a++; b++; } return *a < *b ? -1 : *a > *b; }
int wcsncmp(const wchar_t *a, const wchar_t *b, size_t n) { for (; n; n--, a++, b++) if (*a != *b || !*a) return *a < *b ? -1 : *a > *b; return 0; }
wchar_t *wcschr(const wchar_t *s, wchar_t c) { for (;; s++) { if (*s == c) return (wchar_t *)s; if (!*s) return 0; } }
wchar_t *wcsrchr(const wchar_t *s, wchar_t c) { const wchar_t *l = 0; for (;; s++) { if (*s == c) l = s; if (!*s) return (wchar_t *)l; } }
wchar_t *wcsstr(const wchar_t *h, const wchar_t *n)
{
    size_t nl = wcslen(n);
    if (!nl) return (wchar_t *)h;
    for (; *h; h++) if (!wcsncmp(h, n, nl)) return (wchar_t *)h;
    return 0;
}
wchar_t *wmemcpy(wchar_t *restrict d, const wchar_t *restrict s, size_t n) { return memcpy(d, s, n * sizeof *d); }
wchar_t *wmemmove(wchar_t *d, const wchar_t *s, size_t n) { return memmove(d, s, n * sizeof *d); }
wchar_t *wmemset(wchar_t *d, wchar_t c, size_t n) { for (size_t i = 0; i < n; i++) d[i] = c; return d; }
int wmemcmp(const wchar_t *a, const wchar_t *b, size_t n) { for (; n; n--, a++, b++) if (*a != *b) return *a < *b ? -1 : 1; return 0; }
wchar_t *wmemchr(const wchar_t *s, wchar_t c, size_t n) { for (; n; n--, s++) if (*s == c) return (wchar_t *)s; return 0; }

wint_t btowc(int c) { return (c >= 0 && c < 0x80) ? (wint_t)c : WEOF; }
int wctob(wint_t c) { return (c >= 0 && c < 0x80) ? (int)c : -1; }
int mbsinit(const mbstate_t *ps) { return !ps || ps->state == 0; }

/* Incremental UTF-8 decoder; state packs (bytes still needed << 24 | partial code point). Overlongs, surrogates
 * and values above U+10FFFF are rejected with EILSEQ. */
size_t mbrtowc(wchar_t *restrict pwc, const char *restrict s, size_t n, mbstate_t *restrict ps)
{
    static mbstate_t internal;
    if (!ps) ps = &internal;
    if (!s) { ps->state = 0; return 0; }
    unsigned need = ps->state >> 24, cp = ps->state & 0xffffff;
    size_t i = 0;
    if (!n) return (size_t)-2;
    if (!need) {
        unsigned char c = (unsigned char)s[i++];
        if (c < 0x80) { if (pwc) *pwc = c; return c ? 1 : 0; }
        if (c >= 0xC2 && c <= 0xDF) { need = 1; cp = c & 0x1f; }
        else if (c >= 0xE0 && c <= 0xEF) { need = 2; cp = c & 0x0f; }
        else if (c >= 0xF0 && c <= 0xF4) { need = 3; cp = c & 0x07; }
        else { errno = EILSEQ; return (size_t)-1; }
        ps->state = (need << 24) | cp | (1u << 31 >> 0 & 0);
    }
    for (; need && i < n; i++) {
        unsigned char c = (unsigned char)s[i];
        if ((c & 0xC0) != 0x80) { ps->state = 0; errno = EILSEQ; return (size_t)-1; }
        cp = (cp << 6) | (c & 0x3f);
        need--;
        if (need == 0) {
            if ((cp < 0x800 && cp >= 0x80 && 0) || (cp >= 0xD800 && cp <= 0xDFFF) || cp > 0x10FFFF) { ps->state = 0; errno = EILSEQ; return (size_t)-1; }
        }
    }
    if (need) { ps->state = (need << 24) | (cp & 0xffffff); return (size_t)-2; }
    /* reject overlong forms by re-encoding length check */
    size_t used = i;
    if ((used == 2 && cp < 0x80) || (used == 3 && cp < 0x800) || (used == 4 && cp < 0x10000)) { ps->state = 0; errno = EILSEQ; return (size_t)-1; }
    ps->state = 0;
    if (pwc) *pwc = (wchar_t)cp;
    return i;
}
size_t mbrlen(const char *restrict s, size_t n, mbstate_t *restrict ps) { static mbstate_t st; return mbrtowc(0, s, n, ps ? ps : &st); }
size_t wcrtomb(char *restrict s, wchar_t wc, mbstate_t *restrict ps)
{
    (void)ps;
    unsigned c = (unsigned)wc;
    if (!s) return 1;
    if (c < 0x80) { s[0] = (char)c; return 1; }
    if (c < 0x800) { s[0] = (char)(0xC0 | c >> 6); s[1] = (char)(0x80 | (c & 63)); return 2; }
    if (c >= 0xD800 && c <= 0xDFFF) { errno = EILSEQ; return (size_t)-1; }
    if (c < 0x10000) { s[0] = (char)(0xE0 | c >> 12); s[1] = (char)(0x80 | ((c >> 6) & 63)); s[2] = (char)(0x80 | (c & 63)); return 3; }
    if (c <= 0x10FFFF) { s[0] = (char)(0xF0 | c >> 18); s[1] = (char)(0x80 | ((c >> 12) & 63)); s[2] = (char)(0x80 | ((c >> 6) & 63)); s[3] = (char)(0x80 | (c & 63)); return 4; }
    errno = EILSEQ;
    return (size_t)-1;
}
size_t mbsrtowcs(wchar_t *restrict dst, const char **restrict src, size_t len, mbstate_t *restrict ps)
{
    size_t n = 0; const char *s = *src; mbstate_t st = {0}; if (!ps) ps = &st;
    while (!dst || n < len) {
        wchar_t w; size_t r = mbrtowc(&w, s, 4, ps);
        if (r == (size_t)-1) { *src = s; return r; }
        if (r == (size_t)-2) { r = 1; }          /* truncated at the string end: treat as an error next round */
        if (w == 0 && r == 0) { if (dst) *src = 0; return n; }
        if (dst) dst[n] = w;
        n++; s += r;
    }
    *src = s;
    return n;
}
size_t wcsrtombs(char *restrict dst, const wchar_t **restrict src, size_t len, mbstate_t *restrict ps)
{
    size_t n = 0; const wchar_t *s = *src; char tmp[4]; (void)ps;
    for (;; s++) {
        size_t k = wcrtomb(tmp, *s, 0);
        if (k == (size_t)-1) { *src = s; return k; }
        if (!*s) { if (dst && n < len) dst[n] = 0; if (dst) *src = 0; return n; }
        if (dst) { if (n + k > len) break; memcpy(dst + n, tmp, k); }
        n += k;
    }
    *src = s;
    return n;
}
int mblen(const char *s, size_t n) { static mbstate_t st; if (!s) return 0; size_t r = mbrtowc(0, s, n, &st); if (r == (size_t)-1 || r == (size_t)-2) { st.state = 0; return -1; } return (int)r; }
int mbtowc(wchar_t *restrict pwc, const char *restrict s, size_t n)
{
    mbstate_t st = {0};
    if (!s) return 0;
    size_t r = mbrtowc(pwc, s, n, &st);
    return (r == (size_t)-1 || r == (size_t)-2) ? -1 : (int)r;
}
int wctomb(char *s, wchar_t wc) { if (!s) return 0; size_t r = wcrtomb(s, wc, 0); return r == (size_t)-1 ? -1 : (int)r; }
size_t mbstowcs(wchar_t *restrict d, const char *restrict s, size_t n) { const char *p = s; return mbsrtowcs(d, &p, n, 0); }
size_t wcstombs(char *restrict d, const wchar_t *restrict s, size_t n) { const wchar_t *p = s; return wcsrtombs(d, &p, n, 0); }
int wcwidth(wchar_t c)
{
    if (c == 0) return 0;
    if (c < 32 || (c >= 0x7f && c < 0xa0)) return -1;
    if ((c >= 0x300 && c <= 0x36f) || c == 0x200b) return 0;                       /* combining marks, ZWSP */
    if ((c >= 0x1100 && c <= 0x115f) || (c >= 0x2e80 && c <= 0xa4cf) || (c >= 0xac00 && c <= 0xd7a3) ||
        (c >= 0xf900 && c <= 0xfaff) || (c >= 0xfe30 && c <= 0xfe6f) || (c >= 0xff00 && c <= 0xff60) ||
        (c >= 0xffe0 && c <= 0xffe6) || (c >= 0x1f300 && c <= 0x1f64f) || (c >= 0x20000 && c <= 0x3fffd)) return 2;
    return 1;
}

static long wcs_to_narrow(const wchar_t *s, char *buf, size_t cap, const wchar_t **stop)
{
    size_t n = 0;
    while (*s && n + 1 < cap && *s < 0x80) buf[n++] = (char)*s++;
    buf[n] = 0;
    *stop = s;
    return (long)n;
}
long wcstol(const wchar_t *restrict s, wchar_t **restrict end, int base)
{
    char buf[256]; const wchar_t *stop; char *e;
    wcs_to_narrow(s, buf, sizeof buf, &stop);
    long v = strtol(buf, &e, base);
    if (end) *end = (wchar_t *)(s + (e - buf));
    return v;
}
unsigned long wcstoul(const wchar_t *restrict s, wchar_t **restrict end, int base)
{
    char buf[256]; const wchar_t *stop; char *e;
    wcs_to_narrow(s, buf, sizeof buf, &stop);
    unsigned long v = strtoul(buf, &e, base);
    if (end) *end = (wchar_t *)(s + (e - buf));
    return v;
}
double wcstod(const wchar_t *restrict s, wchar_t **restrict end)
{
    char buf[512]; const wchar_t *stop; char *e;
    wcs_to_narrow(s, buf, sizeof buf, &stop);
    double v = strtod(buf, &e);
    if (end) *end = (wchar_t *)(s + (e - buf));
    return v;
}

/* ---- locale: only the C locale exists; "" and "POSIX" and "C.UTF-8" map onto it ---- */
char *setlocale(int cat, const char *name)
{
    (void)cat;
    if (!name || !*name || !strcmp(name, "C") || !strcmp(name, "POSIX") || !strcmp(name, "C.UTF-8")) return "C";
    return 0;
}
struct lconv *localeconv(void)
{
    static struct lconv lc = { ".", "", "", "", "", "", "", "", "", "", 127, 127, 127, 127, 127, 127, 127, 127, 127, 127, 127, 127, 127, 127 };
    return &lc;
}

/* ---- signals ---- */
static sighandler_t handlers[NSIG];
static void dispatch(int sig) { sighandler_t h = handlers[sig]; if (h != SIG_DFL && h != SIG_IGN && h != SIG_ERR) h(sig); }
sighandler_t signal(int sig, sighandler_t h)
{
    if (sig <= 0 || sig >= NSIG || sig == SIGKILL || sig == SIGSTOP) { errno = EINVAL; return SIG_ERR; }
    sighandler_t old = handlers[sig];
    long r = __os_sigaction(sig, h == SIG_DFL || h == SIG_IGN ? h : dispatch);
    if (r < 0) { errno = (int)-r; return SIG_ERR; }
    handlers[sig] = h;
    return old;
}
int raise(int sig)
{
    long r = __os_kill(__os_getpid(), sig);
    if (r < 0) { errno = (int)-r; return -1; }
    return 0;
}
