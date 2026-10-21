/* LOOS_LIBC_PURE builds only the routines with no OS or allocator dependency (no strdup/strerror); the kernel
 * and the Win32 system DLLs compile this same file that way instead of keeping private copies. */
#include "../include/string.h"
#include <stdint.h>
#ifndef LOOS_LIBC_PURE
#include <stdlib.h>
#include <errno.h>
#include <ctype.h>
#else
static int tolower(int c) { return (c >= 'A' && c <= 'Z') ? c + 32 : c; }
#endif

void *memcpy(void *restrict d, const void *restrict s, size_t n)
{
    void *r = d;
    if (n < 16) { unsigned char *dp = d; const unsigned char *sp = s; while (n--) *dp++ = *sp++; return r; }
    __asm__ volatile("rep movsb" : "+D"(d), "+S"(s), "+c"(n) : : "memory");
    return r;
}
void *mempcpy(void *restrict d, const void *restrict s, size_t n) { return (char *)memcpy(d, s, n) + n; }
void *memmove(void *d, const void *s, size_t n)
{
    unsigned char *dp = d; const unsigned char *sp = s;
    if (dp == sp || !n) return d;
    if (dp < sp || dp >= sp + n) return memcpy(d, s, n);
    while (n--) dp[n] = sp[n];
    return d;
}
void *memset(void *d, int c, size_t n)
{
    void *r = d;
    if (n < 16) { unsigned char *p = d; while (n--) *p++ = (unsigned char)c; return r; }
    __asm__ volatile("rep stosb" : "+D"(d), "+c"(n) : "a"(c) : "memory");
    return r;
}
int memcmp(const void *a, const void *b, size_t n)
{
    const unsigned char *x = a, *y = b;
    while (n >= 8) {
        uint64_t u, v;
        __builtin_memcpy(&u, x, 8); __builtin_memcpy(&v, y, 8);
        if (u != v) break;
        x += 8; y += 8; n -= 8;
    }
    for (; n; n--, x++, y++) if (*x != *y) return *x - *y;
    return 0;
}
void *memchr(const void *s, int c, size_t n)
{
    const unsigned char *p = s;
    for (; n; n--, p++) if (*p == (unsigned char)c) return (void *)p;
    return 0;
}
void *memrchr(const void *s, int c, size_t n)
{
    const unsigned char *p = s;
    while (n--) if (p[n] == (unsigned char)c) return (void *)(p + n);
    return 0;
}
void *memmem(const void *h, size_t hl, const void *nd, size_t nl)
{
    const unsigned char *p = h;
    if (!nl) return (void *)h;
    if (hl < nl) return 0;
    for (size_t i = 0; i + nl <= hl; i++)
        if (p[i] == *(const unsigned char *)nd && !memcmp(p + i, nd, nl)) return (void *)(p + i);
    return 0;
}

size_t strlen(const char *s)
{
    const char *p = s;
    while ((uintptr_t)p & 7) { if (!*p) return (size_t)(p - s); p++; }
    const uint64_t *w = (const uint64_t *)p;
    for (;; w++) {                                           /* classic haszero(): any zero byte in the word */
        uint64_t v = *w;
        if ((v - 0x0101010101010101ULL) & ~v & 0x8080808080808080ULL) break;
    }
    p = (const char *)w;
    while (*p) p++;
    return (size_t)(p - s);
}
size_t strnlen(const char *s, size_t n) { size_t i = 0; while (i < n && s[i]) i++; return i; }
char *strcpy(char *restrict d, const char *restrict s) { char *r = d; while ((*d++ = *s++)) { } return r; }
char *stpcpy(char *restrict d, const char *restrict s) { while ((*d = *s++)) d++; return d; }
char *strncpy(char *restrict d, const char *restrict s, size_t n)
{
    size_t i = 0;
    for (; i < n && s[i]; i++) d[i] = s[i];
    for (; i < n; i++) d[i] = 0;
    return d;
}
char *stpncpy(char *restrict d, const char *restrict s, size_t n)
{
    size_t i = 0;
    for (; i < n && s[i]; i++) d[i] = s[i];
    char *e = d + i;
    for (; i < n; i++) d[i] = 0;
    return e;
}
size_t strlcpy(char *restrict d, const char *restrict s, size_t n)
{
    size_t l = strlen(s);
    if (n) { size_t c = l < n - 1 ? l : n - 1; memcpy(d, s, c); d[c] = 0; }
    return l;
}
size_t strlcat(char *restrict d, const char *restrict s, size_t n)
{
    size_t dl = strnlen(d, n);
    if (dl == n) return n + strlen(s);
    return dl + strlcpy(d + dl, s, n - dl);
}
char *strcat(char *restrict d, const char *restrict s) { strcpy(d + strlen(d), s); return d; }
char *strncat(char *restrict d, const char *restrict s, size_t n)
{
    char *e = d + strlen(d);
    size_t i = 0;
    for (; i < n && s[i]; i++) e[i] = s[i];
    e[i] = 0;
    return d;
}
int strcmp(const char *a, const char *b)
{
    while (*a && *a == *b) { a++; b++; }
    return (unsigned char)*a - (unsigned char)*b;
}
int strncmp(const char *a, const char *b, size_t n)
{
    for (; n; n--, a++, b++) { if (*a != *b || !*a) return (unsigned char)*a - (unsigned char)*b; }
    return 0;
}
int strcoll(const char *a, const char *b) { return strcmp(a, b); }
size_t strxfrm(char *restrict d, const char *restrict s, size_t n) { size_t l = strlen(s); if (n) strlcpy(d, s, n); return l; }
char *strchr(const char *s, int c) { for (;; s++) { if (*s == (char)c) return (char *)s; if (!*s) return 0; } }
char *strchrnul(const char *s, int c) { while (*s && *s != (char)c) s++; return (char *)s; }
char *strrchr(const char *s, int c) { const char *l = 0; for (;; s++) { if (*s == (char)c) l = s; if (!*s) return (char *)l; } }
size_t strspn(const char *s, const char *a) { size_t n = 0; while (s[n] && strchr(a, s[n])) n++; return n; }
size_t strcspn(const char *s, const char *r) { size_t n = 0; while (s[n] && !strchr(r, s[n])) n++; return n; }
char *strpbrk(const char *s, const char *a) { s += strcspn(s, a); return *s ? (char *)s : 0; }
char *strstr(const char *h, const char *n)
{
    size_t nl = strlen(n);
    if (!nl) return (char *)h;
    for (; *h; h++) if (*h == *n && !strncmp(h, n, nl)) return (char *)h;
    return 0;
}
int strcasecmp(const char *a, const char *b)
{
    for (;; a++, b++) {
        int x = tolower((unsigned char)*a), y = tolower((unsigned char)*b);
        if (x != y || !x) return x - y;
    }
}
int strncasecmp(const char *a, const char *b, size_t n)
{
    for (; n; n--, a++, b++) {
        int x = tolower((unsigned char)*a), y = tolower((unsigned char)*b);
        if (x != y || !x) return x - y;
    }
    return 0;
}
char *strcasestr(const char *h, const char *n)
{
    size_t nl = strlen(n);
    if (!nl) return (char *)h;
    for (; *h; h++) if (!strncasecmp(h, n, nl)) return (char *)h;
    return 0;
}
char *strtok_r(char *restrict s, const char *restrict delim, char **restrict save)
{
    if (!s) s = *save;
    if (!s) return 0;
    s += strspn(s, delim);
    if (!*s) { *save = 0; return 0; }
    char *e = s + strcspn(s, delim);
    if (*e) { *e = 0; *save = e + 1; } else *save = 0;
    return s;
}
char *strtok(char *restrict s, const char *restrict delim) { static char *save; return strtok_r(s, delim, &save); }
char *strsep(char **restrict sp, const char *restrict delim)
{
    char *s = *sp;
    if (!s) return 0;
    char *e = s + strcspn(s, delim);
    if (*e) { *e = 0; *sp = e + 1; } else *sp = 0;
    return s;
}
#ifndef LOOS_LIBC_PURE
char *strdup(const char *s) { size_t n = strlen(s) + 1; char *p = malloc(n); return p ? memcpy(p, s, n) : 0; }
char *strndup(const char *s, size_t n)
{
    n = strnlen(s, n);
    char *p = malloc(n + 1);
    if (!p) return 0;
    memcpy(p, s, n);
    p[n] = 0;
    return p;
}

static const char *const errtab[] = {
    [0] = "Success", [EPERM] = "Operation not permitted", [ENOENT] = "No such file or directory",
    [ESRCH] = "No such process", [EINTR] = "Interrupted system call", [EIO] = "Input/output error",
    [ENXIO] = "No such device or address", [E2BIG] = "Argument list too long", [ENOEXEC] = "Exec format error",
    [EBADF] = "Bad file descriptor", [ECHILD] = "No child processes", [EAGAIN] = "Resource temporarily unavailable",
    [ENOMEM] = "Cannot allocate memory", [EACCES] = "Permission denied", [EFAULT] = "Bad address",
    [EBUSY] = "Device or resource busy", [EEXIST] = "File exists", [EXDEV] = "Invalid cross-device link",
    [ENODEV] = "No such device", [ENOTDIR] = "Not a directory", [EISDIR] = "Is a directory",
    [EINVAL] = "Invalid argument", [ENFILE] = "Too many open files in system", [EMFILE] = "Too many open files",
    [ENOTTY] = "Inappropriate ioctl for device", [EFBIG] = "File too large", [ENOSPC] = "No space left on device",
    [ESPIPE] = "Illegal seek", [EROFS] = "Read-only file system", [EMLINK] = "Too many links",
    [EPIPE] = "Broken pipe", [EDOM] = "Numerical argument out of domain", [ERANGE] = "Numerical result out of range",
    [EDEADLK] = "Resource deadlock avoided", [ENAMETOOLONG] = "File name too long", [ENOSYS] = "Function not implemented",
    [ENOTEMPTY] = "Directory not empty", [ELOOP] = "Too many levels of symbolic links",
};
char *strerror(int e)
{
    static char unknown[32];
    if (e >= 0 && (size_t)e < sizeof errtab / sizeof *errtab && errtab[e]) return (char *)errtab[e];
    switch (e) {
    case EOVERFLOW: return "Value too large for defined data type";
    case EILSEQ: return "Invalid or incomplete multibyte or wide character";
    case ENOTSUP: return "Operation not supported";
    case ETIMEDOUT: return "Connection timed out";
    }
    strcpy(unknown, "Unknown error ");
    char tmp[12]; int n = 0; unsigned v = e < 0 ? (unsigned)-e : (unsigned)e;
    do tmp[n++] = (char)('0' + v % 10); while (v /= 10);
    char *o = unknown + 14;
    if (e < 0) *o++ = '-';
    while (n) *o++ = tmp[--n];
    *o = 0;
    return unknown;
}
int strerror_r(int e, char *buf, size_t n)
{
    const char *m = strerror(e);
    if (strlen(m) >= n) { if (n) strlcpy(buf, m, n); return ERANGE; }
    strcpy(buf, m);
    return 0;
}
#endif
