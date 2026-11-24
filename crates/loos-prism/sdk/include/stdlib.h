#ifndef _STDLIB_H
#define _STDLIB_H
#include <_prism_cdefs.h>
typedef struct { int quot, rem; } div_t;
typedef struct { long quot, rem; } ldiv_t;
typedef struct { long long quot, rem; } lldiv_t;
#define EXIT_SUCCESS 0
#define EXIT_FAILURE 1
#define RAND_MAX 0x7fffffff
__BEGIN_DECLS
int ___mb_cur_max(void);
#define MB_CUR_MAX ((size_t)___mb_cur_max())
void abort(void) __attribute__((__noreturn__)); int abs(int); int atexit(void (*)(void));
int at_quick_exit(void (*)(void)); double atof(const char *); int atoi(const char *); long atol(const char *);
long long atoll(const char *); void *bsearch(const void *, const void *, size_t, size_t, int (*)(const void *, const void *));
void *calloc(size_t, size_t); div_t div(int, int); void exit(int) __attribute__((__noreturn__));
void _Exit(int) __attribute__((__noreturn__)); void quick_exit(int) __attribute__((__noreturn__));
void free(void *); char *getenv(const char *); long labs(long); ldiv_t ldiv(long, long);
long long llabs(long long); lldiv_t lldiv(long long, long long); void *malloc(size_t);
int mblen(const char *, size_t); size_t mbstowcs(wchar_t *, const char *, size_t);
int mbtowc(wchar_t *, const char *, size_t); void qsort(void *, size_t, size_t, int (*)(const void *, const void *));
int rand(void); void *realloc(void *, size_t); void srand(unsigned); double strtod(const char *, char **);
float strtof(const char *, char **); long double strtold(const char *, char **);
long strtol(const char *, char **, int); long long strtoll(const char *, char **, int);
unsigned long strtoul(const char *, char **, int); unsigned long long strtoull(const char *, char **, int);
int system(const char *); size_t wcstombs(char *, const wchar_t *, size_t); int wctomb(char *, wchar_t);
void *aligned_alloc(size_t, size_t); int posix_memalign(void **, size_t, size_t);
char *realpath(const char *, char *); int setenv(const char *, const char *, int); int unsetenv(const char *);
int mkstemp(char *); char *mkdtemp(char *); uint32_t arc4random(void); void arc4random_buf(void *, size_t);
uint32_t arc4random_uniform(uint32_t);
__END_DECLS
#include <xlocale.h>
#endif
