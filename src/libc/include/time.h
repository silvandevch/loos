#ifndef _TIME_H
#define _TIME_H
#include "features.h"
#include <stddef.h>
__LOOS_BEGIN
typedef long time_t;
typedef long clock_t;
typedef int clockid_t;
#define CLOCKS_PER_SEC 1000000L
#define CLOCK_REALTIME 0
#define CLOCK_MONOTONIC 1
#define TIME_UTC 1
struct timespec { time_t tv_sec; long tv_nsec; };
struct tm {
    int tm_sec, tm_min, tm_hour, tm_mday, tm_mon, tm_year, tm_wday, tm_yday, tm_isdst;
    long tm_gmtoff; const char *tm_zone;
};
time_t time(time_t *);
clock_t clock(void);
double difftime(time_t, time_t);
time_t mktime(struct tm *);
time_t timegm(struct tm *);
struct tm *gmtime(const time_t *);
struct tm *gmtime_r(const time_t *restrict, struct tm *restrict);
struct tm *localtime(const time_t *);
struct tm *localtime_r(const time_t *restrict, struct tm *restrict);
char *asctime(const struct tm *);
char *ctime(const time_t *);
size_t strftime(char *restrict, size_t, const char *restrict, const struct tm *restrict);
int clock_gettime(clockid_t, struct timespec *);
int timespec_get(struct timespec *, int);
int nanosleep(const struct timespec *, struct timespec *);
unsigned sleep(unsigned);
__LOOS_END
#endif
