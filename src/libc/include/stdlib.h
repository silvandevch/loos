#ifndef _STDLIB_H
#define _STDLIB_H
#include "features.h"
#include <stddef.h>
__LOOS_BEGIN
#define EXIT_SUCCESS 0
#define EXIT_FAILURE 1
#define RAND_MAX 0x7fffffff
#define MB_CUR_MAX 4
typedef struct { int quot, rem; } div_t;
typedef struct { long quot, rem; } ldiv_t;
typedef struct { long long quot, rem; } lldiv_t;
void *malloc(size_t);
void *calloc(size_t, size_t);
void *realloc(void *, size_t);
void free(void *);
void *aligned_alloc(size_t, size_t);
int posix_memalign(void **, size_t, size_t);
size_t malloc_usable_size(void *);
__LOOS_NORETURN void abort(void);
__LOOS_NORETURN void exit(int);
__LOOS_NORETURN void _Exit(int);
__LOOS_NORETURN void quick_exit(int);
int atexit(void (*)(void));
int at_quick_exit(void (*)(void));
char *getenv(const char *);
int setenv(const char *, const char *, int);
int unsetenv(const char *);
int putenv(char *);
int system(const char *);
int abs(int); long labs(long); long long llabs(long long);
div_t div(int, int); ldiv_t ldiv(long, long); lldiv_t lldiv(long long, long long);
int atoi(const char *); long atol(const char *); long long atoll(const char *); double atof(const char *);
long strtol(const char *restrict, char **restrict, int);
unsigned long strtoul(const char *restrict, char **restrict, int);
long long strtoll(const char *restrict, char **restrict, int);
unsigned long long strtoull(const char *restrict, char **restrict, int);
double strtod(const char *restrict, char **restrict);
float strtof(const char *restrict, char **restrict);
long double strtold(const char *restrict, char **restrict);
int rand(void); void srand(unsigned);
long random(void); void srandom(unsigned);
void qsort(void *, size_t, size_t, int (*)(const void *, const void *));
void *bsearch(const void *, const void *, size_t, size_t, int (*)(const void *, const void *));
int mblen(const char *, size_t);
int mbtowc(wchar_t *restrict, const char *restrict, size_t);
int wctomb(char *, wchar_t);
size_t mbstowcs(wchar_t *restrict, const char *restrict, size_t);
size_t wcstombs(char *restrict, const wchar_t *restrict, size_t);
char *mktemp(char *);
int mkstemp(char *);
__LOOS_END
#endif
