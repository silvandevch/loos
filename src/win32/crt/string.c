/* Minimal memory and string routines for the system DLLs. Plain loops on purpose: clear and
 * correct first; the compiler may recognise these idioms and turn them back
 * into calls to themselves, so they are built with -fno-builtin. */
#include "include/string.h"

void *memcpy(void *d, const void *s, size_t n)
{
    unsigned char *dp = d; const unsigned char *sp = s;
    while (n--) *dp++ = *sp++;
    return d;
}

void *memmove(void *d, const void *s, size_t n)
{
    unsigned char *dp = d; const unsigned char *sp = s;
    if (dp == sp || !n) return d;
    if (dp < sp) { while (n--) *dp++ = *sp++; }
    else { dp += n; sp += n; while (n--) *--dp = *--sp; }
    return d;
}

void *memset(void *d, int c, size_t n)
{
    unsigned char *dp = d;
    while (n--) *dp++ = (unsigned char)c;
    return d;
}

int memcmp(const void *a, const void *b, size_t n)
{
    const unsigned char *x = a, *y = b;
    for (; n; n--, x++, y++) if (*x != *y) return *x < *y ? -1 : 1;
    return 0;
}

size_t strlen(const char *s) { size_t n = 0; while (s[n]) n++; return n; }

int strcmp(const char *a, const char *b)
{
    while (*a && *a == *b) { a++; b++; }
    return (unsigned char)*a - (unsigned char)*b;
}

char *strcpy(char *d, const char *s) { char *r = d; while ((*d++ = *s++)) { } return r; }

char *strncpy(char *d, const char *s, size_t n)
{
    size_t i = 0;
    for (; i < n && s[i]; i++) d[i] = s[i];
    for (; i < n; i++) d[i] = 0;
    return d;
}

char *strchr(const char *s, int c)
{
    for (;; s++) { if (*s == (char)c) return (char *)s; if (!*s) return 0; }
}

char *strrchr(const char *s, int c)
{
    const char *last = 0;
    for (;; s++) { if (*s == (char)c) last = s; if (!*s) return (char *)last; }
}
