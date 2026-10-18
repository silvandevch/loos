/* The slice of the C runtime that ntdll.dll has always exported, for the many system tools and drivers-in-
 * userland that link against ntdll instead of a C library. Narrow-character mem/str routines come from
 * crt/string.c; the rest is here. Case-insensitive forms use the same simple Unicode folding as the Rtl layer. */
#include "../include/loos_nt.h"

#define CRT_EXPORT /* exported by name through ntdll/extra_exports.txt */

size_t wcslen(const WCHAR *s) { size_t n = 0; while (s[n]) n++; return n; }
int wcscmp(const WCHAR *a, const WCHAR *b) { while (*a && *a == *b) { a++; b++; } return (int)*a - (int)*b; }
WCHAR *wcscpy(WCHAR *d, const WCHAR *s) { WCHAR *r = d; while ((*d++ = *s++)) { } return r; }
WCHAR *wcsncpy(WCHAR *d, const WCHAR *s, size_t n) { size_t i = 0; for (; i < n && s[i]; i++) d[i] = s[i]; for (; i < n; i++) d[i] = 0; return d; }
WCHAR *wcschr(const WCHAR *s, WCHAR c) { for (;; s++) { if (*s == c) return (WCHAR *)s; if (!*s) return 0; } }
WCHAR *wcsrchr(const WCHAR *s, WCHAR c) { const WCHAR *last = 0; for (;; s++) { if (*s == c) last = s; if (!*s) return (WCHAR *)last; } }

int _wcsicmp(const WCHAR *a, const WCHAR *b)
{
    for (;; a++, b++) {
        WCHAR x = RtlUpcaseUnicodeChar(*a), y = RtlUpcaseUnicodeChar(*b);
        if (x != y) return (int)x - (int)y;
        if (!x) return 0;
    }
}
int _wcsnicmp(const WCHAR *a, const WCHAR *b, size_t n)
{
    for (; n; n--, a++, b++) {
        WCHAR x = RtlUpcaseUnicodeChar(*a), y = RtlUpcaseUnicodeChar(*b);
        if (x != y) return (int)x - (int)y;
        if (!x) return 0;
    }
    return 0;
}
int _stricmp(const char *a, const char *b)
{
    for (;; a++, b++) {
        int x = (unsigned char)*a, y = (unsigned char)*b;
        if (x >= 'a' && x <= 'z') x -= 32;
        if (y >= 'a' && y <= 'z') y -= 32;
        if (x != y) return x - y;
        if (!x) return 0;
    }
}
int _strnicmp(const char *a, const char *b, size_t n)
{
    for (; n; n--, a++, b++) {
        int x = (unsigned char)*a, y = (unsigned char)*b;
        if (x >= 'a' && x <= 'z') x -= 32;
        if (y >= 'a' && y <= 'z') y -= 32;
        if (x != y) return x - y;
        if (!x) return 0;
    }
    return 0;
}
int toupper(int c) { return (c >= 'a' && c <= 'z') ? c - 32 : c; }
int tolower(int c) { return (c >= 'A' && c <= 'Z') ? c + 32 : c; }
int towupper(unsigned c) { return c > 0xFFFF ? (int)c : (int)RtlUpcaseUnicodeChar((WCHAR)c); }
int towlower(unsigned c) { return c > 0xFFFF ? (int)c : (int)RtlDowncaseUnicodeChar((WCHAR)c); }
