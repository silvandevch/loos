/*
 * The small, freestanding UCRT surface used by the first LoOS Win32 apps.
 *
 * This is deliberately implemented without a host C runtime: the PE image
 * imports only the heap and console primitives from kernel32.  The same
 * functions are therefore usable by a native LoOS kernel32 and by the host
 * compatibility harness.
 */
#define WINAPI __attribute__((ms_abi))

typedef unsigned long long size_t;
typedef long long ptrdiff_t;
typedef unsigned long DWORD;
typedef int BOOL;
typedef void *HANDLE;
typedef void *LPVOID;

#define NULL ((void *)0)
#define TRUE 1
#define FALSE 0
#define STD_OUTPUT_HANDLE ((DWORD)-11)
#define HEAP_ZERO_MEMORY 0x00000008UL

typedef __builtin_va_list va_list;
#define va_start(ap, last) __builtin_va_start(ap, last)
#define va_arg(ap, type) __builtin_va_arg(ap, type)
#define va_end(ap) __builtin_va_end(ap)

extern __declspec(dllimport) HANDLE WINAPI GetStdHandle(DWORD which);
extern __declspec(dllimport) BOOL WINAPI WriteFile(HANDLE, const void *, DWORD, DWORD *, void *);
extern __declspec(dllimport) HANDLE WINAPI GetProcessHeap(void);
extern __declspec(dllimport) LPVOID WINAPI HeapAlloc(HANDLE, DWORD, size_t);
extern __declspec(dllimport) LPVOID WINAPI HeapReAlloc(HANDLE, DWORD, LPVOID, size_t);
extern __declspec(dllimport) BOOL WINAPI HeapFree(HANDLE, DWORD, LPVOID);

static size_t text_len(const char *s)
{
    size_t n = 0;
    if (!s) return 0;
    while (s[n]) n++;
    return n;
}

int WINAPI _stricmp(const char *a, const char *b)
{
    unsigned char x, y;
    if (!a) a = "";
    if (!b) b = "";
    for (;;) {
        x = (unsigned char)*a++; y = (unsigned char)*b++;
        if (x >= 'a' && x <= 'z') x = (unsigned char)(x - 'a' + 'A');
        if (y >= 'a' && y <= 'z') y = (unsigned char)(y - 'a' + 'A');
        if (x != y || !x) return (int)x - (int)y;
    }
}

int WINAPI _strnicmp(const char *a, const char *b, size_t n)
{
    while (n--) {
        unsigned char x = (unsigned char)*a++, y = (unsigned char)*b++;
        if (x >= 'a' && x <= 'z') x = (unsigned char)(x - 'a' + 'A');
        if (y >= 'a' && y <= 'z') y = (unsigned char)(y - 'a' + 'A');
        if (x != y || !x) return (int)x - (int)y;
    }
    return 0;
}

size_t WINAPI strlen(const char *s) { return text_len(s); }
size_t WINAPI strnlen(const char *s, size_t n)
{
    size_t i = 0;
    if (!s) return 0;
    while (i < n && s[i]) i++;
    return i;
}

int WINAPI strcmp(const char *a, const char *b)
{
    unsigned char x, y;
    for (;;) {
        x = (unsigned char)*a++; y = (unsigned char)*b++;
        if (x != y || !x) return (int)x - (int)y;
    }
}

int WINAPI strncmp(const char *a, const char *b, size_t n)
{
    while (n--) {
        unsigned char x = (unsigned char)*a++, y = (unsigned char)*b++;
        if (x != y || !x) return (int)x - (int)y;
    }
    return 0;
}

void *WINAPI memcpy(void *dst, const void *src, size_t n)
{
    unsigned char *d = dst; const unsigned char *s = src;
    while (n--) *d++ = *s++;
    return dst;
}

void *WINAPI memmove(void *dst, const void *src, size_t n)
{
    unsigned char *d = dst; const unsigned char *s = src;
    if (d == s) return dst;
    if (d < s) while (n--) *d++ = *s++;
    else { d += n; s += n; while (n--) *--d = *--s; }
    return dst;
}

void *WINAPI memset(void *dst, int c, size_t n)
{
    unsigned char *d = dst;
    while (n--) *d++ = (unsigned char)c;
    return dst;
}

int WINAPI memcmp(const void *a, const void *b, size_t n)
{
    const unsigned char *x = a, *y = b;
    while (n--) if (*x != *y) return (int)*x - (int)*y; else { x++; y++; }
    return 0;
}

char *WINAPI strcpy(char *dst, const char *src) { return (char *)memcpy(dst, src, text_len(src) + 1); }
char *WINAPI strncpy(char *dst, const char *src, size_t n)
{
    size_t i = 0;
    while (i < n && src[i]) { dst[i] = src[i]; i++; }
    while (i < n) dst[i++] = 0;
    return dst;
}

char *WINAPI strcat(char *dst, const char *src)
{
    size_t n = text_len(dst);
    strcpy(dst + n, src);
    return dst;
}

char *WINAPI strchr(const char *s, int c)
{
    while (*s) { if ((unsigned char)*s == (unsigned char)c) return (char *)s; s++; }
    return c == 0 ? (char *)s : NULL;
}

char *WINAPI strrchr(const char *s, int c)
{
    const char *last = NULL;
    do { if ((unsigned char)*s == (unsigned char)c) last = s; } while (*s++);
    return (char *)last;
}

void *WINAPI malloc(size_t n)
{
    return HeapAlloc(GetProcessHeap(), 0, n ? n : 1);
}

void *WINAPI calloc(size_t count, size_t size)
{
    size_t n = count * size;
    if (count && n / count != size) return NULL;
    return HeapAlloc(GetProcessHeap(), HEAP_ZERO_MEMORY, n ? n : 1);
}

void WINAPI free(void *p);

void *WINAPI realloc(void *p, size_t n)
{
    if (!p) return malloc(n);
    if (!n) { free(p); return NULL; }
    return HeapReAlloc(GetProcessHeap(), 0, p, n);
}

void WINAPI free(void *p)
{
    if (p) (void)HeapFree(GetProcessHeap(), 0, p);
}

static void output_bytes(const char *p, size_t n)
{
    HANDLE out = GetStdHandle(STD_OUTPUT_HANDLE);
    while (n) {
        DWORD chunk = n > 0xffffffffULL ? 0xffffffffUL : (DWORD)n;
        DWORD written = 0;
        if (!WriteFile(out, p, chunk, &written, NULL) || !written) return;
        p += written;
        n -= written;
    }
}

static size_t output_unsigned(char *buf, size_t cap, unsigned long long v, unsigned base)
{
    static const char digits[] = "0123456789abcdef";
    char rev[32]; size_t n = 0, i;
    do { rev[n++] = digits[v % base]; v /= base; } while (v && n < sizeof rev);
    if (n > cap) n = cap;
    for (i = 0; i < n; i++) buf[i] = rev[n - i - 1];
    return n;
}

static size_t format(char *dst, size_t cap, const char *fmt, va_list ap)
{
    size_t used = 0;
    while (*fmt) {
        char c = *fmt++;
        if (c != '%') { if (used < cap) dst[used] = c; used++; continue; }
        c = *fmt++;
        if (!c) break;
        if (c == '%') { if (used < cap) dst[used] = '%'; used++; continue; }
        if (c == 's') {
            const char *s = va_arg(ap, const char *); size_t n = text_len(s);
            while (n--) { if (used < cap) dst[used] = *s; used++; s++; }
        } else if (c == 'c') {
            int ch = va_arg(ap, int); if (used < cap) dst[used] = (char)ch; used++;
        } else if (c == 'd' || c == 'i') {
            long long v = va_arg(ap, int); unsigned long long u;
            if (v < 0) { if (used < cap) dst[used] = '-'; used++; u = (unsigned long long)(-(v + 1)) + 1; }
            else u = (unsigned long long)v;
            used += output_unsigned(dst + (used < cap ? used : cap), used < cap ? cap - used : 0, u, 10);
        } else if (c == 'u' || c == 'x' || c == 'X') {
            unsigned long long u = va_arg(ap, unsigned int);
            size_t n = output_unsigned(dst + (used < cap ? used : cap), used < cap ? cap - used : 0,
                                       u, c == 'u' ? 10 : 16);
            if (c == 'X' && used < cap) {
                size_t i; for (i = 0; i < n && used + i < cap; i++)
                    if (dst[used + i] >= 'a' && dst[used + i] <= 'f') dst[used + i] -= 'a' - 'A';
            }
            used += n;
        } else {
            if (used < cap) dst[used] = '%'; used++;
            if (used < cap) dst[used] = c; used++;
        }
    }
    return used;
}

int WINAPI vprintf(const char *fmt, va_list ap)
{
    char buf[1024];
    va_list copy;
    size_t n;
    __builtin_va_copy(copy, ap);
    n = format(buf, sizeof buf, fmt, copy);
    va_end(copy);
    output_bytes(buf, n < sizeof buf ? n : sizeof buf);
    return n > 0x7fffffffULL ? -1 : (int)n;
}

int WINAPI printf(const char *fmt, ...)
{
    va_list ap;
    int r;
    va_start(ap, fmt);
    r = vprintf(fmt, ap);
    va_end(ap);
    return r;
}

int WINAPI puts(const char *s)
{
    size_t n = text_len(s);
    output_bytes(s ? s : "", n);
    output_bytes("\r\n", 2);
    return n > 0x7fffffffULL ? -1 : (int)n;
}

long WINAPI strtol(const char *s, char **end, int base)
{
    const char *p = s; int neg = 0; unsigned long long value = 0;
    if (!p) { if (end) *end = (char *)s; return 0; }
    while (*p == ' ' || *p == '\t' || *p == '\r' || *p == '\n') p++;
    if (*p == '+' || *p == '-') { neg = *p++ == '-'; }
    if (base == 0) { base = 10; if (*p == '0') { base = 8; p++; if (*p == 'x' || *p == 'X') { base = 16; p++; } } }
    else if (base == 16 && p[0] == '0' && (p[1] == 'x' || p[1] == 'X')) p += 2;
    while (*p) {
        unsigned d = (*p >= '0' && *p <= '9') ? (unsigned)(*p - '0') :
                     (*p >= 'a' && *p <= 'z') ? (unsigned)(*p - 'a' + 10) :
                     (*p >= 'A' && *p <= 'Z') ? (unsigned)(*p - 'A' + 10) : base;
        if (d >= (unsigned)base) break;
        value = value * (unsigned)base + d; p++;
    }
    if (end) *end = (char *)p;
    return neg ? -(long)value : (long)value;
}

unsigned long WINAPI strtoul(const char *s, char **end, int base)
{
    return (unsigned long)strtol(s, end, base);
}
