#ifndef _INTTYPES_H
#define _INTTYPES_H
#include <stdint.h>
#include "features.h"
#include <stddef.h>
__LOOS_BEGIN
typedef struct { intmax_t quot, rem; } imaxdiv_t;
intmax_t imaxabs(intmax_t);
imaxdiv_t imaxdiv(intmax_t, intmax_t);
intmax_t strtoimax(const char *restrict, char **restrict, int);
uintmax_t strtoumax(const char *restrict, char **restrict, int);
__LOOS_END
#define PRId8 "d"
#define PRId16 "d"
#define PRId32 "d"
#define PRId64 "ld"
#define PRIi32 "i"
#define PRIi64 "li"
#define PRIu8 "u"
#define PRIu16 "u"
#define PRIu32 "u"
#define PRIu64 "lu"
#define PRIx8 "x"
#define PRIx16 "x"
#define PRIx32 "x"
#define PRIx64 "lx"
#define PRIX32 "X"
#define PRIX64 "lX"
#define PRIo32 "o"
#define PRIo64 "lo"
#define PRIdMAX "jd"
#define PRIuMAX "ju"
#define PRIxMAX "jx"
#define PRIdPTR "ld"
#define PRIuPTR "lu"
#define PRIxPTR "lx"
#define SCNd32 "d"
#define SCNd64 "ld"
#define SCNu32 "u"
#define SCNu64 "lu"
#define SCNx32 "x"
#define SCNx64 "lx"
#endif
