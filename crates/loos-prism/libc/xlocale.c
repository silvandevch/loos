/* Locales for libc++ and the SDK's xlocale API. Prism has one character set (UTF-8) and the C
 * conventions for numbers, money and time, so every locale_t behaves like "C.UTF-8": newlocale
 * accepts C, POSIX, "" and any *.UTF-8 / en_* name, and each *_l function is its plain
 * counterpart. Wide-character classes follow _DefaultRuneLocale below 256 and a coarse Unicode
 * rule above it (letters, with the Unicode space characters as space). */
#include "prism.h"

typedef struct _xlocale { char name[32]; } *locale_t;
typedef int wint_t;
typedef unsigned long wctype_t;
typedef union { char b[128]; long long l; } mbstate_t;
struct lconv;
struct tm;

unsigned ___maskrune(int, unsigned long);
int tolower(int);
int toupper(int);
struct lconv *localeconv(void);
size_t strftime(char *, size_t, const char *, const struct tm *);
double strtod(const char *, char **);
float strtof(const char *, char **);
long long strtoll(const char *, char **, int);
unsigned long long strtoull(const char *, char **, int);
unsigned long strtoul(const char *, char **, int);
size_t mbrtowc(wchar_t *, const char *, size_t, mbstate_t *);
size_t wcrtomb(char *, wchar_t, mbstate_t *);
size_t mbrlen(const char *, size_t, mbstate_t *);
int mbtowc(wchar_t *, const char *, size_t);
size_t mbsrtowcs(wchar_t *, const char **, size_t, mbstate_t *);
wint_t btowc(int);
int wctob(wint_t);
int vsscanf(const char *, const char *, va_list);
int wcscoll(const wchar_t *, const wchar_t *);
size_t wcslen(const wchar_t *);
int ungetc(int, FILE *);
extern int __mb_cur_max;

static struct _xlocale c_locale = { "C" }, utf8_locale = { "en_US.UTF-8" };

static int utf8_name(const char *n) {
    size_t k = strlen(n);
    for (size_t i = 0; i + 5 <= k; i++)
        if ((n[i] == 'U' || n[i] == 'u') && (n[i + 1] == 'T' || n[i + 1] == 't') && (n[i + 2] == 'F' || n[i + 2] == 'f')) return 1;
    return n[0] == 'e' && n[1] == 'n' && (n[2] == '_' || n[2] == 0);
}
locale_t newlocale(int mask, const char *name, locale_t base) {
    (void)mask; (void)base;
    if (!name) { errno = EINVAL; return 0; }
    if (!name[0] || !strcmp(name, "C") || !strcmp(name, "POSIX")) return &c_locale;
    if (utf8_name(name)) return &utf8_locale;
    errno = ENOENT;
    return 0;
}
locale_t duplocale(locale_t l) { return l == (locale_t)-1 || !l ? &c_locale : l; }
void freelocale(locale_t l) { (void)l; }
static locale_t current = (locale_t)-1;
locale_t uselocale(locale_t l) { locale_t old = current; if (l) current = l; return old; }
const char *querylocale(int mask, locale_t l) { (void)mask; return l && l != (locale_t)-1 ? l->name : "C"; }
struct lconv *localeconv_l(locale_t l) { (void)l; return localeconv(); }
int ___mb_cur_max_l(locale_t l) { (void)l; return __mb_cur_max; }

/* ---------------------------------------------------------------- character classes */
#define C_A 0x100
#define C_C 0x200
#define C_D 0x400
#define C_G 0x800
#define C_L 0x1000
#define C_P 0x2000
#define C_S 0x4000
#define C_U 0x8000
#define C_X 0x10000
#define C_B 0x20000
#define C_R 0x40000

static int wupper(wint_t c) { return (c >= 0x391 && c <= 0x3a9) || (c >= 0x400 && c <= 0x42f); }
static int wlower(wint_t c) { return (c >= 0x3b1 && c <= 0x3c9) || (c >= 0x430 && c <= 0x45f); }
static unsigned long wclass(wint_t c) {
    if (c < 0) return 0;
    if (c < 256) return ___maskrune(c, 0xffffffffUL);
    if (c == 0x1680 || (c >= 0x2000 && c <= 0x200a) || c == 0x2028 || c == 0x2029 || c == 0x205f || c == 0x3000)
        return C_S | C_B * (c != 0x2028 && c != 0x2029);
    if ((c >= 0xd800 && c <= 0xdfff) || c > 0x10ffff || c == 0xfffe || c == 0xffff) return 0;
    if ((c >= 0x2010 && c <= 0x2027) || (c >= 0x2030 && c <= 0x205e) || (c >= 0x3001 && c <= 0x3003)) return C_P | C_G | C_R;
    return C_A | C_G | C_R | (wupper(c) ? C_U : 0) | (wlower(c) ? C_L : 0);
}
int isalnum(int c) { return (___maskrune(c, C_A | C_D)) != 0; }
int isspace(int c) { return ___maskrune(c, C_S) != 0; }
int isxdigit(int c) { return ___maskrune(c, C_X) != 0; }
int iswctype(wint_t c, wctype_t m) { return (wclass(c) & m) != 0; }
wctype_t wctype(const char *n) {
    static const char *names[] = { "alnum", "alpha", "blank", "cntrl", "digit", "graph", "lower", "print", "punct", "space", "upper", "xdigit" };
    static const unsigned long bits[] = { C_A | C_D, C_A, C_B, C_C, C_D, C_G, C_L, C_R, C_P, C_S, C_U, C_X };
    for (int i = 0; i < 12; i++) if (!strcmp(n, names[i])) return bits[i];
    return 0;
}
#define WIS(name, bits) int name(wint_t c) { return iswctype(c, bits); } \
    int name##_l(wint_t c, locale_t l) { (void)l; return iswctype(c, bits); }
WIS(iswalnum, C_A | C_D) WIS(iswalpha, C_A) WIS(iswblank, C_B) WIS(iswcntrl, C_C) WIS(iswdigit, C_D)
WIS(iswgraph, C_G) WIS(iswlower, C_L) WIS(iswprint, C_R) WIS(iswpunct, C_P) WIS(iswspace, C_S)
WIS(iswupper, C_U) WIS(iswxdigit, C_X)
int iswctype_l(wint_t c, wctype_t m, locale_t l) { (void)l; return iswctype(c, m); }
wctype_t wctype_l(const char *n, locale_t l) { (void)l; return wctype(n); }
wint_t towlower(wint_t c) {
    if (c < 256) return c < 0 ? c : tolower(c);
    if (wupper(c)) return c >= 0x400 && c < 0x410 ? c + 0x50 : c + 0x20;
    return c;
}
wint_t towupper(wint_t c) {
    if (c < 256) return c < 0 ? c : toupper(c);
    if (wlower(c)) return c >= 0x450 ? c - 0x50 : c - 0x20;
    return c;
}
wint_t towlower_l(wint_t c, locale_t l) { (void)l; return towlower(c); }
wint_t towupper_l(wint_t c, locale_t l) { (void)l; return towupper(c); }
int tolower_l(int c, locale_t l) { (void)l; return tolower(c); }
int toupper_l(int c, locale_t l) { (void)l; return toupper(c); }
#define IS(name, bits) int name##_l(int c, locale_t l) { (void)l; return ___maskrune(c, bits) != 0; }
IS(isalnum, C_A | C_D) IS(isalpha, C_A) IS(isblank, C_B) IS(iscntrl, C_C) IS(isdigit, C_D) IS(isgraph, C_G)
IS(islower, C_L) IS(isprint, C_R) IS(ispunct, C_P) IS(isspace, C_S) IS(isupper, C_U) IS(isxdigit, C_X)

/* ---------------------------------------------------------------- collation (code point order) */
int strcoll(const char *a, const char *b) { return strcmp(a, b); }
size_t strxfrm(char *d, const char *s, size_t n) {
    size_t k = strlen(s);
    if (k < n) memcpy(d, s, k + 1);
    return k;
}
size_t wcsxfrm(wchar_t *d, const wchar_t *s, size_t n) {
    size_t k = wcslen(s);
    if (k < n) memcpy(d, s, (k + 1) * sizeof(wchar_t));
    return k;
}
int strcoll_l(const char *a, const char *b, locale_t l) { (void)l; return strcmp(a, b); }
size_t strxfrm_l(char *d, const char *s, size_t n, locale_t l) { (void)l; return strxfrm(d, s, n); }
int wcscoll_l(const wchar_t *a, const wchar_t *b, locale_t l) { (void)l; return wcscoll(a, b); }
size_t wcsxfrm_l(wchar_t *d, const wchar_t *s, size_t n, locale_t l) { (void)l; return wcsxfrm(d, s, n); }

/* ---------------------------------------------------------------- multibyte (UTF-8) */
size_t mbsnrtowcs(wchar_t *d, const char **src, size_t nms, size_t len, mbstate_t *ps) {
    static mbstate_t own;
    const char *s = *src;
    size_t n = 0;
    if (!ps) ps = &own;
    while (!d || n < len) {
        wchar_t w;
        size_t k = mbrtowc(&w, s, nms, ps);
        if (k == (size_t)-1) { if (d) *src = s; return (size_t)-1; }
        if (k == (size_t)-2) { s += nms; nms = 0; break; }
        if (k == 0) { if (d) { d[n] = 0; *src = 0; } return n; }
        if (d) d[n] = w;
        s += k; nms -= k; n++;
    }
    if (d) *src = s;
    return n;
}
size_t wcsnrtombs(char *d, const wchar_t **src, size_t nwc, size_t len, mbstate_t *ps) {
    const wchar_t *s = *src;
    size_t n = 0;
    char buf[8];
    for (; nwc; nwc--, s++) {
        size_t k = wcrtomb(buf, *s, ps);
        if (k == (size_t)-1) { if (d) *src = s; return (size_t)-1; }
        if (d) {
            if (n + k > len) break;
            memcpy(d + n, buf, k);
        }
        if (!*s) { if (d) *src = 0; return n; }
        n += k;
    }
    if (d) *src = s;
    return n;
}
wint_t btowc_l(int c, locale_t l) { (void)l; return btowc(c); }
int wctob_l(wint_t c, locale_t l) { (void)l; return wctob(c); }
size_t mbrtowc_l(wchar_t *w, const char *s, size_t n, mbstate_t *p, locale_t l) { (void)l; return mbrtowc(w, s, n, p); }
size_t wcrtomb_l(char *s, wchar_t w, mbstate_t *p, locale_t l) { (void)l; return wcrtomb(s, w, p); }
size_t mbrlen_l(const char *s, size_t n, mbstate_t *p, locale_t l) { (void)l; return mbrlen(s, n, p); }
int mbtowc_l(wchar_t *w, const char *s, size_t n, locale_t l) { (void)l; return mbtowc(w, s, n); }
size_t mbsrtowcs_l(wchar_t *d, const char **s, size_t n, mbstate_t *p, locale_t l) { (void)l; return mbsrtowcs(d, s, n, p); }
size_t mbsnrtowcs_l(wchar_t *d, const char **s, size_t a, size_t n, mbstate_t *p, locale_t l) { (void)l; return mbsnrtowcs(d, s, a, n, p); }
size_t wcsnrtombs_l(char *d, const wchar_t **s, size_t a, size_t n, mbstate_t *p, locale_t l) { (void)l; return wcsnrtombs(d, s, a, n, p); }

/* ---------------------------------------------------------------- numbers, formatting, time */
long double strtold(const char *s, char **e) { return strtod(s, e); }
double strtod_l(const char *s, char **e, locale_t l) { (void)l; return strtod(s, e); }
float strtof_l(const char *s, char **e, locale_t l) { (void)l; return strtof(s, e); }
long double strtold_l(const char *s, char **e, locale_t l) { (void)l; return strtod(s, e); }
long strtol_l(const char *s, char **e, int b, locale_t l) { (void)l; return strtol(s, e, b); }
unsigned long strtoul_l(const char *s, char **e, int b, locale_t l) { (void)l; return strtoul(s, e, b); }
long long strtoll_l(const char *s, char **e, int b, locale_t l) { (void)l; return strtoll(s, e, b); }
unsigned long long strtoull_l(const char *s, char **e, int b, locale_t l) { (void)l; return strtoull(s, e, b); }
int vsnprintf_l(char *s, size_t n, locale_t l, const char *f, va_list a) { (void)l; return vsnprintf(s, n, f, a); }
int snprintf_l(char *s, size_t n, locale_t l, const char *f, ...) {
    va_list a; va_start(a, f); int r = vsnprintf(s, n, f, a); va_end(a); (void)l; return r;
}
int vasprintf_l(char **s, locale_t l, const char *f, va_list a) { (void)l; return vasprintf(s, f, a); }
int asprintf_l(char **s, locale_t l, const char *f, ...) {
    va_list a; va_start(a, f); int r = vasprintf(s, f, a); va_end(a); (void)l; return r;
}
int vsscanf_l(const char *s, locale_t l, const char *f, va_list a) { (void)l; return vsscanf(s, f, a); }
int sscanf_l(const char *s, locale_t l, const char *f, ...) {
    va_list a; va_start(a, f); int r = vsscanf(s, f, a); va_end(a); (void)l; return r;
}
size_t strftime_l(char *s, size_t n, const char *f, const struct tm *t, locale_t l) { (void)l; return strftime(s, n, f, t); }

/* nl_langinfo: Darwin's item numbers, C-locale strings (CODESET is UTF-8). */
char *nl_langinfo(int item) {
    static const char *const items[] = {
        "UTF-8", "%a %b %e %H:%M:%S %Y", "%m/%d/%y", "%H:%M:%S", "%I:%M:%S %p", "AM", "PM",
        "Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday",
        "Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat",
        "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December",
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        "", "", "", "", "", ".", "", "^[yY]", "^[nN]", "", "", "-",
    };
    return (char *)(item >= 0 && item < (int)(sizeof items / sizeof *items) ? items[item] : "");
}
char *nl_langinfo_l(int item, locale_t l) { (void)l; return nl_langinfo(item); }

/* ---------------------------------------------------------------- wide strings */
wchar_t *wmemchr(const wchar_t *s, wchar_t c, size_t n) { for (; n; n--, s++) if (*s == c) return (wchar_t *)s; return 0; }
int wmemcmp(const wchar_t *a, const wchar_t *b, size_t n) {
    for (; n; n--, a++, b++) if (*a != *b) return *a < *b ? -1 : 1;
    return 0;
}
/* wcsto*: the wide digits are ASCII, so narrow a prefix and let the byte parser do the work. */
static size_t narrow(const wchar_t *w, char *b, size_t cap) {
    size_t i = 0;
    for (; i + 1 < cap && w[i] > 0 && w[i] < 0x80; i++) b[i] = (char)w[i];
    b[i] = 0;
    return i;
}
#define WCSTO(name, type, call) type name(const wchar_t *w, wchar_t **e, int base) { \
        char b[256]; char *end; narrow(w, b, sizeof b); type r = call; \
        if (e) *e = (wchar_t *)w + (end - b); return r; }
WCSTO(wcstol, long, strtol(b, &end, base))
WCSTO(wcstoul, unsigned long, strtoul(b, &end, base))
WCSTO(wcstoll, long long, strtoll(b, &end, base))
WCSTO(wcstoull, unsigned long long, strtoull(b, &end, base))
#define WCSTOF(name, type, call) type name(const wchar_t *w, wchar_t **e) { \
        char b[256]; char *end; narrow(w, b, sizeof b); type r = call; \
        if (e) *e = (wchar_t *)w + (end - b); return r; }
WCSTOF(wcstod, double, strtod(b, &end))
WCSTOF(wcstof, float, strtof(b, &end))
WCSTOF(wcstold, long double, strtod(b, &end))

/* swprintf: format through the byte printf (the format's non-ASCII characters and %ls arguments
 * pass through as UTF-8), then widen. */
int vswprintf(wchar_t *d, size_t n, const wchar_t *wf, va_list a) {
    char f[512], out[1024];
    size_t i = 0;
    for (; wf[i] && i + 1 < sizeof f; i++) f[i] = wf[i] < 0x80 ? (char)wf[i] : '?';
    f[i] = 0;
    int k = vsnprintf(out, sizeof out, f, a);
    if (k < 0) return -1;
    const char *s = out;
    mbstate_t st = { { 0 } };
    size_t m = mbsrtowcs(d, &s, n, &st);
    if (m == (size_t)-1 || m >= n) { if (n) d[n - 1] = 0; errno = EOVERFLOW; return -1; }
    return (int)m;
}
int swprintf(wchar_t *d, size_t n, const wchar_t *f, ...) {
    va_list a; va_start(a, f); int r = vswprintf(d, n, f, a); va_end(a); return r;
}
wint_t ungetwc(wint_t w, FILE *fp) {
    char b[8];
    mbstate_t st = { { 0 } };
    if (w == -1) return -1;
    size_t k = wcrtomb(b, w, &st);
    if (k == (size_t)-1) return -1;
    while (k--) if (ungetc((unsigned char)b[k], fp) == EOF) return -1;
    return w;
}
