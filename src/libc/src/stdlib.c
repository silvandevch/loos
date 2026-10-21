#include "libc.h"
#include <stdlib.h>
#include <string.h>
#include <ctype.h>
#include <errno.h>
#include <limits.h>
#include <inttypes.h>
#include <stdio.h>
#include <signal.h>
#include <unistd.h>
#include <sys/wait.h>

int abs(int x) { return x < 0 ? -x : x; }
long labs(long x) { return x < 0 ? -x : x; }
long long llabs(long long x) { return x < 0 ? -x : x; }
intmax_t imaxabs(intmax_t x) { return x < 0 ? -x : x; }
div_t div(int a, int b) { div_t r = { a / b, a % b }; return r; }
ldiv_t ldiv(long a, long b) { ldiv_t r = { a / b, a % b }; return r; }
lldiv_t lldiv(long long a, long long b) { lldiv_t r = { a / b, a % b }; return r; }
imaxdiv_t imaxdiv(intmax_t a, intmax_t b) { imaxdiv_t r = { a / b, a % b }; return r; }

/* ---- integer conversion ---- */
static unsigned long long conv_int(const char *s, char **end, int base, int *neg_out, int *ovf)
{
    const char *p = s;
    unsigned long long v = 0;
    int neg = 0, any = 0;
    *ovf = 0;
    while (isspace((unsigned char)*p)) p++;
    if (*p == '+' || *p == '-') neg = *p++ == '-';
    if ((base == 0 || base == 16) && p[0] == '0' && (p[1] == 'x' || p[1] == 'X') && isxdigit((unsigned char)p[2])) { p += 2; base = 16; }
    else if (base == 0) base = p[0] == '0' ? 8 : 10;
    if (base < 2 || base > 36) { errno = EINVAL; if (end) *end = (char *)s; *neg_out = 0; return 0; }
    for (;; p++) {
        int d, c = (unsigned char)*p;
        if (isdigit(c)) d = c - '0'; else if (isalpha(c)) d = tolower(c) - 'a' + 10; else break;
        if (d >= base) break;
        any = 1;
        if (v > (ULLONG_MAX - (unsigned)d) / (unsigned)base) *ovf = 1; else v = v * (unsigned)base + (unsigned)d;
    }
    if (end) *end = (char *)(any ? p : s);
    *neg_out = neg;
    return v;
}

unsigned long long strtoull(const char *restrict s, char **restrict end, int base)
{
    int neg, ovf;
    unsigned long long v = conv_int(s, end, base, &neg, &ovf);
    if (ovf) { errno = ERANGE; return ULLONG_MAX; }
    return neg ? 0 - v : v;
}
long long strtoll(const char *restrict s, char **restrict end, int base)
{
    int neg, ovf;
    unsigned long long v = conv_int(s, end, base, &neg, &ovf);
    if (ovf || (!neg && v > LLONG_MAX) || (neg && v > (unsigned long long)LLONG_MAX + 1)) {
        errno = ERANGE;
        return neg ? LLONG_MIN : LLONG_MAX;
    }
    return neg ? (long long)(0 - v) : (long long)v;
}
unsigned long strtoul(const char *restrict s, char **restrict e, int b) { return (unsigned long)strtoull(s, e, b); }
long strtol(const char *restrict s, char **restrict e, int b) { return (long)strtoll(s, e, b); }
intmax_t strtoimax(const char *restrict s, char **restrict e, int b) { return strtoll(s, e, b); }
uintmax_t strtoumax(const char *restrict s, char **restrict e, int b) { return strtoull(s, e, b); }
int atoi(const char *s) { return (int)strtol(s, 0, 10); }
long atol(const char *s) { return strtol(s, 0, 10); }
long long atoll(const char *s) { return strtoll(s, 0, 10); }
double atof(const char *s) { return strtod(s, 0); }

/* ---- pseudo-random numbers (64-bit LCG, high bits; period 2^64) ---- */
static uint64_t rng_state = 1;
void srand(unsigned seed) { rng_state = seed; }
int rand(void)
{
    rng_state = rng_state * 6364136223846793005ULL + 1442695040888963407ULL;
    return (int)(rng_state >> 33);
}
void srandom(unsigned seed) { srand(seed); }
long random(void) { return rand(); }

/* ---- sorting: introsort (median-of-3 quicksort, insertion sort for small ranges, heapsort fallback) ---- */
typedef int (*cmp_fn)(const void *, const void *);
static void swap_bytes(char *a, char *b, size_t n)
{
    while (n--) { char t = *a; *a++ = *b; *b++ = t; }
}
static void insertion(char *base, size_t n, size_t w, cmp_fn cmp)
{
    for (size_t i = 1; i < n; i++)
        for (size_t j = i; j > 0 && cmp(base + (j - 1) * w, base + j * w) > 0; j--)
            swap_bytes(base + (j - 1) * w, base + j * w, w);
}
static void sift(char *base, size_t start, size_t n, size_t w, cmp_fn cmp)
{
    for (size_t root = start;;) {
        size_t child = 2 * root + 1;
        if (child >= n) return;
        if (child + 1 < n && cmp(base + child * w, base + (child + 1) * w) < 0) child++;
        if (cmp(base + root * w, base + child * w) >= 0) return;
        swap_bytes(base + root * w, base + child * w, w);
        root = child;
    }
}
static void heapsort(char *base, size_t n, size_t w, cmp_fn cmp)
{
    for (size_t i = n / 2; i-- > 0;) sift(base, i, n, w, cmp);
    for (size_t end = n; end-- > 1;) { swap_bytes(base, base + end * w, w); sift(base, 0, end, w, cmp); }
}
static void introsort(char *base, size_t n, size_t w, cmp_fn cmp, int depth)
{
    while (n > 16) {
        if (depth-- == 0) { heapsort(base, n, w, cmp); return; }
        char *lo = base, *mid = base + (n / 2) * w, *hi = base + (n - 1) * w;
        if (cmp(lo, mid) > 0) swap_bytes(lo, mid, w);
        if (cmp(mid, hi) > 0) { swap_bytes(mid, hi, w); if (cmp(lo, mid) > 0) swap_bytes(lo, mid, w); }
        swap_bytes(mid, base + (n - 2) * w, w);              /* pivot parked at n-2 */
        char *pivot = base + (n - 2) * w;
        size_t i = 0, j = n - 2;
        for (;;) {
            while (cmp(base + ++i * w, pivot) < 0) { }
            while (cmp(base + --j * w, pivot) > 0) { if (j == 0) break; }
            if (i >= j) break;
            swap_bytes(base + i * w, base + j * w, w);
        }
        swap_bytes(base + i * w, pivot, w);
        size_t left = i, right = n - i - 1;
        if (left < right) { introsort(base, left, w, cmp, depth); base += (i + 1) * w; n = right; }
        else { introsort(base + (i + 1) * w, right, w, cmp, depth); n = left; }
    }
    insertion(base, n, w, cmp);
}
void qsort(void *base, size_t n, size_t w, cmp_fn cmp)
{
    int depth = 0;
    if (n < 2 || w == 0) return;
    for (size_t m = n; m; m >>= 1) depth += 2;
    introsort(base, n, w, cmp, depth);
}
void *bsearch(const void *key, const void *base, size_t n, size_t w, cmp_fn cmp)
{
    size_t lo = 0, hi = n;
    while (lo < hi) {
        size_t mid = lo + (hi - lo) / 2;
        const char *p = (const char *)base + mid * w;
        int c = cmp(key, p);
        if (c == 0) return (void *)p;
        if (c < 0) hi = mid; else lo = mid + 1;
    }
    return 0;
}

/* ---- environment ---- */
extern char **environ;
static int env_owned;                                        /* environ array was allocated by us */

static size_t env_count(void) { size_t n = 0; if (environ) while (environ[n]) n++; return n; }
static int env_index(const char *name, size_t len)
{
    if (!environ) return -1;
    for (int i = 0; environ[i]; i++)
        if (!strncmp(environ[i], name, len) && environ[i][len] == '=') return i;
    return -1;
}
char *getenv(const char *name)
{
    size_t len = strlen(name);
    if (!len || strchr(name, '=')) return 0;
    int i = env_index(name, len);
    return i < 0 ? 0 : environ[i] + len + 1;
}
static int env_grow(size_t extra)
{
    size_t n = env_count();
    char **ne = malloc((n + extra + 1) * sizeof *ne);
    if (!ne) return -1;
    for (size_t i = 0; i < n; i++) ne[i] = environ[i];
    ne[n] = 0;
    if (env_owned) free(environ);
    environ = ne;
    env_owned = 1;
    return 0;
}
int putenv(char *s)
{
    char *eq = strchr(s, '=');
    if (!eq || eq == s) { errno = EINVAL; return -1; }
    int i = env_index(s, (size_t)(eq - s));
    if (i >= 0) { environ[i] = s; return 0; }
    if (env_grow(1)) return -1;
    size_t n = env_count();
    environ[n] = s; environ[n + 1] = 0;
    return 0;
}
int setenv(const char *name, const char *value, int overwrite)
{
    size_t nl = strlen(name), vl = strlen(value);
    if (!nl || strchr(name, '=')) { errno = EINVAL; return -1; }
    int i = env_index(name, nl);
    if (i >= 0 && !overwrite) return 0;
    char *e = malloc(nl + vl + 2);
    if (!e) return -1;
    memcpy(e, name, nl); e[nl] = '='; memcpy(e + nl + 1, value, vl + 1);
    if (i >= 0) { environ[i] = e; return 0; }
    if (env_grow(1)) { free(e); return -1; }
    size_t n = env_count();
    environ[n] = e; environ[n + 1] = 0;
    return 0;
}
int unsetenv(const char *name)
{
    size_t nl = strlen(name);
    if (!nl || strchr(name, '=')) { errno = EINVAL; return -1; }
    int i;
    while ((i = env_index(name, nl)) >= 0)
        for (int j = i; (environ[j] = environ[j + 1]); j++) { }
    return 0;
}
int system(const char *cmd)
{
    if (!cmd) return access("/bin/sh", X_OK) == 0;
    pid_t pid = fork();
    if (pid < 0) return -1;
    if (pid == 0) {
        char *argv[] = { "sh", "-c", (char *)cmd, 0 };
        execve("/bin/sh", argv, environ);
        _exit(127);
    }
    int st;
    while (waitpid(pid, &st, 0) < 0) if (errno != EINTR) return -1;
    return st;
}

/* ---- process termination ---- */
#define ATEXIT_MAX 64
static void (*atexit_fns[ATEXIT_MAX])(void);
static int atexit_n;
static void (*quick_fns[ATEXIT_MAX])(void);
static int quick_n;

int atexit(void (*fn)(void)) { if (atexit_n == ATEXIT_MAX) return -1; atexit_fns[atexit_n++] = fn; return 0; }
int at_quick_exit(void (*fn)(void)) { if (quick_n == ATEXIT_MAX) return -1; quick_fns[quick_n++] = fn; return 0; }
void __run_atexit(void) { while (atexit_n) atexit_fns[--atexit_n](); }
void _Exit(int status) { __os_exit(status); }
void exit(int status) { __run_atexit(); __stdio_flush_all(); __os_exit(status); }
void quick_exit(int status) { while (quick_n) quick_fns[--quick_n](); __os_exit(status); }
void abort(void)
{
    raise(SIGABRT);
    __os_sigaction(SIGABRT, SIG_DFL);
    raise(SIGABRT);
    __os_exit(134);
}
void _exit(int status) { __os_exit(status); }

