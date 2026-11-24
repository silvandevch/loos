#ifndef _INTTYPES_H
#define _INTTYPES_H
#include <_prism_cdefs.h>
typedef struct { intmax_t quot, rem; } imaxdiv_t;
#define PRId64 "lld"
#define PRIi64 "lli"
#define PRIu64 "llu"
#define PRIx64 "llx"
#define PRIX64 "llX"
#define PRIo64 "llo"
#define PRId32 "d"
#define PRIu32 "u"
#define PRIx32 "x"
#define PRIdMAX "jd"
#define PRIuMAX "ju"
#define PRIdPTR "ld"
#define PRIuPTR "lu"
#define PRIxPTR "lx"
#define SCNd64 "lld"
#define SCNu64 "llu"
__BEGIN_DECLS
intmax_t imaxabs(intmax_t); imaxdiv_t imaxdiv(intmax_t, intmax_t);
intmax_t strtoimax(const char *, char **, int); uintmax_t strtoumax(const char *, char **, int);
intmax_t wcstoimax(const wchar_t *, wchar_t **, int); uintmax_t wcstoumax(const wchar_t *, wchar_t **, int);
__END_DECLS
#endif
