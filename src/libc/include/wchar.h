#ifndef _WCHAR_H
#define _WCHAR_H
#include "features.h"
#include <stddef.h>
#include <stdint.h>
__LOOS_BEGIN
typedef int wint_t;
typedef struct { unsigned state; } mbstate_t;
#define WEOF ((wint_t)-1)
size_t wcslen(const wchar_t *);
wchar_t *wcscpy(wchar_t *restrict, const wchar_t *restrict);
wchar_t *wcsncpy(wchar_t *restrict, const wchar_t *restrict, size_t);
wchar_t *wcscat(wchar_t *restrict, const wchar_t *restrict);
int wcscmp(const wchar_t *, const wchar_t *);
int wcsncmp(const wchar_t *, const wchar_t *, size_t);
wchar_t *wcschr(const wchar_t *, wchar_t);
wchar_t *wcsrchr(const wchar_t *, wchar_t);
wchar_t *wcsstr(const wchar_t *, const wchar_t *);
wchar_t *wmemcpy(wchar_t *restrict, const wchar_t *restrict, size_t);
wchar_t *wmemmove(wchar_t *, const wchar_t *, size_t);
wchar_t *wmemset(wchar_t *, wchar_t, size_t);
int wmemcmp(const wchar_t *, const wchar_t *, size_t);
wchar_t *wmemchr(const wchar_t *, wchar_t, size_t);
long wcstol(const wchar_t *restrict, wchar_t **restrict, int);
unsigned long wcstoul(const wchar_t *restrict, wchar_t **restrict, int);
double wcstod(const wchar_t *restrict, wchar_t **restrict);
wint_t btowc(int);
int wctob(wint_t);
int mbsinit(const mbstate_t *);
size_t mbrlen(const char *restrict, size_t, mbstate_t *restrict);
size_t mbrtowc(wchar_t *restrict, const char *restrict, size_t, mbstate_t *restrict);
size_t wcrtomb(char *restrict, wchar_t, mbstate_t *restrict);
size_t mbsrtowcs(wchar_t *restrict, const char **restrict, size_t, mbstate_t *restrict);
size_t wcsrtombs(char *restrict, const wchar_t **restrict, size_t, mbstate_t *restrict);
int wcwidth(wchar_t);
__LOOS_END
#endif
