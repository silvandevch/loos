#include "rt.h"
#include <stdint.h>

void *memcpy(void *d, const void *s, size_t n)
{
    void *r = d;
    __asm__ volatile("rep movsb" : "+D"(d), "+S"(s), "+c"(n) : : "memory");
    return r;
}

void *memmove(void *d, const void *s, size_t n)
{
    uint8_t *dd = d; const uint8_t *ss = s;
    if (dd == ss || n == 0) return d;
    if (dd < ss || dd >= ss + n) return memcpy(d, s, n);
    while (n--) dd[n] = ss[n];
    return d;
}

void *memset(void *d, int c, size_t n)
{
    void *r = d;
    __asm__ volatile("rep stosb" : "+D"(d), "+c"(n) : "a"(c) : "memory");
    return r;
}

int memcmp(const void *a, const void *b, size_t n)
{
    const uint8_t *x = a, *y = b;
    for (; n; n--, x++, y++) if (*x != *y) return *x < *y ? -1 : 1;
    return 0;
}

size_t strlen(const char *s) { size_t n = 0; while (s[n]) n++; return n; }
